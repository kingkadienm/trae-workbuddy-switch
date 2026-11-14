//! 给上游客户端装 / 卸本工具的 hook（P15 · S3）。
//!
//! ## 它做什么
//!
//! 往客户端 `settings.json` 的 `hooks` 字段里，为每个关心的事件追加一条
//! 「调用本工具、把事件转给我们」的命令。客户端（Claude Code 兼容的 command hooks）
//! 会在每次会话 / 工具调用时同步执行它。
//!
//! ## ★ 三条纪律（`P15-SPIKE-RESULTS §6 D41`）
//!
//! 1. **不新造写入路径**：一律走 [`super::client_settings::patch_settings`] ——
//!    它自带**格式指纹保持**（不重排用户的 JSON 排版）、**写前自动备份**、
//!    **进程守卫**（客户端在跑就拒写，否则会被它的内存态覆盖）。
//!    本模块只负责算出「合并后的 `hooks` 应该长什么样」，落盘交给它。
//! 2. **合并不是覆盖**：用户可能已经装了别人的 hook。合并规则是
//!    「剔除**自己的**旧条目（幂等）→ 保留**别人的** → 追加自己的一条」，
//!    见 [`merge_hooks`]（纯函数，逐形态单测）。
//! 3. **可一键还原**：卸载 = 再合并一次、只是不追加（[`strip_our_hooks`]），
//!    且每一步都有 `client_settings` 的自动备份兜底。
//!
//! ## 为什么事件集里**没有** `PreCompact`
//!
//! [`super::agent_watch::HookKind`] 把 `PreCompact` 映射成 `Unknown` 并**整条丢弃**。
//! 注册一个我们明确不处理的事件，只会让客户端在每次上下文压缩时白起一个进程 ——
//! 所以**只注册状态机用得上的那 9 个**。

use std::path::Path;

use serde_json::{json, Map, Value};

use super::client_settings::{self, patch_settings, read_settings};

/// 要注册到客户端的事件（**Claude Code 系**：Claude Code / WorkBuddy / CodeBuddy CLI）。
///
/// 与客户端支持的事件集对齐，**只取状态机用得上的**（见模块头对 `PreCompact` 的说明）。
pub const HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PermissionRequest",
    "Notification",
    "Stop",
    "SessionEnd",
];

/// **Trae** 要注册的事件。
///
/// ## 取证（Trae 自带的 hook 执行器里的字符串枚举）
///
/// `D:\Programs\Trae CN\resources\app\resources\harness\harness.dll` 里可直接读到事件名表：
/// `SessionStart / UserPromptSubmit / PreToolUse / PostToolUse / Stop / Notification /
/// PreCompact / PostCompact / SessionEnd`。
///
/// ⚠️ 与 [`HOOK_EVENTS`] 的差异 —— **Trae 没有 `PostToolUseFailure` 与 `PermissionRequest`**
/// （`PermissionRequest` 在 Trae 里是一个**工具输入类型**，不是 hook 事件）。
/// ⇒ 不注册它不认的键：注册了也不会被派发，只会让用户的配置里多一条死条目。
///
/// ★ `TRAE_HOOK_EVENTS ⊆ HOOK_EVENTS`（刻意保持这个包含关系）：卸载时只要遍历
/// [`HOOK_EVENTS`] 就能把任一一档装进去的条目清干净，不需要「并集」再维护一份。
pub const TRAE_HOOK_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "Stop",
    "SessionEnd",
];

/// handler 的超时（秒）。
///
/// 与外部参考实现取同一个值：本命令只做「读 stdin + 转发一条本地 HTTP」，
/// 正常在毫秒级完成；3 s 是留给杀软 / 磁盘抖动这类一次性卡顿的余量。
pub const HOOK_TIMEOUT_SECONDS: u64 = 3;

/// 客户端侧显示的来源名（出现在客户端的 hook 列表里）。
pub const HOOK_STATUS_MESSAGE: &str = "Buddy Switch";

/// 一个客户端对 hook 配置的**格式档位**。
///
/// ## 为什么需要它（而不是一套格式打天下）
///
/// 两个客户端把「同一件事」的字段集**不一致**，而写错字段不是「被忽略」那么轻：
/// 解析端如果用了 `deny_unknown_fields`，多一个字段会让**整份配置**解析失败 ——
/// 用户原有的 hook 会一起失效。
///
/// 故按客户端分档，且**只写有取证支持的字段**：
///
/// | 档 | 事件集 | `async` | 依据 |
/// |:---|:---|:---|:---|
/// | [`HOOK_PROFILE_CLAUDE`] | [`HOOK_EVENTS`]（9） | 有（`SessionEnd` 除外） | 本机实测（Claude Code / CodeBuddy 的 `hooks` 键） |
/// | [`HOOK_PROFILE_TRAE`] | [`TRAE_HOOK_EVENTS`]（7） | **无** | Trae 的 handler 结构字段里**没有** `async` |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HookProfile {
    /// 要注册的事件（顺序稳定）。
    pub events: &'static [&'static str],
    /// handler 是否带 `async: true`（**客户端不认这个字段时必须为 `false`**）。
    pub async_handlers: bool,
}

/// Claude Code 系：见 [`HookProfile`] 的表。
pub const HOOK_PROFILE_CLAUDE: HookProfile = HookProfile {
    events: HOOK_EVENTS,
    async_handlers: true,
};

/// Trae：见 [`HookProfile`] 的表。
///
/// ⚠️ `async_handlers = false` —— Trae 的 handler 结构是
/// `{ type, command, url, if, timeout, headers, allowedEnvVars, statusMessage }`，
/// **没有 `async`**。写进去是一个它不认识的字段。
pub const HOOK_PROFILE_TRAE: HookProfile = HookProfile {
    events: TRAE_HOOK_EVENTS,
    async_handlers: false,
};

/// **卸载**用的档位：遍历 [`HOOK_EVENTS`]（⊇ Trae 档）。
///
/// 卸下来的条目可能来自任一档，而 Trae 的事件集是 Claude 档的子集 ⇒
/// 遍历 Claude 档即可清干净，不必再维护一份「并集」。
const HOOK_PROFILE_STRIP: HookProfile = HookProfile {
    events: HOOK_EVENTS,
    async_handlers: true,
};

/// 槽位 → 格式档位（**唯一取值点**）。
///
/// 判据落在**槽位的实现维度**上（`client_settings` 的 `SlotKind`），而不是在
/// 这里再抄一份槽位 key 列表 —— 抄一份就会在两处漂移。
pub fn profile_for(slot_key: &str) -> HookProfile {
    if client_settings::is_trae_hooks_slot(slot_key) {
        HOOK_PROFILE_TRAE
    } else {
        HOOK_PROFILE_CLAUDE
    }
}

/// 构造要写进客户端配置的那条命令行。
///
/// ⚠️ 路径**必须加引号**：Windows 上安装目录常含空格（`C:\Program Files\...`），
/// 不加引号会被客户端按空格切成两个参数。
pub fn hook_command(exe: &Path, source: &str) -> String {
    format!("\"{}\" hook --source {source}", exe.display())
}

/// 这条命令是不是**我们自己**装的。
///
/// 判据锚在「可执行文件是我们的」+「这是 hook 子命令」两件事上，
/// **不锚**在路径上（用户可以把应用装到任何地方、也可以改名后再改名回来）。
pub fn is_our_hook(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    (lower.contains("buddyswitch") || lower.contains("buddy-switch"))
        && lower.contains(" hook")
        && lower.contains("--source")
}

/// 一个 handler 对象（写进 `hooks.<Event>[].hooks[]`）。
fn handler(command: &str, event: &str, profile: HookProfile) -> Value {
    let mut handler = json!({
        "type": "command",
        "command": command,
        "timeout": HOOK_TIMEOUT_SECONDS,
        "statusMessage": HOOK_STATUS_MESSAGE,
    });
    // `SessionEnd` 是同步的：客户端在退出路径上等它跑完，此时起后台进程没意义。
    // 其余事件加 `async`，避免我们的转发拖慢客户端的工具调用（与外部参考实现同款）。
    //
    // ⚠️ 仅当**该档位认这个字段**时才写：Trae 的 handler 结构里没有 `async`，
    //    对一个可能 `deny_unknown_fields` 的解析端，多写一个字段会让**整份配置**读不出来。
    if profile.async_handlers && event != "SessionEnd" {
        handler["async"] = json!(true);
    }
    handler
}

/// 合并的**唯一**实现（纯函数）。
///
/// `command = Some(c)` ⇒ 「装」：剔除自己的旧条目后追加 `c`；
/// `command = None` ⇒ 「卸」：只剔除，不追加。
///
/// `profile` 决定「往哪些事件里插 / 插出来的 handler 长什么样」（见 [`HookProfile`]）。
///
/// ## 逐形态处理（用户配置里真实存在的两种写法）
///
/// - **分组形态** `{ matcher, hooks: [...] }` —— 主流写法，且**可能一个分组里混装**
///   我们与别人的 hook ⇒ 必须逐条过滤、且过滤后**不能留下空分组**
///   （空分组会被客户端当成「匹配任何 matcher 但没有 handler」）。
/// - **裸 handler 形态** `{ type: "command", command, ... }` —— 客户端早期写法 / 手写产物。
///
/// 除这两种以外的值（字符串、数字、null）**原样保留**：我们不认识的形态宁可不动，
/// 也不能替用户删掉。
///
/// ## 顶层**其他键一个都不动**
///
/// 实现是「克隆整份 doc → 只替换 `hooks`」⇒ `version`（Trae 有）、`env`、`permissions`、
/// `sandbox` 等键的**存在性与内容**都不受影响。这条对 Trae 尤其重要：
/// 它的 hooks 文件顶层是 `{version, hooks}`，我们**不复刻 `version`**（不知道取值域），
/// 靠的就是「只动 `hooks`」这一点。
pub fn rewrite_hooks(existing: &Value, command: Option<&str>, profile: HookProfile) -> Value {
    let mut doc = match existing {
        Value::Object(map) => Value::Object(map.clone()),
        _ => json!({}),
    };
    let mut hooks: Map<String, Value> = doc
        .get("hooks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    for event in profile.events {
        let groups = hooks
            .get(*event)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut kept: Vec<Value> = Vec::new();
        for group in groups {
            match group {
                Value::Object(ref map) if map.get("hooks").is_some_and(Value::is_array) => {
                    let inner = map["hooks"].as_array().cloned().unwrap_or_default();
                    let survivors: Vec<Value> = inner
                        .into_iter()
                        .filter(|item| !is_our_hook(command_of(item)))
                        .collect();
                    // 过滤后为空 ⇒ 整个分组丢掉（见文档：空分组是有害的残留）
                    if !survivors.is_empty() {
                        let mut next = map.clone();
                        next.insert("hooks".to_string(), Value::Array(survivors));
                        kept.push(Value::Object(next));
                    }
                }
                Value::Object(ref map) => {
                    if !is_our_hook(
                        map.get("command")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                    ) {
                        kept.push(group.clone());
                    }
                }
                other => kept.push(other.clone()),
            }
        }

        if let Some(command) = command {
            kept.push(json!({ "matcher": "", "hooks": [handler(command, event, profile)] }));
        }

        if kept.is_empty() {
            // 卸载后该事件上一条 hook 都不剩 ⇒ 删掉这个键，而不是留一个 `[]`
            // （空数组在客户端配置里没有意义，还会让用户以为「装过东西」）。
            hooks.remove(*event);
        } else {
            hooks.insert((*event).to_string(), Value::Array(kept));
        }
    }

    if hooks.is_empty() {
        // 一条都不剩 ⇒ 把 `hooks` 键整个删掉（不留空对象）
        if let Some(map) = doc.as_object_mut() {
            map.remove("hooks");
        }
    } else {
        doc["hooks"] = Value::Object(hooks);
    }
    doc
}

/// 「装」：见 [`rewrite_hooks`]。
pub fn merge_hooks(existing: &Value, command: &str, profile: HookProfile) -> Value {
    rewrite_hooks(existing, Some(command), profile)
}

/// 「卸」：见 [`rewrite_hooks`]。
///
/// 用 [`HOOK_PROFILE_STRIP`]（遍历全部已知事件）⇒ 无论当初按哪一档装进去的，
/// 都能清干净（Trae 档的事件集是 Claude 档的子集）。
pub fn strip_our_hooks(existing: &Value) -> Value {
    rewrite_hooks(existing, None, HOOK_PROFILE_STRIP)
}

/// handler 里的命令串（两种形态通吃）。
fn command_of(item: &Value) -> &str {
    item.get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// 某个事件的 handler 里，有没有**我们**的条目。
fn event_has_ours(stored: &Value, event: &str) -> bool {
    let Some(groups) = stored
        .get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
    else {
        return false;
    };
    groups.iter().any(|group| match group {
        Value::Object(map) if map.get("hooks").is_some_and(Value::is_array) => map["hooks"]
            .as_array()
            .is_some_and(|inner| inner.iter().any(|item| is_our_hook(command_of(item)))),
        other => is_our_hook(command_of(other)),
    })
}

/// 读槽位当前的配置（`values`）；读不到 ⇒ 明确的错误（见 [`missing_config_message`]）。
fn load_values(slot_key: &str) -> Result<Value, String> {
    let current = read_settings(slot_key)?;
    current
        .get("values")
        .filter(|values| !values.is_null())
        .cloned()
        .ok_or_else(|| missing_config_message(slot_key))
}

/// 配置文件不存在时的错误文案。
///
/// ★ **Trae 单独一档**，因为「文件不存在」在那里含义完全不同：Trae 的 `hooks.json` 是
/// **客户端自己生成**的（顶层 `{version, hooks}`，本工具**不复刻 `version`** ——
/// 不知道它的取值域，编一个可能让客户端整份配置读不出来）⇒ 文件不在**不代表客户端没装**，
/// 而代表「用户还没在 Trae 里启用过 Hooks」。此时让人「重新安装 Trae」是**指错方向**，
/// 必须说清该去哪儿、点什么。
fn missing_config_message(slot_key: &str) -> String {
    if client_settings::is_trae_hooks_slot(slot_key) {
        return format!(
            "找不到 Trae 的 hooks.json（槽位「{slot_key}」）。\
             Trae 的 hooks 文件由**客户端自己生成**，本工具不代它创建\
             （它顶层带一个 `version` 字段，取值域未取证 —— 编一个可能让 Trae 读不出整份配置）。\
             请先在 Trae 里打开「设置 → Hooks」启用一次\
             （想直接复用 Claude 的配置，可同时勾选「导入 Claude hooks」），\
             生成 hooks.json 后再回到这里开启监听。"
        );
    }
    format!("槽位「{slot_key}」没有可读的配置文件，无法安装 hook（该客户端未安装？）")
}

/// 从**已读出的**配置值判断哪些事件装了我们的 hook（供调用方避免二次读盘）。
pub fn installed_events_of(values: &Value, profile: HookProfile) -> Vec<String> {
    profile
        .events
        .iter()
        .filter(|event| event_has_ours(values, event))
        .map(|event| (*event).to_string())
        .collect()
}

/// 该槽位上，哪些事件已经装了我们的 hook（按该槽位档位的事件顺序）。
///
/// 诊断用：设置页据此告诉用户「装了几条」，而不是给一个含糊的「已开启」。
pub fn installed_events(slot_key: &str) -> Result<Vec<String>, String> {
    let profile = profile_for(slot_key);
    let current = read_settings(slot_key)?;
    let Some(values) = current.get("values").filter(|values| !values.is_null()) else {
        return Ok(Vec::new());
    };
    Ok(installed_events_of(values, profile))
}

/// 装：合并后写回。返回 [`patch_settings`] 的结果（含备份 id / 路径）。
///
/// ## 两层 fail-closed
///
/// 1. 配置文件**不存在** ⇒ 不创建，报 [`missing_config_message`]（不猜格式）；
/// 2. 客户端**正在运行** ⇒ `patch_settings` 的进程守卫拒写 —— 这是**刻意的**：
///    客户端退出时会把内存态写回，我们的改动会被静默覆盖。
pub fn install(slot_key: &str, command: &str) -> Result<Value, String> {
    let profile = profile_for(slot_key);
    let values = load_values(slot_key)?;
    let merged = merge_hooks(&values, command, profile);
    let hooks = merged.get("hooks").cloned().unwrap_or_else(|| json!({}));
    patch_settings(slot_key, &json!({ "hooks": hooks }))
}

/// 卸：剔除我们的条目后写回。
pub fn uninstall(slot_key: &str) -> Result<Value, String> {
    let values = load_values(slot_key)?;
    let stripped = strip_our_hooks(&values);
    let patch = match stripped.get("hooks") {
        Some(hooks) => json!({ "hooks": hooks }),
        // 一条都不剩 ⇒ 显式置空对象（而不是留 undefined）
        None => json!({ "hooks": {} }),
    };
    patch_settings(slot_key, &patch)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD: &str =
        "\"C:\\Program Files\\Buddy Switch\\BuddySwitch.exe\" hook --source workbuddy";

    fn others_hook() -> Value {
        json!({
            "matcher": "",
            "hooks": [{ "type": "command", "command": "/usr/bin/python3 /home/u/other.py" }]
        })
    }

    /// ★ **真实形态回归**：本机 `~/.claude/settings.json` 里已经有一条**别人的** hook
    /// （`codegraph.cmd prompt-hook` 挂在 `UserPromptSubmit` 上）。
    ///
    /// 我们装 / 卸一轮后，那条 hook 必须**逐字**还在（它不属于我们，我们无权动它）。
    ///
    /// 可证伪性：把 `rewrite_hooks` 的「保留别人的」改成「清空该事件数组」⇒ 变红。
    /// 这条用例的价值在于**语料取自真实文件**——手写的 `others_hook()` 只覆盖了
    /// 「分组形态 + 单条 handler」，真实语料还带 `matcher` 与自己的字段排布。
    #[test]
    fn real_world_claude_hook_survives_install_and_uninstall() {
        let original = json!({
            "env": { "SOME_FLAG": "1" },
            "permissions": { "allow": ["Bash(ls:*)"] },
            "model": "some-model",
            "hooks": {
                "UserPromptSubmit": [
                    { "hooks": [{ "type": "command", "command": "codegraph.cmd prompt-hook" }] }
                ]
            },
            "theme": "dark"
        });

        let installed = merge_hooks(&original, CMD, HOOK_PROFILE_CLAUDE);
        // ① 别人的那条仍在，且 `UserPromptSubmit` 上**多**了我们的一条。
        let groups = installed["hooks"]["UserPromptSubmit"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "别人的 + 我们的 = 2 条");
        assert_eq!(
            groups[0]["hooks"][0]["command"],
            json!("codegraph.cmd prompt-hook"),
            "第三方 hook 必须原样保留"
        );
        assert_eq!(groups[1]["hooks"][0]["command"], json!(CMD));
        // ② 我们注册的事件都被写进去了。
        for event in HOOK_EVENTS {
            assert!(
                installed["hooks"][event].is_array(),
                "事件 {event} 上应有我们的 hook"
            );
        }
        // ③ 与 hooks 无关的顶层键**一个都不能动**。
        assert_eq!(installed["env"]["SOME_FLAG"], json!("1"));
        assert_eq!(installed["permissions"]["allow"][0], json!("Bash(ls:*)"));
        assert_eq!(installed["model"], json!("some-model"));
        assert_eq!(installed["theme"], json!("dark"));

        // ④ 卸载后**逐字**回到原状（含键顺序无关的等值）。
        assert_eq!(
            strip_our_hooks(&installed),
            original,
            "装/卸一轮必须回到原状"
        );
    }

    /// ★ **真实形态回归**：本机 `~/.codebuddy/settings.json` 的 `hooks` 里挂着一条
    /// **别的工具**装的 shell hook（`bash '…/.wb-switch/hook.sh'`）。
    ///
    /// ⚠️ 它的命令串里**没有** `--source`、也不是我们的可执行文件 ⇒ 必须被当作「别人的」保留。
    /// 可证伪性：把 `is_our_hook` 放宽成「含 `hook` 就算」⇒ 这条被误删，变红
    /// （用户会莫名其妙丢掉另一个工具的会话监听）。
    #[test]
    fn foreign_shell_hook_is_not_ours_and_survives() {
        let foreign = "bash '/c/Users/u/.wb-switch/hook.sh'";
        assert!(
            !is_our_hook(foreign),
            "别的工具的 shell hook 不得被认成我们的：{foreign}"
        );

        let original = json!({
            "env": { "CODEBUDDY_AUTH_TOKEN": "x" },
            "hooks": {
                "FinalStop": [{ "matcher": "", "hooks": [{ "command": foreign, "type": "command" }] }],
                "Stop": [{ "matcher": "", "hooks": [{ "command": foreign, "type": "command" }] }],
            }
        });
        let installed = merge_hooks(&original, CMD, HOOK_PROFILE_CLAUDE);
        assert_eq!(
            installed["hooks"]["Stop"][0]["hooks"][0]["command"],
            json!(foreign),
            "别人的 hook 必须还在"
        );
        assert_eq!(installed["hooks"]["Stop"].as_array().unwrap().len(), 2);
        // ⚠️ `FinalStop` **不在**我们的 `HOOK_EVENTS` 里 ⇒ 那个事件我们一条都不加，
        //    只保留别人的（原样一条）。
        assert_eq!(
            installed["hooks"]["FinalStop"].as_array().unwrap().len(),
            1,
            "我们没注册的事件不得被追加条目"
        );
        assert_eq!(strip_our_hooks(&installed), original);
    }

    /// ★ 空配置 ⇒ 9 个事件各一条，且**只碰 `hooks` 键**。
    #[test]
    fn install_into_empty_config_touches_only_hooks() {
        let merged = merge_hooks(
            &json!({ "sandbox": { "enabled": true } }),
            CMD,
            HOOK_PROFILE_CLAUDE,
        );
        assert_eq!(
            merged["sandbox"]["enabled"],
            json!(true),
            "别的键必须原样保留"
        );
        let hooks = merged["hooks"].as_object().expect("hooks 必须是对象");
        assert_eq!(hooks.len(), HOOK_EVENTS.len());
        for event in HOOK_EVENTS {
            let groups = hooks[*event].as_array().expect("每事件一个数组");
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0]["hooks"][0]["command"], json!(CMD));
            assert_eq!(groups[0]["hooks"][0]["type"], json!("command"));
        }
    }

    /// `SessionEnd` 同步、其余带 `async`（否则会拖慢客户端的工具调用）。
    #[test]
    fn only_session_end_is_synchronous() {
        let merged = merge_hooks(&json!({}), CMD, HOOK_PROFILE_CLAUDE);
        assert!(merged["hooks"]["SessionEnd"][0]["hooks"][0]
            .get("async")
            .is_none());
        for event in ["PreToolUse", "Stop", "Notification"] {
            assert_eq!(merged["hooks"][event][0]["hooks"][0]["async"], json!(true));
        }
    }

    /// ★ **幂等**：装两次的结果与装一次逐字相同。
    ///
    /// 可证伪性：把 `rewrite_hooks` 里的「剔除自己的旧条目」删掉 ⇒ 变红（事件上会叠两条）。
    #[test]
    fn installing_twice_is_idempotent() {
        let once = merge_hooks(&json!({}), CMD, HOOK_PROFILE_CLAUDE);
        let twice = merge_hooks(&once, CMD, HOOK_PROFILE_CLAUDE);
        assert_eq!(once, twice, "重复安装不得叠加条目");
    }

    /// ★ **保留别人的 hook**（合并不是覆盖）。
    ///
    /// 可证伪性：把「保留别人的」改成「清空整个事件数组」⇒ 变红。
    #[test]
    fn other_peoples_hooks_survive() {
        let existing = json!({ "hooks": { "Stop": [others_hook()] } });
        let merged = merge_hooks(&existing, CMD, HOOK_PROFILE_CLAUDE);
        let groups = merged["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "别人的 + 我们的 = 2 条");
        assert_eq!(
            groups[0]["hooks"][0]["command"],
            json!("/usr/bin/python3 /home/u/other.py")
        );
    }

    /// ★ **同一分组内混装**时只剔掉我们那条，分组本身要留下。
    ///
    /// 可证伪性：把逐条过滤改成「整个分组丢掉」⇒ 别人的 hook 被误删，变红。
    #[test]
    fn shared_group_keeps_other_handlers() {
        let existing = json!({ "hooks": { "Stop": [{
            "matcher": "",
            "hooks": [
                { "type": "command", "command": "/usr/bin/python3 /home/u/other.py" },
                { "type": "command", "command": CMD },
            ]
        }] } });
        let merged = merge_hooks(&existing, CMD, HOOK_PROFILE_CLAUDE);
        let groups = merged["hooks"]["Stop"].as_array().unwrap();
        let handlers = groups[0]["hooks"].as_array().unwrap();
        assert_eq!(handlers.len(), 1, "我们那条应被剔除，别人的留下");
        assert_eq!(
            handlers[0]["command"],
            json!("/usr/bin/python3 /home/u/other.py")
        );
    }

    /// 裸 handler 形态（无 `hooks` 数组的扁平写法）也要能识别与剔除。
    #[test]
    fn bare_handler_form_is_recognized() {
        let existing = json!({ "hooks": { "Stop": [
            { "type": "command", "command": CMD },
            { "type": "command", "command": "/usr/bin/python3 /home/u/other.py" },
        ] } });
        let merged = merge_hooks(&existing, CMD, HOOK_PROFILE_CLAUDE);
        let groups = merged["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(groups.len(), 2, "别人的留下 + 我们新加的一条");
        assert_eq!(
            groups[0]["command"],
            json!("/usr/bin/python3 /home/u/other.py")
        );
        assert_eq!(groups[1]["hooks"][0]["command"], json!(CMD));
    }

    /// ★ **卸载只删我们的**，且删干净后**不留空数组键**。
    ///
    /// 可证伪性：把 `hooks.remove(*event)` 改成写 `Value::Array(vec![])` ⇒ 变红。
    #[test]
    fn uninstall_removes_our_entries_and_empty_keys() {
        let installed = merge_hooks(&json!({}), CMD, HOOK_PROFILE_CLAUDE);
        let stripped = strip_our_hooks(&installed);
        assert!(
            stripped.get("hooks").is_none(),
            "一条都不剩时必须删掉 hooks 键，而不是留空对象/空数组：{stripped}"
        );
        // 幂等：再卸一次同样干净
        assert_eq!(strip_our_hooks(&stripped), stripped);
    }

    /// 卸载**不碰**别人的 hook（装 → 卸 一轮后回到原样）。
    #[test]
    fn install_then_uninstall_restores_others() {
        let original = json!({ "hooks": { "Stop": [others_hook()] }, "sandbox": {} });
        let installed = merge_hooks(&original, CMD, HOOK_PROFILE_CLAUDE);
        let restored = strip_our_hooks(&installed);
        assert_eq!(restored, original, "装/卸一轮后必须逐字回到原状");
    }

    /// 不认识的形态（字符串等）**原样保留**（不能替用户删看不懂的东西）。
    #[test]
    fn unknown_shapes_are_preserved() {
        let existing = json!({ "hooks": { "Stop": ["weird", 42, null] } });
        let merged = merge_hooks(&existing, CMD, HOOK_PROFILE_CLAUDE);
        let groups = merged["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(groups[0], json!("weird"));
        assert_eq!(groups[1], json!(42));
        assert_eq!(groups[2], json!(null));
    }

    /// `hooks` 是脏类型（字符串）⇒ 当作空对象处理，不 panic。
    #[test]
    fn dirty_hooks_value_degrades_to_empty() {
        let merged = merge_hooks(&json!({ "hooks": "oops" }), CMD, HOOK_PROFILE_CLAUDE);
        assert_eq!(
            merged["hooks"].as_object().unwrap().len(),
            HOOK_EVENTS.len()
        );
        let stripped = strip_our_hooks(&json!({ "hooks": "oops" }));
        assert!(stripped.get("hooks").is_none());
    }

    /// ★ 自己的 hook 识别：认「我们的可执行文件 + hook 子命令」，**不认**路径。
    ///
    /// 可证伪性：把判据放宽成「含 hook 就算」⇒ 第一条断言变红（会把别人的 hook 删掉）。
    #[test]
    fn our_hook_detection_is_specific() {
        assert!(is_our_hook(CMD));
        assert!(is_our_hook("\"/opt/buddy-switch\" hook --source workbuddy"));
        assert!(!is_our_hook("/usr/bin/python3 /home/u/other.py"));
        assert!(!is_our_hook("agent-studio-runtime hook --source workbuddy"));
        assert!(!is_our_hook("\"C:\\x\\BuddySwitch.exe\" --version"));
    }

    /// `hook_command` 对含空格的路径加引号（Windows 安装目录的常态）。
    #[test]
    fn hook_command_quotes_the_executable_path() {
        let command = hook_command(
            Path::new("C:\\Program Files\\Buddy Switch\\BuddySwitch.exe"),
            "workbuddy",
        );
        assert!(
            command.starts_with('"'),
            "含空格的路径必须加引号：{command}"
        );
        assert!(command.ends_with("hook --source workbuddy"));
        assert!(is_our_hook(&command), "自己构造的命令必须被自己认出来");
    }

    /// `installed_events` 的判据（纯逻辑部分）：装过的认得出，没装的不认。
    #[test]
    fn installed_events_detects_our_entries() {
        let installed = merge_hooks(&json!({}), CMD, HOOK_PROFILE_CLAUDE);
        assert_eq!(
            HOOK_EVENTS
                .iter()
                .filter(|e| event_has_ours(&installed, e))
                .count(),
            HOOK_EVENTS.len()
        );
        let stripped = strip_our_hooks(&installed);
        assert_eq!(
            HOOK_EVENTS
                .iter()
                .filter(|e| event_has_ours(&stripped, e))
                .count(),
            0
        );
    }

    // ---- 档位（`HookProfile`）------------------------------------------------

    /// ★ `TRAE_HOOK_EVENTS ⊆ HOOK_EVENTS` —— **卸载只遍历 Claude 档**这件事全靠它。
    ///
    /// 可证伪性：往 `TRAE_HOOK_EVENTS` 里加一个 Claude 档没有的事件（如 `PostCompact`）
    /// ⇒ 变红。破了这条包含关系，`strip_our_hooks` 就**清不掉** Trae 档装进去的那条，
    /// 用户点「关闭监听」后客户端里仍留着我们的 hook（而且他会以为已经关了）。
    #[test]
    fn trae_events_are_a_subset_of_the_claude_set() {
        for event in TRAE_HOOK_EVENTS {
            assert!(
                HOOK_EVENTS.contains(event),
                "Trae 事件 {event} 不在 Claude 档里 —— 卸载会漏掉它"
            );
        }
        assert!(
            TRAE_HOOK_EVENTS.len() < HOOK_EVENTS.len(),
            "Trae 档应当比 Claude 档窄（它没有 PostToolUseFailure / PermissionRequest）"
        );
    }

    /// ★ Trae 档：**handler 里没有 `async`**，且只注册 Trae 认的事件。
    ///
    /// 可证伪性：把 `HOOK_PROFILE_TRAE.async_handlers` 改成 `true`
    /// ⇒ 第一条断言变红（Trae 的 handler 结构里没有这个字段）。
    #[test]
    fn trae_profile_omits_async_and_unknown_events() {
        let merged = merge_hooks(&json!({}), CMD, HOOK_PROFILE_TRAE);
        let hooks = merged["hooks"].as_object().expect("hooks 必须是对象");
        assert_eq!(hooks.len(), TRAE_HOOK_EVENTS.len());
        for event in TRAE_HOOK_EVENTS {
            let node = &merged["hooks"][event][0]["hooks"][0];
            assert!(
                node.get("async").is_none(),
                "Trae 的 handler 不得带 async（{event}）—— 那是它不认识的字段"
            );
            assert_eq!(node["type"], json!("command"));
            assert_eq!(node["timeout"], json!(HOOK_TIMEOUT_SECONDS));
            assert_eq!(node["statusMessage"], json!(HOOK_STATUS_MESSAGE));
        }
        // Trae 不认的两个事件**一个键都不能出现**。
        assert!(merged["hooks"].get("PostToolUseFailure").is_none());
        assert!(merged["hooks"].get("PermissionRequest").is_none());
        // 对照组：Claude 档**必须**带 async（否则这条用例会因为两边一样而失去意义）。
        let claude = merge_hooks(&json!({}), CMD, HOOK_PROFILE_CLAUDE);
        assert_eq!(
            claude["hooks"]["PreToolUse"][0]["hooks"][0]["async"],
            json!(true)
        );
    }

    /// ★ Trae 的 hooks 文件顶层是 `{version, hooks}` —— **`version` 必须逐字保留**。
    ///
    /// 我们**不复刻** `version`（不知道它的取值域），靠的就是「只替换 `hooks` 键」这一点。
    /// 可证伪性：把 `rewrite_hooks` 改成「整份 doc 重建」⇒ 变红。
    #[test]
    fn trae_merge_preserves_the_version_key() {
        let original = json!({
            "version": "1",
            "hooks": { "Stop": [{ "matcher": "", "hooks": [{ "type": "command", "command": "someone-else" }] }] }
        });
        let merged = merge_hooks(&original, CMD, HOOK_PROFILE_TRAE);
        assert_eq!(merged["version"], json!("1"), "version 必须原样保留");
        assert_eq!(
            merged["hooks"]["Stop"][0]["hooks"][0]["command"],
            json!("someone-else"),
            "别人的条目必须保留"
        );
        assert_eq!(merged["hooks"]["Stop"].as_array().unwrap().len(), 2);
        // 装 → 卸 一轮逐字回到原状（version 也算在内）。
        assert_eq!(strip_our_hooks(&merged), original);
    }

    /// ★ **两档混装**时，一次卸载把它们**都**清干净。
    ///
    /// 场景真实存在：用户先在 Trae 档装上（7 个事件），后来我们改了档位、或他换了客户端。
    /// 卸载走 Claude 档遍历 ⇒ 两档装的事件都在覆盖范围内。
    ///
    /// 可证伪性：把 `strip_our_hooks` 改成只遍历 `TRAE_HOOK_EVENTS` ⇒ 变红
    /// （Trae 档没有 `SessionEnd`？—— 有，但它缺 `PostToolUseFailure`/`PermissionRequest`，
    /// 这两个事件上 Claude 档装的条目就清不掉了）。
    #[test]
    fn strip_removes_entries_installed_by_either_profile() {
        let base = json!({ "hooks": {} });
        let by_claude = merge_hooks(&base, CMD, HOOK_PROFILE_CLAUDE);
        let by_trae = merge_hooks(&base, CMD, HOOK_PROFILE_TRAE);
        for installed in [by_claude, by_trae] {
            let stripped = strip_our_hooks(&installed);
            assert!(
                stripped.get("hooks").is_none(),
                "两档装进去的条目都必须能被一次卸载清干净：{stripped}"
            );
        }
    }

    /// ★ 档位按**槽位**选（唯一取值点 = `client_settings` 的 `SlotKind`）。
    ///
    /// 可证伪性：把 `profile_for` 改成恒返回 Claude 档 ⇒ Trae 两个槽位的断言变红。
    #[test]
    fn profile_selection_follows_the_slot() {
        assert_eq!(profile_for("trae-hooks-cn"), HOOK_PROFILE_TRAE);
        assert_eq!(profile_for("trae-hooks-global"), HOOK_PROFILE_TRAE);
        assert_eq!(profile_for("claude"), HOOK_PROFILE_CLAUDE);
        assert_eq!(profile_for("codebuddy"), HOOK_PROFILE_CLAUDE);
        assert_eq!(profile_for("workbuddy-cn"), HOOK_PROFILE_CLAUDE);
        assert_eq!(profile_for("workbuddy-global"), HOOK_PROFILE_CLAUDE);
        // 未知槽位走**宽档**（Claude）：多写一个 `async` 的代价，小于漏掉一个 client 的
        // 全部事件。且未知槽位本来就不可能进安装目标表。
        assert_eq!(profile_for("nope"), HOOK_PROFILE_CLAUDE);
    }

    /// 诊断用的「装了几条」必须**按该槽位的档位**数，否则 Trae 会显示成「7/9 已装」。
    #[test]
    fn installed_event_counts_follow_the_profile() {
        let claude = merge_hooks(&json!({}), CMD, HOOK_PROFILE_CLAUDE);
        assert_eq!(
            installed_events_of(&claude, HOOK_PROFILE_CLAUDE).len(),
            HOOK_EVENTS.len()
        );
        let trae = merge_hooks(&json!({}), CMD, HOOK_PROFILE_TRAE);
        assert_eq!(
            installed_events_of(&trae, HOOK_PROFILE_TRAE).len(),
            TRAE_HOOK_EVENTS.len(),
            "Trae 档装完就是 7 条，不该按 Claude 的 9 条去数"
        );
        // ★ 交叉对照：Trae 装出来的东西按 Claude 档去数会**少 2 条** ——
        //    这正是「按档位数」这件事有意义的证据（否则两个数字会相等，断言恒真）。
        assert_eq!(
            installed_events_of(&trae, HOOK_PROFILE_CLAUDE).len(),
            TRAE_HOOK_EVENTS.len(),
            "Trae 档没装 Claude 独有的两个事件 ⇒ 按 Claude 档数也只能数到 7"
        );
    }
}
