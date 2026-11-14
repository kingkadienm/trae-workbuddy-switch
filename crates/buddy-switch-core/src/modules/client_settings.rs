//! 统一托管**上游客户端自身**的 `settings.json`（读 · 编辑 · 备份 · 恢复 · 差异比对）。
//!
//! ⚠️ 本模块管的是**被本工具代理的上游客户端**（WorkBuddy 两版 + Trae 四个程序位）
//! 自己的 `settings.json`，**不是**本工具自己的 `~/.buddy-switch/**`，也与
//! `modules::trae::settings`（本工具的 Trae 代理设置，落在 `~/.buddy-switch/trae/`）
//! **是两个不同的文件**，只是名字撞车。
//!
//! ## 槽位与路径（唯一取值点）
//!
//! | 槽位 key | 产品 | 路径 |
//! |:---|:---|:---|
//! | `workbuddy-cn` | WorkBuddy | `~/.workbuddy/settings.json` |
//! | `workbuddy-global` | WorkBuddy | `~/.workbuddy-ai/settings.json` |
//! | `trae-code-cn` | Trae | `%APPDATA%\Trae CN\User\settings.json` |
//! | `trae-code-global` | Trae | `%APPDATA%\Trae\User\settings.json` |
//! | ~~`trae-work-cn`~~ | ~~Trae~~ | ★ issue #28 **已退役**（客户端被合并卸载） |
//! | ~~`trae-work-global`~~ | ~~Trae~~ | ★ 同上 |
//!
//! - WorkBuddy 侧走 [`session_data_dir`]（CN `.workbuddy` / Global `.workbuddy-ai`）；
//! - Trae 侧**必须**走 [`trae_platform::user_data_dir_for_program`]（按**程序位**精确取
//!   该程序位自己声明的唯一目录名），**不能**走 `detect_data_dir_for` —— 后者的变体候选表
//!   里混着 `TRAE SOLO`（**国际版客户端**），缺席时会静默落到另一个客户端。
//!
//! ⚠️ **不读** `%APPDATA%\WorkBuddy\User\settings.json`：本机不存在该目录（PRD 明确禁止硬编码）。
//!
//! ## 为什么必须「外科式文本改写」，而不是 `to_string_pretty` 整文件覆盖
//!
//! `crates/buddy-switch-core/Cargo.toml` 的 `serde_json` **未开 `preserve_order`**
//! ⇒ `serde_json::Map` 是 `BTreeMap`，解析再序列化会把键**按字典序重排**，产生与内容无关的
//! 大 diff，并抹掉用户手改的排版（实测四种风格：2 空格 LF / tab CRLF / 无缩进 / 压缩式）。
//!
//! 因此本模块自带一个**只读 JSON 扫描器**（按字节递归下降、处理字符串转义；UTF-8 多字节
//! 的高位为 1，不会与 `"` / `\` / `{}` / `[]` 冲突，故按字节扫描是安全的），产出带
//! **字节跨度**的节点树：
//!
//! - **替换**已存在的键：只把 `text[value.start..value.end]` 换成按原风格渲染的新值，
//!   其余字节**逐字不动**（保证「diff 只含目标键那一行」）；
//! - **新增**键：插到父对象最后一个成员之后（`,\n<indent>"k": v`），空对象插在 `{` 之后；
//!   **绝不重排已有键**。
//!
//! 写回流程：渲染 → **自读一遍校验 JSON 合法** → 备份 → [`atomic_write`]。
//! 校验不过**不替换**。
//!
//! ## 备份（模块内部，任何调用方都绕不过）
//!
//! 落到 `backups/client-settings/<slot>/<utc_iso>/settings.json`。触发须**同时**满足
//! 「目标原本存在」**且**「本次实际发生改写」⇒ **幂等**：同一 patch 两次只产生一份备份。
//! 备份失败**不阻断**写入（返回 `backup: null`，与「无需备份」同形，靠 `changed` 区分）。
//!
//! ## 写前必须判进程
//!
//! 客户端运行中写 `settings.json` 会被其内存态覆盖（VS Code 系退出时才落盘）
//! ⇒ [`ClientSlot::is_running`] 为 true 时**直接拒绝**，不静默写。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::modules::config::{atomic_write, backup_dir, home_dir, utc_iso};
use crate::modules::error_code::{AppError, ErrorCode};
use crate::modules::process;
use crate::modules::region::Region;
use crate::modules::session::session_data_dir;
use crate::modules::trae::platform as trae_platform;
use crate::modules::trae::region::{TraeProgram, TraeRegion};
use crate::modules::trae::variant::{all_specs, TraeVariant};

// ---------------------------------------------------------------------------
// 槽位规格表
// ---------------------------------------------------------------------------

/// 一个「可被本工具托管设置的上游客户端实例」。
pub struct ClientSlot {
    /// 稳定标识（前端传参 / 备份目录名 / 错误参数）。**发布后不得改名**。
    pub key: &'static str,
    /// i18n 键（模块级常量表**存键不存文案**，由前端渲染成当前语言）。
    pub display_key: &'static str,
    /// 所属产品（`workbuddy` / `trae`），供前端分组。
    pub product: &'static str,
    /// 路径 / 进程判定的唯一取值点。
    kind: SlotKind,
}

/// 槽位的内部实现维度：WorkBuddy 按 [`Region`]，Trae 按 [`TraeVariant`]，
/// CLI 系客户端按 [`HomeFile`]。
///
/// ⚠️ Trae 侧存**变体**而不是 `(区域, 程序)`：`is_running_for` 直接吃变体；
/// 路径侧再由 [`program_of_variant`] 推 `(TraeRegion, TraeProgram)`。
#[derive(Debug, Clone, Copy)]
enum SlotKind {
    Workbuddy(Region),
    Trae(TraeVariant),
    HomeFile(HomeFile),
    /// Trae 的 **hooks 文件**（`hooks.json`），按**区域**分家。
    ///
    /// ## 为什么不能复用 [`SlotKind::Trae`]
    ///
    /// - **路径不同**：`SlotKind::Trae` 指向 `%APPDATA%\<userDataDirName>\User\settings.json`
    ///   （`user_data_dir_for_program`）；hooks 文件在产品数据目录 `~/.trae-cn` / `~/.trae` 下
    ///   （客户端 `product.json` 的 `dataFolderName`，见 [`trae_hooks_dir`]）。
    /// - **粒度不同**：Trae 有 6 个变体，但**只有 2 个 hooks 文件**（国内 / 国际各一）。
    ///   按变体建模会得到 6 个槽位指向 2 个文件 —— 用户会看到「Trae Work CN」和
    ///   「TraeCode CN」两条状态，其实改的是同一个文件。
    ///
    /// 进程守卫则相反：**要按区域跨变体判**（任何一个该区域的 Trae 在跑都要拦）——
    /// 它们都会在退出时把内存态写回同一份全局资产。
    TraeHooks(TraeRegion),
}

/// 用户主目录下的一个 JSON 配置文件（CLI 系客户端的槽位形态）。
///
/// ## 为什么单独一档，而不是复用 Workbuddy / Trae
///
/// 这两档的路径都由「客户端自己的目录约定」推出（`session_data_dir` /
/// `user_data_dir_for_program`）。Claude Code / CodeBuddy CLI 把配置直接放在
/// 家目录下的点目录里（`~/.claude/settings.json`），**没有** `%APPDATA%` 那一层，
/// 也没有区域 / 程序位之分 ⇒ 硬套现有两档会把路径算错（算错的表现是
/// `installed: false`，看起来像「没装」）。
#[derive(Debug, Clone, Copy)]
struct HomeFile {
    /// 家目录下的目录名（如 `.claude`）。
    dir: &'static str,
    /// 配置文件名（如 `settings.json`）。
    file: &'static str,
    /// 进程映像名（写前守卫用；见 `process::is_image_running`）。
    ///
    /// ⚠️ 必须是**完整映像名**（Windows 上带 `.exe`）。填错的表现是守卫永远放行
    /// —— 那不是「少一道检查」，而是「看起来有、实际没有」的假护栏。
    image: &'static str,
}

const WORKBUDDY_CN: ClientSlot = ClientSlot {
    key: "workbuddy-cn",
    display_key: "clientSettings.slot.workbuddyCn",
    product: "workbuddy",
    kind: SlotKind::Workbuddy(Region::Cn),
};

const WORKBUDDY_GLOBAL: ClientSlot = ClientSlot {
    key: "workbuddy-global",
    display_key: "clientSettings.slot.workbuddyGlobal",
    product: "workbuddy",
    kind: SlotKind::Workbuddy(Region::Global),
};

/// Trae **国内版**的代理设置槽位（客户端 `Trae CN`）。
///
/// ★ issue #28 收敛后每区域**只有这一个**槽位：`trae-code-cn` / `trae-code-global`
/// 曾与 `trae-work-cn` / `trae-work-global` **编辑同一个文件**
/// （`%APPDATA%\Trae CN\User\settings.json` —— 合并版复用了 TraeCode 的 userData），
/// 两条槽位各写各的键必然互相覆盖。收敛后只剩一个写者，问题从根上消失。
///
/// ⚠️ **键名沿用 `trae-code-*` 而不改名**：它是**写进客户端 settings.json 的键**，
/// 改名等于让用户已配置的代理设置「凭空消失」（读路径只认新键）。
const TRAE_CODE_CN: ClientSlot = ClientSlot {
    key: "trae-code-cn",
    display_key: "clientSettings.slot.traeCodeCn",
    product: "trae",
    kind: SlotKind::Trae(TraeVariant::TraeMerged),
};

/// Trae **国际版**的代理设置槽位（客户端 `Trae`）。
const TRAE_CODE_GLOBAL: ClientSlot = ClientSlot {
    key: "trae-code-global",
    display_key: "clientSettings.slot.traeCodeGlobal",
    product: "trae",
    kind: SlotKind::Trae(TraeVariant::GlobalTraeMerged),
};

/// 声明式规格表：**4 个**槽位，**顺序稳定**。新增变体只加一行、不改逻辑。
///
/// ⚠️ 未安装者**不从列表消失**（`installed: false` 照常列出）—— 沿用「不因探测失败而消失、
/// 不伪造成功」的纪律。
///
/// ⚠️ **本表只放「VS Code 系代理设置槽位」**：`语义映射表与槽位表对齐` 那条单测要求
/// 每个成员都在 [`PROXY_FIELDS`] 里有代理路径声明。CLI 系客户端（没有 `http.proxy`
/// 这类字段）**不进本表**，见 [`HOOK_FILE_SLOTS`]。
///
/// ★ issue #28：Trae 原先 4 个槽位（Work / Code × 两区域），收敛后**只剩 2 个**
/// （每区域一个合并版客户端）。退役槽位**不得**留在本表里 —— 它们指向已卸载的客户端，
/// 留着会让「客户端环境」页报出四个程序位，其中两个永远未安装。
pub const CLIENT_SLOTS: &[ClientSlot] = &[
    WORKBUDDY_CN,
    WORKBUDDY_GLOBAL,
    TRAE_CODE_CN,
    TRAE_CODE_GLOBAL,
];

// ---------------------------------------------------------------------------
// hook 安装目标槽位（CLI 系客户端）
// ---------------------------------------------------------------------------

/// Claude Code（`~/.claude/settings.json` 的 command hooks）。
const CLAUDE: ClientSlot = ClientSlot {
    key: "claude",
    display_key: "clientSettings.slot.claude",
    product: "agent",
    kind: SlotKind::HomeFile(HomeFile {
        dir: ".claude",
        file: "settings.json",
        image: "claude.exe",
    }),
};

/// CodeBuddy CLI（`~/.codebuddy/settings.json` 的 command hooks）。
const CODEBUDDY: ClientSlot = ClientSlot {
    key: "codebuddy",
    display_key: "clientSettings.slot.codebuddy",
    product: "agent",
    kind: SlotKind::HomeFile(HomeFile {
        dir: ".codebuddy",
        file: "settings.json",
        image: "codebuddy.exe",
    }),
};

/// **只服务 hook 安装**的槽位表（CLI 系客户端 + Trae 的 hooks 文件）。
///
/// ## 为什么与 [`CLIENT_SLOTS`] 分开
///
/// `CLIENT_SLOTS` 的语义是「VS Code 系客户端里我们托管**代理设置**的槽位」——
/// 它被 [`PROXY_FIELDS`] 的语义表**逐槽位**要求声明代理路径（有单测钉住）。
/// Claude Code / CodeBuddy CLI 是**命令行客户端**，没有 `http.proxy` /
/// `trae.network.proxy.effectiveMode` 这类字段；Trae 的 `hooks.json` 更是一个**独立文件**
/// （不是 settings.json 的键）。塞进 `CLIENT_SLOTS` 只会让两张表互相说谎。
///
/// ## 但写入仍然只有一条路
///
/// 本表只影响「哪些槽位**存在**」；读 / 写 / 备份 / 进程守卫 / 格式指纹保持
/// 全部复用本模块同一套原语（[`read_settings`] / [`patch_settings`] / [`patch_at`]）。
/// 换言之：**分开的是「表」，不是「写入路径」**（`P15-SPIKE-RESULTS §6 D41` 那条纪律）。
pub const HOOK_FILE_SLOTS: &[ClientSlot] = &[CLAUDE, CODEBUDDY, TRAE_HOOKS_CN, TRAE_HOOKS_GLOBAL];

/// Trae 的 **产品数据目录名**（客户端 `product.json` 的 `dataFolderName`）。
///
/// ## 取证
///
/// - `D:\Programs\Trae CN\resources\app\product.json` 的 `dataFolderName` = `.trae-cn`；
///   国际版 `D:\Programs\Trae\...\product.json` = `.trae`。
/// - 该目录在**本机确实存在**且承载全局资产：`~/.trae-cn/{skills,commands,rules,permission,plugins,builtin}`。
/// - Trae 的 hook 资产注册表（`resources/app/out/vs/workbench/workbench.desktop.main.js`）：
///   `{assetType:"hook", scope:3, projectRelPath:".trae/hooks.json", globalRelPath:"hooks.json"}`
///   —— `scope:3` = 项目 + 全局两侧；**全局侧是 `<产品数据目录>/hooks.json`**。
///
/// ⇒ 全局 hooks 文件 = `~/.trae-cn/hooks.json`（国内）/ `~/.trae/hooks.json`（国际）。
///
/// ⚠️ **不要**用 `%APPDATA%\<userDataDirName>`（那是 [`SlotKind::Trae`] 的 settings.json 所在处，
/// 两者刻意不同源）。
fn trae_hooks_dir(region: TraeRegion) -> &'static str {
    match region {
        TraeRegion::Cn => ".trae-cn",
        TraeRegion::Global => ".trae",
    }
}

/// Trae **国内版**的 hooks 文件（`~/.trae-cn/hooks.json`）。
const TRAE_HOOKS_CN: ClientSlot = ClientSlot {
    key: "trae-hooks-cn",
    display_key: "clientSettings.slot.traeHooksCn",
    product: "agent",
    kind: SlotKind::TraeHooks(TraeRegion::Cn),
};

/// Trae **国际版**的 hooks 文件（`~/.trae/hooks.json`）。
const TRAE_HOOKS_GLOBAL: ClientSlot = ClientSlot {
    key: "trae-hooks-global",
    display_key: "clientSettings.slot.traeHooksGlobal",
    product: "agent",
    kind: SlotKind::TraeHooks(TraeRegion::Global),
};

/// 变体 → `(区域, 程序)`。
///
/// ★ issue #28 收敛后**只看区域**：每个区域只有一个程序位（合并版），
/// 所以退役变体（`TraeWork` / `Trae` / `Global` / `GlobalTraeCode`）解析到
/// **本区域的合并版程序位**。它们的槽位已退役，老记录里的 variant 字段仍要能解码。
///
/// ⚠️ **本函数与 [`variant_for_program`] 互为逆**（有单测钉住双向一致）。
fn program_of_variant(variant: TraeVariant) -> (TraeRegion, TraeProgram) {
    let region = variant.region();
    (region, TraeProgram::Trae)
}

/// `(区域, 程序)` → 变体，即 [`program_of_variant`] 的逆映射（**唯一事实来源**）。
///
/// 仓库里**没有**别的 `variant_for_program`（`platform::variant_for_program` 是**私有**的），
/// 故在此提供一份公开的、与 [`program_of_variant`] 互为逆的映射，并有单测钉住双向一致。
///
/// ★ issue #28：程序位退化成「每区域一个」后，本函数是**区域 → 合并版变体**的纯映射；
/// `program` 参数被刻意忽略（任何程序位都写到该区域那一个客户端）。
pub fn variant_for_program(region: TraeRegion, _program: TraeProgram) -> TraeVariant {
    match region {
        TraeRegion::Cn => TraeVariant::TraeMerged,
        TraeRegion::Global => TraeVariant::GlobalTraeMerged,
    }
}

impl ClientSlot {
    /// 该槽位的 `settings.json` 路径（**唯一取值点**；目录不保证存在）。
    ///
    /// 取不到基目录（`%APPDATA%` / 用户主目录都拿不到）时返回 `None`。
    pub fn settings_path(&self) -> Option<PathBuf> {
        match self.kind {
            SlotKind::Workbuddy(region) => Some(session_data_dir(region).join("settings.json")),
            SlotKind::Trae(variant) => {
                let (region, program) = program_of_variant(variant);
                trae_platform::user_data_dir_for_program(region, program)
                    .map(|dir| dir.join("User").join("settings.json"))
            }
            // CLI 系：配置直接在家目录的点目录里，没有 `%APPDATA%` 那一层。
            // ⚠️ 走 `config::home_dir()` 而**不是** `dirs::home_dir()`：前者尊重
            //    `BUDDY_SWITCH_HOME` 覆盖（测试据此把整个家目录挪到临时目录）。
            //    绕过它会让测试**写到真实用户的 `~/.claude/settings.json`**。
            SlotKind::HomeFile(spec) => Some(home_dir().join(spec.dir).join(spec.file)),
            // Trae 的 hooks 文件在产品数据目录（`~/.trae-cn` / `~/.trae`）下，见 [`trae_hooks_dir`]。
            SlotKind::TraeHooks(region) => {
                Some(home_dir().join(trae_hooks_dir(region)).join("hooks.json"))
            }
        }
    }

    /// 该槽位的客户端进程是否正在运行（**写前必须判**）。
    pub fn is_running(&self) -> bool {
        match self.kind {
            SlotKind::Workbuddy(region) => process::is_workbuddy_running_for(region),
            SlotKind::Trae(variant) => trae_platform::is_running_for(variant),
            SlotKind::HomeFile(spec) => process::is_image_running(spec.image),
            // ★ **按区域跨变体判**：该区域**任何一个** Trae 变体在跑都拦。
            //   6 个变体共享 2 个 hooks 文件，只判一个变体会让另一个变体现在退出时
            //   把内存态写回、把我们刚写的内容覆盖掉（正是守卫要防的事）。
            //   遍历一律走 `all_specs()`（6 条），**不用** `TraeVariant::all()`（只有国内 2 条）。
            SlotKind::TraeHooks(region) => all_specs()
                .iter()
                .filter(|spec| spec.variant.region() == region)
                .any(|spec| trae_platform::is_running_for(spec.variant)),
        }
    }
}

/// 按 key 查槽位；未知返回 `None`。
///
/// ⚠️ **两张表都查**：[`CLIENT_SLOTS`]（代理设置槽位）+ [`HOOK_FILE_SLOTS`]（hook 安装槽位）。
/// 这样 `read_settings` / `patch_settings` / 备份 / 进程守卫对 CLI 系槽位**原样生效** ——
/// 写入路径只有一条，不因为多了一张表就分叉。
pub fn slot_by_key(key: &str) -> Option<&'static ClientSlot> {
    CLIENT_SLOTS
        .iter()
        .chain(HOOK_FILE_SLOTS.iter())
        .find(|slot| slot.key == key)
}

/// 该槽位是不是 **Trae 的 hooks 文件**。
///
/// 唯一用途：`agent_hook_install::profile_for` 据此选格式档位（Trae 的事件集与
/// handler 字段与 Claude 系**不同**，见 `HookProfile` 的表）。
///
/// ⚠️ 判据落在**槽位的实现维度**上（`SlotKind`），而不是让调用方再抄一份槽位 key 列表 ——
/// 抄一份就会在两处漂移（新增 Trae 槽位时漏掉一处 ⇒ 用错格式档位 ⇒ 写出客户端不认的字段）。
pub fn is_trae_hooks_slot(key: &str) -> bool {
    matches!(
        slot_by_key(key).map(|slot| slot.kind),
        Some(SlotKind::TraeHooks(_))
    )
}

// ---------------------------------------------------------------------------
// 语义映射表（与**路径规格表** [`CLIENT_SLOTS`] 刻意分离）
// ---------------------------------------------------------------------------
//
// PRD Requirement 7：规格表**只声明路径**，字段语义另立本表 —— 两张表分离，
// 避免 Trae 新增字段时污染路径表。

/// WorkBuddy 侧 `http.proxySupport` 的**取值域**（VS Code 语义）。
///
/// 取证：WorkBuddy 自带 bundle 的 `parseProxySettings` 直接分支 `"off"` / `"on"`
/// （`D:\Program Files\WorkBuddy\resources\app.asar.unpacked\cli\dist\codebuddy-headless.js`）。
/// `"override"` 是 VS Code 同名字段的第三个枚举值；本机 `~/.workbuddy/settings.json`
/// **未出现**该键，故 `"override"` 属**未逐字节复核**。
///
/// ⚠️ 本常量**只做展示 / 校验**，**不**用于「一处改两边生效」的写映射（PRD Out of scope）。
pub const WORKBUDDY_PROXY_SUPPORT_VALUES: &[&str] = &["off", "on", "override"];

/// Trae 侧 `trae.network.proxy.effectiveMode` 的**取值域**。
///
/// 取证（逐字节）：Trae 客户端 bundle `D:\Programs\Trae CN\resources\app\out\main.js` 内
/// `function jfe(t){return t==="system"||t==="direct"||t==="manual"||t==="cloud"?t:"system"}`
/// —— 合法值恰为这 4 个，非法值回落 `"system"`。本机 4 份 Trae settings.json 实测均为
/// `"system"`。
///
/// ⚠️ 同上：**只做展示 / 校验**，不用于写映射。
pub const TRAE_PROXY_MODE_VALUES: &[&str] = &["system", "direct", "manual", "cloud"];

/// 跨产品**语义等价**字段：`(语义名, 槽位 key, 该槽位里的点号路径)`。
///
/// 当前只有**一组**真正跨产品等价（PRD §四）：代理。
///
/// | 语义 | WorkBuddy | Trae（4 变体） |
/// |:---|:---|:---|
/// | `proxy` | `http.proxy` + `http.proxySupport` | `trae.network.proxy.effectiveMode` |
///
/// ⚠️ 本表**只用于差异比对的归并展示**（跨产品时把两侧代理字段并成一组），
/// **不**用于「一处改两边生效」：两侧取值域语义不同（WorkBuddy 是 VS Code 的
/// `http.proxy`(URL) + `http.proxySupport`(开关)，Trae 是单一 `effectiveMode`(模式)），
/// **无法无损互写**（PRD Out of scope 明确不做）。
pub const PROXY_FIELDS: &[(&str, &str, &str)] = &[
    ("proxy", "workbuddy-cn", "http.proxy"),
    ("proxy", "workbuddy-cn", "http.proxySupport"),
    ("proxy", "workbuddy-global", "http.proxy"),
    ("proxy", "workbuddy-global", "http.proxySupport"),
    ("proxy", "trae-code-cn", "trae.network.proxy.effectiveMode"),
    (
        "proxy",
        "trae-code-global",
        "trae.network.proxy.effectiveMode",
    ),
];

/// 某槽位在 [`PROXY_FIELDS`] 里声明的**全部**代理字段路径。
fn proxy_paths_of(slot_key: &str) -> Vec<&'static str> {
    PROXY_FIELDS
        .iter()
        .filter(|(_, slot, _)| *slot == slot_key)
        .map(|(_, _, path)| *path)
        .collect()
}

// ---------------------------------------------------------------------------
// 公开命令面（四通道共用；两宿主**原样返回**这些 `Value`，不得各包一层）
// ---------------------------------------------------------------------------

/// `list_client_settings_slots`：列出 6 个槽位（未安装者 `installed: false` **仍在列表**）。
///
/// ⚠️ 刻意**不做进程探测**：这里要跑 `tasklist`（慢），而列表只回答「有没有这个文件」。
/// 运行态由 [`read_settings`] / [`patch_settings`] 按需给出。
pub fn list_slots() -> Value {
    let slots: Vec<Value> = CLIENT_SLOTS.iter().map(slot_view).collect();
    json!({ "slots": slots })
}

fn slot_view(slot: &ClientSlot) -> Value {
    let path = slot.settings_path();
    let installed = path.as_ref().map(|p| p.is_file()).unwrap_or(false);
    json!({
        "key": slot.key,
        "displayKey": slot.display_key,
        "product": slot.product,
        "installed": installed,
        "path": path.map(|p| p.to_string_lossy().to_string()),
    })
}

/// `read_client_settings`：原文 + 解析后键值 + 格式指纹。
///
/// 解析失败返回 [`ErrorCode::ClientSettingsParseFailed`] 错误，**绝不**返回空对象。
/// 未安装（文件不存在）**不是错误**：返回 `installed: false` 且 `text` / `values` 为 `null`。
pub fn read_settings(slot_key: &str) -> Result<Value, String> {
    let slot = slot_by_key(slot_key).ok_or_else(|| unknown_slot(slot_key))?;
    let path = slot
        .settings_path()
        .ok_or_else(|| locate_failed(slot, slot_key))?;
    let installed = path.is_file();
    let (text, values, key_order, fingerprint) = if installed {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("读取客户端设置失败（{}）：{e}", path.display()))?;
        let tree = parse_json(&text).map_err(|e| parse_failed(&path, &e))?;
        let value: Value =
            serde_json::from_str(&text).map_err(|e| parse_failed(&path, &e.to_string()))?;
        let order = top_level_order(&tree);
        // 先取指纹再移动 `text`（`fingerprint_of_text` 借 `&str`，移动后不能再借）。
        let fingerprint = fingerprint_of_text(&text);
        (Some(text), Some(value), order, Some(fingerprint))
    } else {
        (None, None, Vec::new(), None)
    };

    Ok(json!({
        "slot": slot.key,
        "displayKey": slot.display_key,
        "product": slot.product,
        "path": path.to_string_lossy(),
        "installed": installed,
        "running": installed && slot.is_running(),
        "text": text,
        "values": values,
        "keyOrder": key_order,
        "fingerprint": fingerprint.map(|f| json!({
            "indent": f.indent,
            "newline": f.newline,
        })),
    }))
}

/// `patch_client_settings`：键值级 patch，**格式保持**，模块内部自动备份。
///
/// `patch` 形如 `{ "<点号路径>": <新值> }`（如 `claw.channels.wechatmp.enabled`）。
pub fn patch_settings(slot_key: &str, patch: &Value) -> Result<Value, String> {
    let slot = slot_by_key(slot_key).ok_or_else(|| unknown_slot(slot_key))?;
    let path = slot
        .settings_path()
        .ok_or_else(|| locate_failed(slot, slot_key))?;
    if slot.is_running() {
        return Err(process_running(slot));
    }
    patch_at(&path, patch, slot, &backup_dir())
}

/// `list_client_settings_backups`：该槽位的历史备份点（按 id 降序）。
pub fn list_backups(slot_key: &str) -> Result<Value, String> {
    let slot = slot_by_key(slot_key).ok_or_else(|| unknown_slot(slot_key))?;
    Ok(list_backups_in(slot, &backup_dir()))
}

/// `restore_client_settings_backup`：整文件恢复；**恢复前对当前文件再备份一次**。
pub fn restore_backup(slot_key: &str, backup_id: &str) -> Result<Value, String> {
    let slot = slot_by_key(slot_key).ok_or_else(|| unknown_slot(slot_key))?;
    validate_backup_id(backup_id)?;
    let path = slot
        .settings_path()
        .ok_or_else(|| locate_failed(slot, slot_key))?;
    if slot.is_running() {
        return Err(process_running(slot));
    }
    restore_at(&path, slot, backup_id, &backup_dir())
}

/// `diff_client_settings`：任选两个槽位比对，输出「仅 A 有 / 仅 B 有 / 值不同」三类。
///
/// 跨产品比对时，代理字段按 [`PROXY_FIELDS`] **归并**成 `proxyGroups` 一组
/// （否则会各报一条 `onlyA` + `onlyB`，看不出「其实是同一件事」）。
pub fn diff_slots(slot_a: &str, slot_b: &str) -> Result<Value, String> {
    let a = slot_by_key(slot_a).ok_or_else(|| unknown_slot(slot_a))?;
    let b = slot_by_key(slot_b).ok_or_else(|| unknown_slot(slot_b))?;
    let (a_path, a_installed, a_values) = load_values(a)?;
    let (b_path, b_installed, b_values) = load_values(b)?;
    let mut out = diff_values(&a_values, a.key, a.product, &b_values, b.key, b.product);
    out["slotA"] = json!({
        "key": a.key, "displayKey": a.display_key, "path": a_path, "installed": a_installed,
    });
    out["slotB"] = json!({
        "key": b.key, "displayKey": b.display_key, "path": b_path, "installed": b_installed,
    });
    Ok(out)
}

/// 两个已解析值的比对（**可测**，不依赖真实槽位路径）。
///
/// 跨产品（`a_product != b_product`）时按 [`PROXY_FIELDS`] 归并代理字段；
/// 同产品时两侧路径本就相同，走普通三类即可，不做归并。
fn diff_values(
    a: &Value,
    a_key: &str,
    a_product: &str,
    b: &Value,
    b_key: &str,
    b_product: &str,
) -> Value {
    let mut a_flat = flatten(a);
    let mut b_flat = flatten(b);
    // 跨产品：代理字段按 [`PROXY_FIELDS`] 归并（同产品两侧路径本就相同，走普通三类）。
    let proxy_groups = if a_product != b_product {
        merge_proxy_fields(&mut a_flat, a_key, &mut b_flat, b_key)
    } else {
        Vec::new()
    };
    let mut out = diff_flat(&a_flat, &b_flat);
    // 形状稳定：同产品 / 无声明时为空数组（两宿主返回逐字相同）。
    out["proxyGroups"] = Value::Array(proxy_groups);
    out
}

fn load_values(slot: &ClientSlot) -> Result<(String, bool, Value), String> {
    let path = slot
        .settings_path()
        .ok_or_else(|| locate_failed(slot, slot.key))?;
    let display = path.to_string_lossy().to_string();
    if !path.is_file() {
        // 未安装 ⇒ 空对象（**不是**错误：比对两侧一方缺席是常态）。
        return Ok((display, false, json!({})));
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("读取客户端设置失败（{}）：{e}", path.display()))?;
    let value: Value =
        serde_json::from_str(&text).map_err(|e| parse_failed(&path, &e.to_string()))?;
    Ok((display, true, value))
}

// ---------------------------------------------------------------------------
// 读 / 写实现（可注入 backup_root，便于单测完全隔离真实备份目录）
// ---------------------------------------------------------------------------

/// patch 的**可测**实现：`backup_root` 是备份根目录（生产传 `config::backup_dir()`）。
fn patch_at(
    path: &Path,
    patch: &Value,
    slot: &ClientSlot,
    backup_root: &Path,
) -> Result<Value, String> {
    if !path.is_file() {
        return Err(format!(
            "【{}】未安装（找不到 {}），无法编辑",
            slot.display_key,
            path.display()
        ));
    }
    let patch_obj = patch
        .as_object()
        .ok_or_else(|| "patch 必须是 { \"<点号路径>\": <新值> } 形式".to_string())?;

    let original = std::fs::read_to_string(path)
        .map_err(|e| format!("读取客户端设置失败（{}）：{e}", path.display()))?;
    let tree = parse_json(&original).map_err(|e| parse_failed(path, &e))?;
    if !matches!(tree.kind, NodeKind::Object(_)) {
        return Err(parse_failed(path, "顶层不是 JSON 对象"));
    }
    let fp = fingerprint_of_text(&original);

    let mut edits: Vec<Edit> = Vec::new();
    let mut seq = 0usize;
    for (key, value) in patch_obj {
        validate_patch_key(key)?;
        let segments: Vec<&str> = key.split('.').collect();
        if let Some(edit) = locate_edit(&tree, &original, &segments, value, &fp, 0)? {
            edits.push(Edit { seq, ..edit });
            seq += 1;
        }
    }
    let keys: Vec<String> = patch_obj.keys().cloned().collect();

    let (next, changed) = apply_edits(&original, edits);
    if !changed {
        // 幂等：本次无实际改写 ⇒ **不备份、不写盘**。
        return Ok(result_view(
            slot,
            path,
            false,
            Value::Null,
            keys,
            original.clone(),
            serde_json::from_str(&original).unwrap_or(Value::Null),
        ));
    }

    // 自读一遍校验 JSON 合法：校验不过**不替换**。
    if let Err(e) = serde_json::from_str::<Value>(&next) {
        return Err(write_failed(
            path,
            &format!("改写后不是合法 JSON，已放弃写入：{e}"),
        ));
    }

    let backup = backup_settings(slot.key, &original, backup_root);
    atomic_write(path, &next).map_err(|e| write_failed(path, &e.to_string()))?;

    Ok(result_view(
        slot,
        path,
        true,
        backup
            .map(|p| Value::String(p.to_string_lossy().to_string()))
            .unwrap_or(Value::Null),
        keys,
        next.clone(),
        serde_json::from_str(&next).unwrap_or(Value::Null),
    ))
}

/// 恢复的**可测**实现。
fn restore_at(
    path: &Path,
    slot: &ClientSlot,
    backup_id: &str,
    backup_root: &Path,
) -> Result<Value, String> {
    let src = backup_root
        .join("client-settings")
        .join(slot.key)
        .join(backup_id)
        .join("settings.json");
    if !src.is_file() {
        return Err(format!("备份点不存在：{}", src.display()));
    }
    let content = std::fs::read_to_string(&src)
        .map_err(|e| format!("读取备份失败（{}）：{e}", src.display()))?;
    if let Err(e) = serde_json::from_str::<Value>(&content) {
        return Err(parse_failed(&src, &format!("备份内容不是合法 JSON：{e}")));
    }

    let current = std::fs::read_to_string(path).ok();
    let changed = current.as_deref() != Some(content.as_str());
    let mut backup = Value::Null;
    if changed {
        // 「恢复前对当前文件再备份一次」——当前文件不存在则无原文可留。
        if let Some(cur) = current.as_deref() {
            if let Some(dest) = backup_settings(slot.key, cur, backup_root) {
                backup = Value::String(dest.to_string_lossy().to_string());
            }
        }
        atomic_write(path, &content).map_err(|e| write_failed(path, &e.to_string()))?;
    }

    let mut out = result_view(
        slot,
        path,
        changed,
        backup,
        Vec::new(),
        content.clone(),
        serde_json::from_str(&content).unwrap_or(Value::Null),
    );
    out["restoredFrom"] = Value::String(backup_id.to_string());
    Ok(out)
}

/// 两条通道共用的结果形状（**两宿主逐字相同**）。
fn result_view(
    slot: &ClientSlot,
    path: &Path,
    changed: bool,
    backup: Value,
    keys: Vec<String>,
    text: String,
    values: Value,
) -> Value {
    json!({
        "slot": slot.key,
        "displayKey": slot.display_key,
        "product": slot.product,
        "path": path.to_string_lossy(),
        "installed": true,
        "changed": changed,
        "backup": backup,
        "keys": keys,
        "text": text,
        "values": values,
    })
}

/// 该槽位的备份点列表（可测实现）。
fn list_backups_in(slot: &ClientSlot, backup_root: &Path) -> Value {
    let dir = backup_root.join("client-settings").join(slot.key);
    let mut items: Vec<Value> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if !entry_path.is_dir() {
                continue;
            }
            let file = entry_path.join("settings.json");
            if !file.is_file() {
                continue;
            }
            items.push(json!({
                "id": entry.file_name().to_string_lossy(),
                "path": file.to_string_lossy(),
                "size": entry.metadata().map(|m| m.len()).unwrap_or(0),
            }));
        }
    }
    // id 是 `utc_iso` 时间戳（可含 `-N` 去重后缀）⇒ 字典序降序即时间降序。
    items.sort_by(|a, b| {
        b["id"]
            .as_str()
            .unwrap_or("")
            .cmp(a["id"].as_str().unwrap_or(""))
    });
    json!({
        "slot": slot.key,
        "displayKey": slot.display_key,
        "dir": dir.to_string_lossy(),
        "backups": items,
    })
}

/// 备份改前原文到 `<backup_root>/client-settings/<slot>/<utc_iso>/settings.json`。
///
/// 备份失败**不阻断**写入（返回 `None`）。同一秒内产生多份备份时追加 `-N` 去重后缀
/// （否则第二次会把第一次**覆盖**掉 —— 恢复流程「恢复前再备份」几乎必然同秒发生）。
fn backup_settings(slot_key: &str, content: &str, backup_root: &Path) -> Option<PathBuf> {
    let base = backup_root.join("client-settings").join(slot_key);
    let stamp = utc_iso();
    let mut dir = base.join(&stamp);
    let mut n = 2u32;
    while dir.exists() {
        dir = base.join(format!("{stamp}-{n}"));
        n += 1;
        if n > 999 {
            return None;
        }
    }
    std::fs::create_dir_all(&dir).ok()?;
    let dest = dir.join("settings.json");
    atomic_write(&dest, content).ok()?;
    Some(dest)
}

// ---------------------------------------------------------------------------
// 差异比对
// ---------------------------------------------------------------------------

/// 两个已解析的 JSON 值比对：三类 + 计数。
///
/// 只有**对象**递归下钻；数组 / 标量当叶子整值比对（顺序敏感，符合 JSON 语义）。
///
/// ⚠️ **仅测试使用**（`#[cfg(test)]`）：生产路径一律经 `diff_values` → `diff_flat`
/// （后者要按槽位的 product 做凭据剥离等加工）。这个薄壳只是给单测一个
/// 「不经槽位、直接比两个 JSON」的入口 —— 若不加 `cfg(test)`，它会以
/// `dead_code` 形式进产物（本仓 `cargo build` 的警告即由此而来）。
#[cfg(test)]
fn diff_maps(a: &Value, b: &Value) -> Value {
    diff_flat(&flatten(a), &flatten(b))
}

/// 两个**已拍平**结果（`点号路径 → 叶子值`）的三类比对 + 计数。
fn diff_flat(a_flat: &BTreeMap<String, Value>, b_flat: &BTreeMap<String, Value>) -> Value {
    let mut only_a = Vec::new();
    let mut only_b = Vec::new();
    let mut different = Vec::new();
    let mut same = 0usize;

    for (path, value) in a_flat {
        match b_flat.get(path) {
            None => only_a.push(json!({ "path": path, "value": value })),
            Some(other) if other == value => same += 1,
            Some(other) => different.push(json!({ "path": path, "a": value, "b": other })),
        }
    }
    for (path, value) in b_flat {
        if !a_flat.contains_key(path) {
            only_b.push(json!({ "path": path, "value": value }));
        }
    }

    json!({
        "onlyA": only_a,
        "onlyB": only_b,
        "different": different,
        "counts": {
            "onlyA": a_flat.len() - same - different.len(),
            "onlyB": only_b.len(),
            "different": different.len(),
            "same": same,
        },
    })
}

/// 把嵌套对象拍平成 `点号路径 → 叶子值`。
fn flatten(value: &Value) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    flatten_into(value, "", &mut out);
    out
}

fn flatten_into(value: &Value, prefix: &str, out: &mut BTreeMap<String, Value>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten_into(child, &path, out);
            }
        }
        other => {
            if !prefix.is_empty() {
                out.insert(prefix.to_string(), other.clone());
            }
        }
    }
}

/// 跨产品比对时，把代理字段从两侧拍平结果里**摘出**并归并成一组。
///
/// 直接在**拍平层**按 [`PROXY_FIELDS`] 声明的路径摘除，而不是按点号路径逐层下钻 ——
/// 点号路径有**歧义**：Trae 的键本身就叫 `trae.network.proxy.effectiveMode`
/// （一个字面键含点），WorkBuddy 则是嵌套的 `http` → `proxy`；两者拍平后都是
/// `http.proxy` / `trae.network.proxy.effectiveMode`，**按拍平路径摘除对两种都成立**。
///
/// - 只在**两侧都声明**了代理字段时才归并（单侧声明无从归并，按普通三类处理）；
/// - `same` 按**有效值集合**（`present: true` 的那些值）判定，不比较字段结构
///   （两侧字段结构本就不同）。
/// - 摘出的路径**不再**参与 `onlyA` / `onlyB` / `different`，避免同一件事被拆成两条。
fn merge_proxy_fields(
    a_flat: &mut BTreeMap<String, Value>,
    a_key: &str,
    b_flat: &mut BTreeMap<String, Value>,
    b_key: &str,
) -> Vec<Value> {
    let a_paths = proxy_paths_of(a_key);
    let b_paths = proxy_paths_of(b_key);
    if a_paths.is_empty() || b_paths.is_empty() {
        return Vec::new();
    }
    let fields = |flat: &BTreeMap<String, Value>, paths: &[&'static str]| -> Vec<Value> {
        paths
            .iter()
            .map(|path| {
                let value = flat.get(*path);
                json!({
                    "path": path,
                    "present": value.is_some(),
                    "value": value.cloned().unwrap_or(Value::Null),
                })
            })
            .collect()
    };
    let a_fields = fields(a_flat, &a_paths);
    let b_fields = fields(b_flat, &b_paths);
    let values_of = |list: &[Value]| -> Vec<Value> {
        list.iter()
            .filter(|field| field["present"] == Value::Bool(true))
            .map(|field| field["value"].clone())
            .collect()
    };
    let same = values_of(&a_fields) == values_of(&b_fields);
    let group = json!({
        "semantic": "proxy",
        "a": a_fields,
        "b": b_fields,
        "same": same,
    });

    // 摘掉代理字段：它们是「同一件事」，不该再各报一条 onlyA / onlyB。
    for path in &a_paths {
        a_flat.remove(*path);
    }
    for path in &b_paths {
        b_flat.remove(*path);
    }
    vec![group]
}

// ---------------------------------------------------------------------------
// 格式指纹
// ---------------------------------------------------------------------------

/// 原文件的排版指纹。`indent = None` 表示**全文无换行**（压缩式）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatFingerprint {
    /// 首个成员行的前导空白（`"  "` / `"\t"` / `""`）；无换行 ⇒ `None`。
    pub indent: Option<String>,
    /// 文中首个 `\r\n` 或 `\n`（都没有时用 `\n` 占位，压缩式下不会被用到）。
    pub newline: String,
}

/// 检测排版指纹（**一律靠检测，不写死**）。
fn fingerprint_of_text(text: &str) -> FormatFingerprint {
    let newline = if text.contains("\r\n") {
        "\r\n".to_string()
    } else {
        "\n".to_string()
    };
    let indent = text.find('\n').map(|idx| {
        text[idx + 1..]
            .chars()
            .take_while(|c| *c == ' ' || *c == '\t')
            .collect::<String>()
    });
    FormatFingerprint { indent, newline }
}

// ---------------------------------------------------------------------------
// 渲染（按原风格）
// ---------------------------------------------------------------------------

/// 按指纹渲染一个值；`depth` 是**该值自身**的深度（根对象为 0）。
fn render_value(value: &Value, fp: &FormatFingerprint, depth: usize) -> String {
    match &fp.indent {
        None => serde_json::to_string(value).unwrap_or_else(|_| "null".to_string()),
        Some(indent) => {
            let mut out = String::new();
            write_value(value, indent, &fp.newline, depth, &mut out);
            out
        }
    }
}

fn write_value(value: &Value, indent: &str, nl: &str, depth: usize, out: &mut String) {
    match value {
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            let pad_child = indent.repeat(depth + 1);
            let pad_parent = indent.repeat(depth);
            for (index, (key, child)) in map.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(nl);
                out.push_str(&pad_child);
                out.push_str(&json_key(key));
                out.push_str(": ");
                write_value(child, indent, nl, depth + 1, out);
            }
            out.push_str(nl);
            out.push_str(&pad_parent);
            out.push('}');
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            let pad_child = indent.repeat(depth + 1);
            let pad_parent = indent.repeat(depth);
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(nl);
                out.push_str(&pad_child);
                write_value(item, indent, nl, depth + 1, out);
            }
            out.push_str(nl);
            out.push_str(&pad_parent);
            out.push(']');
        }
        other => out.push_str(&serde_json::to_string(other).unwrap_or_else(|_| "null".to_string())),
    }
}

/// JSON 对象的键字面量（含转义与引号）。
fn json_key(key: &str) -> String {
    serde_json::to_string(&Value::String(key.to_string())).unwrap_or_else(|_| format!("\"{key}\""))
}

/// 把 `segments` 渲染成嵌套对象文本（`["b","c"]` + 值 `v` ⇒ `{"b":{"c":v}}` 的文本）。
///
/// 每个段都作为**对象成员键**渲染，最后一个段包住 `value`（**不得**把末段丢掉）。
fn build_nested_value(
    segments: &[&str],
    value: &Value,
    fp: &FormatFingerprint,
    depth: usize,
) -> String {
    let key = json_key(segments[0]);
    let inner = if segments.len() == 1 {
        render_value(value, fp, depth + 1)
    } else {
        build_nested_value(&segments[1..], value, fp, depth + 1)
    };
    match &fp.indent {
        None => format!("{{{key}:{inner}}}"),
        Some(indent) => format!(
            "{{{nl}{pad_child}{key}: {inner}{nl}{pad_parent}}}",
            nl = fp.newline,
            pad_child = indent.repeat(depth + 1),
            pad_parent = indent.repeat(depth),
        ),
    }
}

/// 在对象末尾追加一个成员的编辑（空对象插在 `{` 之后）。
fn render_member_insert(
    obj: &Node,
    key: &str,
    value_text: &str,
    fp: &FormatFingerprint,
    depth: usize,
) -> Edit {
    let key_json = json_key(key);
    let empty = matches!(&obj.kind, NodeKind::Object(members) if members.is_empty());
    if empty {
        let text = match &fp.indent {
            None => format!("{key_json}:{value_text}"),
            Some(indent) => format!(
                "{nl}{pad_child}{key_json}: {value_text}{nl}{pad_parent}",
                nl = fp.newline,
                pad_child = indent.repeat(depth + 1),
                pad_parent = indent.repeat(depth),
            ),
        };
        Edit {
            start: obj.start + 1,
            end: obj.start + 1,
            text,
            seq: 0,
        }
    } else {
        let text = match &fp.indent {
            None => format!(",{key_json}:{value_text}"),
            Some(indent) => format!(
                ",{nl}{pad_child}{key_json}: {value_text}",
                nl = fp.newline,
                pad_child = indent.repeat(depth + 1),
            ),
        };
        Edit {
            start: obj.end - 1,
            end: obj.end - 1,
            text,
            seq: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// 编辑定位与应用
// ---------------------------------------------------------------------------

/// 一处文本编辑：替换 `[start, end)`；`start == end` 表示纯插入。
#[derive(Debug, Clone)]
struct Edit {
    start: usize,
    end: usize,
    text: String,
    /// 生成序号：同一偏移上的多个插入按它**倒序**应用，保证最终文本顺序与生成顺序一致。
    seq: usize,
}

/// 定位一个 patch 键对应的编辑；值已逐字节相同 ⇒ `None`（幂等）。
fn locate_edit(
    node: &Node,
    original: &str,
    segments: &[&str],
    value: &Value,
    fp: &FormatFingerprint,
    depth: usize,
) -> Result<Option<Edit>, String> {
    let NodeKind::Object(members) = &node.kind else {
        return Err(format!(
            "路径「{}」的中间节点不是 JSON 对象，无法定位",
            segments.join(".")
        ));
    };
    let seg = segments[0];
    match members.iter().find(|member| member.key == seg) {
        Some(member) if segments.len() == 1 => {
            let replacement = render_value(value, fp, depth + 1);
            if original.get(member.value.start..member.value.end) == Some(replacement.as_str()) {
                return Ok(None);
            }
            Ok(Some(Edit {
                start: member.value.start,
                end: member.value.end,
                text: replacement,
                seq: 0,
            }))
        }
        Some(member) => locate_edit(
            &member.value,
            original,
            &segments[1..],
            value,
            fp,
            depth + 1,
        ),
        None => {
            // 中间键缺失 ⇒ 把**剩余路径**（不含当前段）渲染成嵌套对象，插到当前对象末尾。
            // 单段时值直接就是 `value`（不可走 `build_nested_value`，否则会多包一层同名键）。
            let nested = if segments.len() == 1 {
                render_value(value, fp, depth + 1)
            } else {
                build_nested_value(&segments[1..], value, fp, depth + 1)
            };
            Ok(Some(render_member_insert(node, seg, &nested, fp, depth)))
        }
    }
}

/// 从高偏移到低偏移应用编辑（低偏移处的内容不会被先前的编辑移动）。
fn apply_edits(original: &str, mut edits: Vec<Edit>) -> (String, bool) {
    edits.sort_by(|a, b| b.start.cmp(&a.start).then(b.seq.cmp(&a.seq)));
    let mut text = original.to_string();
    let mut changed = false;
    for edit in &edits {
        if edit.start == edit.end {
            text.insert_str(edit.start, &edit.text);
            changed = true;
        } else if original.get(edit.start..edit.end) != Some(edit.text.as_str()) {
            text.replace_range(edit.start..edit.end, &edit.text);
            changed = true;
        }
    }
    (text, changed)
}

// ---------------------------------------------------------------------------
// 只读 JSON 扫描器（带字节跨度）
// ---------------------------------------------------------------------------

/// 一个 JSON 节点的**字节跨度**与结构。
#[derive(Debug, Clone)]
struct Node {
    /// 起始字节（含）。
    start: usize,
    /// 结束字节（不含）。
    end: usize,
    kind: NodeKind,
}

#[derive(Debug, Clone)]
enum NodeKind {
    Object(Vec<Member>),
    /// 数组元素：**保留结构**以便扫描器完整校验（元素本身不参与编辑定位，
    /// patch 只按对象键下钻），故字段只在解析期被写入、读取期不消费。
    Array(#[allow(dead_code)] Vec<Node>),
    /// 字符串 / 数字 / 布尔 / null（值本身不需要，替换时整段重渲染）。
    Scalar,
}

#[derive(Debug, Clone)]
struct Member {
    /// **解码后**的键（`\uXXXX` 等已还原），用于点号路径定位。
    key: String,
    value: Node,
}

/// 解析整段文本为节点树；**尾部有多余内容即失败**。
fn parse_json(text: &str) -> Result<Node, String> {
    let mut scanner = Scanner::new(text);
    scanner.skip_ws();
    let node = scanner.parse_value()?;
    scanner.skip_ws();
    if scanner.pos != scanner.bytes.len() {
        return Err(format!("第 {} 字节起有 JSON 之外的多余内容", scanner.pos));
    }
    Ok(node)
}

/// 顶层键的**文件顺序**（`serde_json::Map` 是 `BTreeMap`，拿不到原顺序）。
fn top_level_order(tree: &Node) -> Vec<String> {
    match &tree.kind {
        NodeKind::Object(members) => members.iter().map(|m| m.key.clone()).collect(),
        _ => Vec::new(),
    }
}

struct Scanner<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Scanner<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            bytes: text.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn parse_value(&mut self) -> Result<Node, String> {
        self.skip_ws();
        let start = self.pos;
        match self.peek() {
            Some(b'{') => {
                self.pos += 1;
                let members = self.parse_object_members()?;
                Ok(Node {
                    start,
                    end: self.pos,
                    kind: NodeKind::Object(members),
                })
            }
            Some(b'[') => {
                self.pos += 1;
                let items = self.parse_array_items()?;
                Ok(Node {
                    start,
                    end: self.pos,
                    kind: NodeKind::Array(items),
                })
            }
            Some(b'"') => {
                self.parse_string()?;
                Ok(Node {
                    start,
                    end: self.pos,
                    kind: NodeKind::Scalar,
                })
            }
            Some(_) => {
                self.parse_primitive()?;
                Ok(Node {
                    start,
                    end: self.pos,
                    kind: NodeKind::Scalar,
                })
            }
            None => Err("意外的结尾：期望一个值".to_string()),
        }
    }

    fn parse_object_members(&mut self) -> Result<Vec<Member>, String> {
        let mut members = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(members);
        }
        loop {
            self.skip_ws();
            let key = self.parse_string()?;
            self.skip_ws();
            if self.peek() != Some(b':') {
                return Err(format!("第 {} 字节处对象成员缺少 ':'", self.pos));
            }
            self.pos += 1;
            let value = self.parse_value()?;
            members.push(Member { key, value });
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(format!("第 {} 字节处对象成员分隔符错误", self.pos)),
            }
        }
        Ok(members)
    }

    fn parse_array_items(&mut self) -> Result<Vec<Node>, String> {
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(items);
        }
        loop {
            let item = self.parse_value()?;
            items.push(item);
            self.skip_ws();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    break;
                }
                _ => return Err(format!("第 {} 字节处数组元素分隔符错误", self.pos)),
            }
        }
        Ok(items)
    }

    fn parse_string(&mut self) -> Result<String, String> {
        if self.peek() != Some(b'"') {
            return Err(format!("第 {} 字节处期望字符串键", self.pos));
        }
        self.pos += 1;
        let mut out = String::new();
        loop {
            let byte = *self
                .bytes
                .get(self.pos)
                .ok_or_else(|| "字符串未闭合".to_string())?;
            match byte {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    self.pos += 1;
                    let esc = *self
                        .bytes
                        .get(self.pos)
                        .ok_or_else(|| "转义序列未闭合".to_string())?;
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.parse_unicode_escape()?),
                        _ => {
                            return Err(format!(
                                "第 {} 字节处非法转义 \\{}",
                                self.pos - 1,
                                esc as char
                            ))
                        }
                    }
                }
                _ => {
                    let start = self.pos;
                    while let Some(b) = self.peek() {
                        if b == b'"' || b == b'\\' {
                            break;
                        }
                        self.pos += 1;
                    }
                    let chunk = std::str::from_utf8(&self.bytes[start..self.pos])
                        .map_err(|_| format!("第 {start} 字节处字符串不是合法 UTF-8"))?;
                    out.push_str(chunk);
                }
            }
        }
    }

    /// 解析 `\uXXXX`（含 UTF-16 代理对）。调用时 `self.pos` 已指向 4 位十六进制。
    fn parse_unicode_escape(&mut self) -> Result<char, String> {
        let high = self.read_hex4()?;
        if (0xD800..0xDC00).contains(&high) {
            // 高代理 ⇒ 必须紧跟低代理 `\uXXXX`，否则按替换字符处理（**不报错**：不阻断读取）。
            if self.peek() == Some(b'\\') && self.bytes.get(self.pos + 1) == Some(&b'u') {
                self.pos += 2;
                let low = self.read_hex4()?;
                let combined =
                    0x1_0000 + ((high as u32 - 0xD800) << 10) + (low as u32).saturating_sub(0xDC00);
                return Ok(char::from_u32(combined).unwrap_or('\u{FFFD}'));
            }
            return Ok('\u{FFFD}');
        }
        Ok(char::from_u32(high as u32).unwrap_or('\u{FFFD}'))
    }

    fn read_hex4(&mut self) -> Result<u16, String> {
        let end = self.pos + 4;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| "\\u 转义不完整".to_string())?;
        let text = std::str::from_utf8(slice).map_err(|_| "\\u 转义不是合法 UTF-8".to_string())?;
        let code =
            u16::from_str_radix(text, 16).map_err(|_| format!("\\u{text} 不是合法十六进制"))?;
        self.pos = end;
        Ok(code)
    }

    /// 数字 / `true` / `false` / `null`：读到分隔符为止（合法性交给 `serde_json` 复核）。
    fn parse_primitive(&mut self) -> Result<(), String> {
        let start = self.pos;
        while let Some(b) = self.peek() {
            if b == b','
                || b == b'}'
                || b == b']'
                || b == b' '
                || b == b'\t'
                || b == b'\n'
                || b == b'\r'
            {
                break;
            }
            self.pos += 1;
        }
        if self.pos == start {
            return Err(format!("第 {start} 字节处期望一个值"));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 路径与错误
// ---------------------------------------------------------------------------

/// patch 的点号路径校验：**路径穿越防护**（`..` / 斜杠 / 空段一律拒绝）。
fn validate_patch_key(key: &str) -> Result<(), String> {
    if key.trim().is_empty() {
        return Err("patch 的键不能为空".to_string());
    }
    if key.contains("..") {
        return Err(format!("patch 的键含非法路径片段（`..`）：{key}"));
    }
    if key.starts_with('.') || key.ends_with('.') {
        return Err(format!("patch 的键不得以 `.` 开头或结尾：{key}"));
    }
    for segment in key.split('.') {
        if segment.is_empty() {
            return Err(format!("patch 的键含空路径段：{key}"));
        }
        if segment.contains('/') || segment.contains('\\') || segment.contains('\0') {
            return Err(format!("patch 的键含非法字符（路径分隔符 / NUL）：{key}"));
        }
    }
    Ok(())
}

/// 备份点 id 校验（目录名，**不得**含路径分隔符或 `..`）。
fn validate_backup_id(backup_id: &str) -> Result<(), String> {
    if backup_id.trim().is_empty() {
        return Err("backupId 不能为空".to_string());
    }
    if backup_id.contains("..")
        || backup_id.contains('/')
        || backup_id.contains('\\')
        || backup_id.contains('\0')
    {
        return Err(format!("backupId 含非法字符：{backup_id}"));
    }
    Ok(())
}

fn unknown_slot(slot_key: &str) -> String {
    format!("未知的客户端槽位：{slot_key}")
}

fn locate_failed(slot: &ClientSlot, slot_key: &str) -> String {
    format!(
        "无法定位【{slot_key}】的 userData 目录（取不到 %APPDATA% / 用户主目录）；槽位展示键 {}",
        slot.display_key
    )
}

fn process_running(slot: &ClientSlot) -> String {
    AppError::new(
        ErrorCode::ClientSettingsProcessRunning,
        format!(
            "【{}】客户端正在运行，已拒绝写入其 settings.json（运行中写入会被内存态覆盖，请先退出客户端）",
            slot.display_key
        ),
    )
    .with("slot", slot.display_key)
    .to_wire()
}

fn parse_failed(path: &Path, reason: &str) -> String {
    AppError::new(
        ErrorCode::ClientSettingsParseFailed,
        format!(
            "客户端 settings.json 解析失败（{}）：{reason}",
            path.display()
        ),
    )
    .with("path", path.to_string_lossy().to_string())
    .to_wire()
}

fn write_failed(path: &Path, reason: &str) -> String {
    AppError::new(
        ErrorCode::ClientSettingsWriteFailed,
        format!(
            "客户端 settings.json 写入失败（{}）：{reason}",
            path.display()
        ),
    )
    .with("path", path.to_string_lossy().to_string())
    .to_wire()
}

// ---------------------------------------------------------------------------
// 单测
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::error_code::decode_wire;

    /// 未安装判定是**纯函数**（文件存在性），因此可完全脱离真实客户端单测。
    fn installed_at(path: &Path) -> bool {
        path.is_file()
    }

    /// 极简临时目录（**刻意不引入 `tempfile` 依赖**）：`Drop` 时递归删除。
    struct TempDir(PathBuf);

    impl TempDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tempdir() -> TempDir {
        let dir = std::env::temp_dir().join(format!(
            "buddy-switch-client-settings-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        TempDir(dir)
    }

    fn write_temp(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).expect("写临时文件");
        path
    }

    fn backup_root(dir: &Path) -> PathBuf {
        dir.join("backups")
    }

    fn changed_line_count(before: &str, after: &str) -> usize {
        before
            .lines()
            .zip(after.lines())
            .filter(|(a, b)| a != b)
            .count()
    }

    // ---- 槽位表 ----------------------------------------------------------

    #[test]
    fn 槽位表只含在用槽位且键互不重复() {
        // ★ issue #28：Trae 的 4 个槽位收敛为 2 个（每区域一个合并版客户端），
        //   整表 6 → 4。退役槽位**不得**留在表里（它们指向已卸载的客户端）。
        assert_eq!(CLIENT_SLOTS.len(), 4, "槽位数必须是 4（WB 2 + Trae 2）");
        let mut keys: Vec<&str> = CLIENT_SLOTS.iter().map(|s| s.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), 4, "槽位 key 必须互不重复");

        let mut displays: Vec<&str> = CLIENT_SLOTS.iter().map(|s| s.display_key).collect();
        displays.sort_unstable();
        displays.dedup();
        assert_eq!(displays.len(), 4, "槽位 display_key 必须互不重复");

        // 预期 key 集合逐字钉住（改名即断前端传参与备份目录）。
        let expected = [
            "workbuddy-cn",
            "workbuddy-global",
            "trae-code-cn",
            "trae-code-global",
        ];
        // ★ 退役的两个 key 必须**查不到** —— 它们仍可能出现在用户备份目录名里，
        //   留着就会让「恢复备份」把配置写进一个已卸载的客户端。
        for retired in ["trae-work-cn", "trae-work-global"] {
            assert!(
                slot_by_key(retired).is_none(),
                "退役槽位 {retired} 仍可查 —— 备份/恢复会把配置写进已卸载的客户端"
            );
        }
        for key in expected {
            assert!(slot_by_key(key).is_some(), "缺槽位 {key}");
        }
        assert!(slot_by_key("nope").is_none());
    }

    // ---- hook 安装目标槽位（CLI 系）---------------------------------------

    /// ★ hook 目标槽位：key 唯一、与代理槽位**不撞**、且 `slot_by_key` 都能查到。
    ///
    /// 可证伪性：把某个 hook 槽位的 key 改成 `workbuddy-cn` ⇒ 变红
    /// （`slot_by_key` 会先命中代理表，hook 会被装到**错的客户端**上）。
    #[test]
    fn hook_file_slots_are_disjoint_from_proxy_slots_and_lookupable() {
        assert!(
            !HOOK_FILE_SLOTS.is_empty(),
            "hook 目标表不得为空（否则安装面是空转）"
        );

        let mut keys: Vec<&str> = HOOK_FILE_SLOTS.iter().map(|s| s.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), HOOK_FILE_SLOTS.len(), "hook 目标 key 有重复");

        for slot in HOOK_FILE_SLOTS {
            assert!(
                CLIENT_SLOTS.iter().all(|s| s.key != slot.key),
                "hook 目标 {} 与代理槽位重名 —— 代理页会把它当代理槽位渲染",
                slot.key
            );
            assert_eq!(
                slot_by_key(slot.key).map(|s| s.key),
                Some(slot.key),
                "hook 目标 {} 必须能被 slot_by_key 查到（否则读写会报「未知槽位」）",
                slot.key
            );
            assert_eq!(slot.product, "agent", "hook 目标的产品分组必须是 agent");
        }
    }

    /// ★ **进程守卫不是空转**：每个 hook 目标都要能算出「在不在跑」。
    ///
    /// 可证伪性（两档各一条）：
    /// - `HomeFile.image` 留空 ⇒ 变红（`process::is_image_running` 会保守返回 true，
    ///   守卫永远拒绝写入 —— 用户看到「请先退出客户端」，而客户端根本没开，是**假护栏**）；
    /// - `TraeHooks` 的目录名留空 ⇒ 路径断言变红。
    #[test]
    fn hook_file_slots_declare_enough_to_guard_writes() {
        for slot in HOOK_FILE_SLOTS {
            match slot.kind {
                SlotKind::HomeFile(spec) => {
                    assert!(!spec.image.trim().is_empty(), "{} 缺映像名", slot.key);
                    assert!(!spec.dir.trim().is_empty(), "{} 缺目录名", slot.key);
                    assert!(!spec.file.trim().is_empty(), "{} 缺文件名", slot.key);
                }
                SlotKind::TraeHooks(region) => {
                    assert!(
                        !trae_hooks_dir(region).trim().is_empty(),
                        "{} 缺产品数据目录名",
                        slot.key
                    );
                    // 该区域必须至少有一个变体可供判进程（否则守卫恒为 false ⇒ 空转）。
                    assert!(
                        all_specs()
                            .iter()
                            .any(|spec| spec.variant.region() == region),
                        "{} 所属区域没有任何变体 —— 进程守卫会恒为「没在跑」",
                        slot.key
                    );
                }
                other => panic!("hook 目标 {} 的 kind 不受支持：{other:?}", slot.key),
            }
        }
    }

    /// hook 目标的路径：CLI 系 = `家目录/<dir>/<file>`；Trae = `家目录/<产品数据目录>/hooks.json`。
    ///
    /// ⚠️ 这里的 `home` 必须走 [`home_dir`]（与实现同一函数）—— 换成 `dirs::home_dir()`
    /// 会在设了 `BUDDY_SWITCH_HOME` 的测试里拿**真实**家目录，于是断言在
    /// 「实现也走 dirs」时假绿、在「实现走 config」时假红。
    ///
    /// 可证伪性：把 `TraeHooks` 分支改成拼 `%APPDATA%\<userDataDirName>`（`SlotKind::Trae` 那一套）
    /// ⇒ Trae 两条路径断言变红 —— 那正是「hooks 文件与 settings.json 不同源」这条事实。
    #[test]
    fn hook_file_paths_live_under_the_home_directory() {
        // 本用例读进程级 HOME / 网关配置：必须与「改 HOME 的用例」串行，
        // 否则并行跑时会读到别人临时改过的家目录（曾因此 flaky）。
        let _env = crate::modules::config::env_lock();
        let home = home_dir();
        for slot in HOOK_FILE_SLOTS {
            let path = slot.settings_path().expect("家目录可解析 ⇒ 路径必须可解析");
            assert!(
                path.starts_with(&home),
                "hook 目标必须在家目录下：{}",
                path.display()
            );
            let expected = match slot.kind {
                SlotKind::HomeFile(spec) => home.join(spec.dir).join(spec.file),
                SlotKind::TraeHooks(region) => home.join(trae_hooks_dir(region)).join("hooks.json"),
                other => panic!("不受支持的 kind：{other:?}"),
            };
            assert_eq!(path, expected, "{} 的路径不对", slot.key);
        }
        // ★ 两个 Trae 槽位必须是**两个不同区域**的文件（否则国际版会写进国内版的 hooks）。
        let cn = slot_by_key("trae-hooks-cn").expect("缺 trae-hooks-cn");
        let global = slot_by_key("trae-hooks-global").expect("缺 trae-hooks-global");
        assert_ne!(
            cn.settings_path(),
            global.settings_path(),
            "国内 / 国际的 hooks 文件必须分开"
        );
        // ⚠️ 断言比较**完整期望路径**，不要用 `Path::ends_with("trae-cn/hooks.json")`：
        //    它按**路径分量**比较，而真实目录名是 `.trae-cn`（**带前导点**）⇒
        //    写成不带点会恒假（假红），而且这条假红会掩盖真正的漂移。
        assert_eq!(
            cn.settings_path().unwrap(),
            home.join(".trae-cn").join("hooks.json"),
            "国内版 hooks 文件必须落在 `~/.trae-cn` 下"
        );
        assert_eq!(
            global.settings_path().unwrap(),
            home.join(".trae").join("hooks.json"),
            "国际版 hooks 文件必须落在 `~/.trae` 下"
        );
    }

    /// 未安装者 `installed: false` 且**仍在列表**（不因探测失败而消失）。
    #[test]
    fn 未安装槽位仍在列表且installed为false() {
        let out = list_slots();
        let slots = out["slots"].as_array().expect("slots 必须是数组");
        assert_eq!(slots.len(), 4, "未安装者不得从列表消失（且退役槽位已移除）");
        for slot in slots {
            assert!(slot["key"].is_string());
            assert!(slot["displayKey"].is_string());
            assert!(
                slot["installed"].is_boolean(),
                "installed 必须是布尔（探测失败也不得省略）"
            );
            assert!(slot["path"].is_string() || slot["path"].is_null());
        }

        // 存在性判定本身：临时目录里造一个假文件，`installed_at` 必须为 true。
        let tmp = tempdir();
        let fake = write_temp(tmp.path(), "settings.json", "{}");
        assert!(installed_at(&fake));
        assert!(!installed_at(&tmp.path().join("missing.json")));
    }

    /// 变体 ↔ (区域, 程序) 必须一致 —— **合并版是刻意的多对一**（issue #20）。
    #[test]
    fn 变体与区域程序双向一致() {
        // ★★ issue #28 收敛后：映射是「**区域 → 该区域的合并版变体**」的纯函数，
        //   程序轴只剩一个取值 ⇒ 往返一致性退化成「区域自洽」。
        //   反例（改坏会红）：让两个区域落到同一个变体 ⇒ 两套账号体系的登录态
        //   会写进同一个快照目录（不报错，只是数据串了）。
        for region in TraeRegion::all() {
            for program in TraeProgram::all() {
                let variant = variant_for_program(region, program);
                assert_eq!(
                    program_of_variant(variant),
                    (region, program),
                    "{region:?}/{program:?} 往返不逐字一致"
                );
            }
        }
        for variant in [
            TraeVariant::TraeWork,
            TraeVariant::Trae,
            TraeVariant::TraeMerged,
            TraeVariant::Global,
            TraeVariant::GlobalTraeCode,
            TraeVariant::GlobalTraeMerged,
        ] {
            let (region, program) = program_of_variant(variant);
            // 退役变体往返后落到**本区域的合并版**（收敛后它们不再可切换）。
            assert_eq!(
                variant_for_program(region, program).region(),
                variant.region()
            );
            // 区域必须与 `variant.region()` 同源。
            assert_eq!(region, variant.region(), "{variant:?} 的区域漂了");
        }
        // 两个区域 → 两个互不相同的合并版变体。
        let mut seen: Vec<TraeVariant> = Vec::new();
        for region in TraeRegion::all() {
            let variant = variant_for_program(region, TraeProgram::Trae);
            assert!(
                !seen.contains(&variant),
                "{variant:?} 被两个区域共用 —— 两套账号体系的快照目录会重合"
            );
            seen.push(variant);
        }
        assert_eq!(seen.len(), 2, "两个区域必须各映射到一个独立变体");
    }

    // ---- 格式指纹 --------------------------------------------------------

    #[test]
    fn 格式指纹对四种输入分别正确() {
        let two_space = fingerprint_of_text("{\n  \"a\": 1\n}\n");
        assert_eq!(two_space.indent.as_deref(), Some("  "));
        assert_eq!(two_space.newline, "\n");

        let tab_crlf = fingerprint_of_text("{\r\n\t\"a\": 1\r\n}\r\n");
        assert_eq!(tab_crlf.indent.as_deref(), Some("\t"));
        assert_eq!(tab_crlf.newline, "\r\n");

        let no_indent = fingerprint_of_text("{\n\"a\": 1\n}\n");
        assert_eq!(no_indent.indent.as_deref(), Some(""));
        assert_eq!(no_indent.newline, "\n");

        let compact = fingerprint_of_text("{\"a\":1}");
        assert_eq!(compact.indent, None, "全文无换行 ⇒ 压缩式");
    }

    // ---- patch：格式保持 -------------------------------------------------

    #[test]
    fn patch保持缩进风格且只改目标行() {
        let tmp = tempdir();
        let slot = slot_by_key("workbuddy-cn").unwrap();

        let cases = [
            ("2sp", "{\n  \"a\": 1,\n  \"b\": 2\n}\n"),
            ("tab_crlf", "{\r\n\t\"a\": 1,\r\n\t\"b\": 2\r\n}\r\n"),
            ("no_indent", "{\n\"a\": 1,\n\"b\": 2\n}\n"),
        ];
        for (name, content) in cases {
            let path = write_temp(tmp.path(), &format!("{name}.json"), content);
            let out = patch_at(&path, &json!({ "a": 7 }), slot, &backup_root(tmp.path()))
                .expect("patch 应成功");
            assert_eq!(out["changed"], json!(true), "{name} 应发生改写");

            let after = std::fs::read_to_string(&path).unwrap();
            assert_eq!(
                after,
                content.replace("\"a\": 1", "\"a\": 7"),
                "{name} 的排版被改动（必须逐字节保持）"
            );
            assert_eq!(
                changed_line_count(content, &after),
                1,
                "{name} 的 diff 必须只含目标键那一行"
            );
        }

        // 压缩式：无换行、无空格。
        let compact = "{\"a\":1,\"b\":2}";
        let path = write_temp(tmp.path(), "compact.json", compact);
        let out =
            patch_at(&path, &json!({ "a": 7 }), slot, &backup_root(tmp.path())).expect("patch");
        assert_eq!(out["changed"], json!(true));
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\"a\":7,\"b\":2}",
            "压缩式必须渲染成无空格紧凑形式"
        );
    }

    /// 未知键与键顺序不变 + 三级点号路径能定位 + 新增键插到末尾。
    #[test]
    fn 点号路径定位且未知键与顺序不变() {
        let tmp = tempdir();
        let slot = slot_by_key("workbuddy-cn").unwrap();
        let content = "{\n  \"zzz\": 1,\n  \"claw\": {\n    \"channels\": {\n      \"wechatmp\": {\n        \"enabled\": false\n      }\n    }\n  },\n  \"aaa\": 2\n}\n";
        let path = write_temp(tmp.path(), "nested.json", content);

        let out = patch_at(
            &path,
            &json!({ "claw.channels.wechatmp.enabled": true }),
            slot,
            &backup_root(tmp.path()),
        )
        .expect("三级点号路径应能定位");
        assert_eq!(out["changed"], json!(true));

        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            after,
            content.replace("\"enabled\": false", "\"enabled\": true")
        );
        assert_eq!(changed_line_count(content, &after), 1);
        // 键顺序不变（`zzz` 仍在 `aaa` 之前）。
        assert!(after.find("\"zzz\"").unwrap() < after.find("\"aaa\"").unwrap());

        // 新增键：插到父对象末尾，不重排已有键。
        let out2 = patch_at(
            &path,
            &json!({ "claw.channels.wechatmp.note": "hi" }),
            slot,
            &backup_root(tmp.path()),
        )
        .expect("新增键应成功");
        assert_eq!(out2["changed"], json!(true));
        let after2 = std::fs::read_to_string(&path).unwrap();
        assert!(after2.contains("\"note\": \"hi\""));
        assert!(after2.find("\"enabled\"").unwrap() < after2.find("\"note\"").unwrap());

        // 中间键缺失 ⇒ 把剩余路径整体补成嵌套对象。
        let out3 = patch_at(
            &path,
            &json!({ "brand.new.key": 5 }),
            slot,
            &backup_root(tmp.path()),
        )
        .expect("中间键缺失时应自动补嵌套对象");
        assert_eq!(out3["changed"], json!(true));
        let after3 = std::fs::read_to_string(&path).unwrap();
        assert!(after3.contains("\"brand\""));
        assert!(after3.contains("\"new\""));
        assert!(after3.contains("\"key\": 5"));
        assert!(after3.find("\"aaa\"").unwrap() < after3.find("\"brand\"").unwrap());
    }

    /// 幂等：同一 patch 两次只产生**一份**备份，第二次 `changed: false`。
    #[test]
    fn 同一patch两次只产生一份备份() {
        let tmp = tempdir();
        let slot = slot_by_key("workbuddy-cn").unwrap();
        let root = backup_root(tmp.path());
        let path = write_temp(tmp.path(), "settings.json", "{\n  \"a\": 1\n}\n");

        let first = patch_at(&path, &json!({ "a": 2 }), slot, &root).expect("首次 patch");
        assert_eq!(first["changed"], json!(true));
        assert!(first["backup"].is_string(), "首次改写必须留备份");

        let second = patch_at(&path, &json!({ "a": 2 }), slot, &root).expect("二次 patch");
        assert_eq!(second["changed"], json!(false), "内容相同 ⇒ 不得再改写");
        assert!(second["backup"].is_null(), "无改写 ⇒ 不得产生垃圾备份");

        let dir = root.join("client-settings").join(slot.key);
        assert_eq!(
            std::fs::read_dir(&dir).unwrap().count(),
            1,
            "同一 patch 两次只允许一份备份"
        );
    }

    // ---- 恢复 ------------------------------------------------------------

    #[test]
    fn 恢复后与备份点逐字节相同且恢复前再备份() {
        let tmp = tempdir();
        let slot = slot_by_key("workbuddy-cn").unwrap();
        let root = backup_root(tmp.path());
        let original = "{\n  \"a\": 1\n}\n";
        let path = write_temp(tmp.path(), "settings.json", original);

        // 第一次改写：a=2（备份点 1 存的是 a=1）
        patch_at(&path, &json!({ "a": 2 }), slot, &root).unwrap();
        let after_first = std::fs::read_to_string(&path).unwrap();
        // 第二次改写：a=3（备份点 2 存的是 a=2）
        patch_at(&path, &json!({ "a": 3 }), slot, &root).unwrap();

        let listed = list_backups_in(slot, &root);
        let ids: Vec<String> = listed["backups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(ids.len(), 2, "两次改写应有两份备份");
        // 最早的那份存的是 a=1（降序列表的末项）。
        let restore_id = ids.last().unwrap().clone();

        let out = restore_at(&path, slot, &restore_id, &root).expect("恢复应成功");
        assert_eq!(out["changed"], json!(true));
        assert!(
            out["backup"].is_string(),
            "恢复前必须把当前文件（a=3）另存一份新备份"
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            original,
            "恢复后必须与备份点逐字节相同"
        );

        let listed2 = list_backups_in(slot, &root);
        assert_eq!(
            listed2["backups"].as_array().unwrap().len(),
            3,
            "恢复前的新备份必须落盘"
        );
        // 恢复前的当前内容（a=2 的那一版）也在备份里 —— 用 `after_first` 反证。
        assert!(after_first.contains("\"a\": 2"));
    }

    #[test]
    fn 恢复id不得穿越目录() {
        let tmp = tempdir();
        let slot = slot_by_key("workbuddy-cn").unwrap();
        let path = write_temp(tmp.path(), "settings.json", "{}");
        for bad in ["..", "../x", "a/b", "a\\b", ""] {
            assert!(
                restore_at(&path, slot, bad, &backup_root(tmp.path())).is_err(),
                "非法 backupId 必须拒绝：{bad}"
            );
        }
    }

    // ---- 差异比对 --------------------------------------------------------

    #[test]
    fn 比对三类分类正确() {
        let a = json!({ "x": 1, "y": 2, "nested": { "p": 1 } });
        let b = json!({ "x": 1, "y": 9, "onlyB": true, "nested": { "p": 1 } });
        let out = diff_maps(&a, &b);
        let only_a: Vec<&str> = out["onlyA"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["path"].as_str().unwrap())
            .collect();
        let only_b: Vec<&str> = out["onlyB"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["path"].as_str().unwrap())
            .collect();
        let diff: Vec<&str> = out["different"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["path"].as_str().unwrap())
            .collect();

        // A 侧独有：`nested` 被拍平 ⇒ 实际是 `nested.p`（两侧都有，故不计）。
        assert!(only_a.is_empty(), "本例 A 侧无独有键：{only_a:?}");
        assert_eq!(only_b, vec!["onlyB"]);
        assert_eq!(diff, vec!["y"]);
        assert_eq!(out["counts"]["different"], json!(1));
        assert_eq!(out["counts"]["same"], json!(2)); // x, nested.p
        assert_eq!(out["counts"]["onlyB"], json!(1));

        // 反向：只 A 有的键。
        let out2 = diff_maps(&json!({ "a": 1 }), &json!({}));
        assert_eq!(out2["onlyA"].as_array().unwrap().len(), 1);
        assert_eq!(out2["onlyA"][0]["path"], json!("a"));
    }

    // ---- 错误与防护 ------------------------------------------------------

    #[test]
    fn 损坏json返回解析失败错误而非空对象() {
        let tmp = tempdir();
        let slot = slot_by_key("workbuddy-cn").unwrap();
        let path = write_temp(tmp.path(), "broken.json", "{\n  \"a\": 1,\n");
        let err = patch_at(&path, &json!({ "a": 2 }), slot, &backup_root(tmp.path()))
            .expect_err("损坏 JSON 必须报错");
        let decoded = decode_wire(&err);
        assert_eq!(
            decoded.code,
            Some(ErrorCode::ClientSettingsParseFailed),
            "必须带解析失败码：{err}"
        );
        assert!(decoded.params.iter().any(|(k, _)| k == "path"));

        // 顶层不是对象同样拒绝。
        let arr = write_temp(tmp.path(), "array.json", "[1,2,3]");
        let err2 = patch_at(&arr, &json!({ "a": 2 }), slot, &backup_root(tmp.path()))
            .expect_err("顶层数组必须报错");
        assert_eq!(
            decode_wire(&err2).code,
            Some(ErrorCode::ClientSettingsParseFailed)
        );
    }

    #[test]
    fn patch路径穿越一律拒绝() {
        let tmp = tempdir();
        let slot = slot_by_key("workbuddy-cn").unwrap();
        let path = write_temp(tmp.path(), "settings.json", "{\n  \"a\": 1\n}\n");
        let root = backup_root(tmp.path());
        for bad in ["..", "a..b", "a/../b", "a/b", "a\\b", "", "a.", ".a", "a.."] {
            let mut map = serde_json::Map::new();
            map.insert(bad.to_string(), json!(1));
            let err = patch_at(&path, &Value::Object(map), slot, &root)
                .expect_err(&format!("非法路径必须拒绝：{bad}"));
            assert!(
                err.contains("非法") || err.contains("不能为空") || err.contains("不得"),
                "{bad} ⇒ {err}"
            );
        }
        // 合法路径仍可通过。
        assert!(patch_at(&path, &json!({ "a": 2 }), slot, &root).is_ok());
    }

    #[test]
    fn 未安装时patch报错而非静默成功() {
        let tmp = tempdir();
        let slot = slot_by_key("workbuddy-cn").unwrap();
        let missing = tmp.path().join("nope").join("settings.json");
        let err = patch_at(&missing, &json!({ "a": 2 }), slot, &backup_root(tmp.path()))
            .expect_err("未安装必须报错");
        assert!(err.contains("未安装"), "{err}");
    }

    /// 进程占用时的错误必须带码与 `slot` 参数（前端按当前语言重渲染）。
    #[test]
    fn 进程占用错误带码与槽位参数() {
        let slot = slot_by_key("workbuddy-cn").unwrap();
        let err = process_running(slot);
        let decoded = decode_wire(&err);
        assert_eq!(
            decoded.code,
            Some(ErrorCode::ClientSettingsProcessRunning),
            "{err}"
        );
        assert_eq!(
            decoded.params,
            vec![("slot".to_string(), slot.display_key.to_string())]
        );
    }

    /// 扫描器：转义 / 嵌套 / 数组 / 尾部垃圾。
    #[test]
    fn 扫描器覆盖转义与嵌套() {
        let tree = parse_json(
            "{\n  \"a\\\"b\": \"x\\\\y\",\n  \"arr\": [1, {\"k\": null}],\n  \"u\": \"\\u4e2d\"\n}",
        )
        .unwrap();
        let NodeKind::Object(members) = &tree.kind else {
            panic!("顶层应为对象");
        };
        assert_eq!(members.len(), 3);
        assert_eq!(members[0].key, "a\"b", "转义键必须解码后用于定位");
        assert!(matches!(members[1].value.kind, NodeKind::Array(_)));
        assert_eq!(top_level_order(&tree), vec!["a\"b", "arr", "u"]);

        assert!(parse_json("{\"a\":1} 垃圾").is_err());
        assert!(parse_json("{\"a\":").is_err());
        assert!(parse_json("").is_err());
    }

    /// 值替换时保留原文件里**非目标键**的全部字节（含注释外的空白与换行）。
    #[test]
    fn 替换值不动周边空白() {
        let tmp = tempdir();
        let slot = slot_by_key("workbuddy-cn").unwrap();
        let content = "{\n  \"a\": 1,   \n  \"b\": [1, 2, 3]\n}\n";
        let path = write_temp(tmp.path(), "settings.json", content);
        patch_at(&path, &json!({ "b": [4] }), slot, &backup_root(tmp.path())).unwrap();
        let after = std::fs::read_to_string(&path).unwrap();
        assert_eq!(after, "{\n  \"a\": 1,   \n  \"b\": [\n    4\n  ]\n}\n");
    }

    // ---- 语义映射表（代理字段）------------------------------------------

    /// 代理字段取值域**写成常量并钉住**（域漂了必须变红）。
    #[test]
    fn 代理字段取值域常量被钉住() {
        assert_eq!(WORKBUDDY_PROXY_SUPPORT_VALUES, &["off", "on", "override"]);
        assert_eq!(
            TRAE_PROXY_MODE_VALUES,
            &["system", "direct", "manual", "cloud"]
        );
        // 本机实测落盘值必须落在域内（Trae 4 份 settings.json 实测均为 `"system"`）。
        assert!(TRAE_PROXY_MODE_VALUES.contains(&"system"));
        assert!(WORKBUDDY_PROXY_SUPPORT_VALUES.contains(&"on"));
    }

    /// 语义表与路径规格表**对齐**：引用的槽位都存在，且路径与实测一致。
    #[test]
    fn 语义映射表与槽位表对齐() {
        assert!(!PROXY_FIELDS.is_empty(), "代理语义表不得为空");
        for (semantic, slot_key, path) in PROXY_FIELDS {
            assert_eq!(*semantic, "proxy", "当前只有「代理」一组语义");
            let slot = slot_by_key(slot_key)
                .unwrap_or_else(|| panic!("语义表引用了不存在的槽位：{slot_key}"));
            match slot.product {
                "workbuddy" => assert!(
                    matches!(*path, "http.proxy" | "http.proxySupport"),
                    "WorkBuddy 侧只认 http.proxy / http.proxySupport：{path}"
                ),
                "trae" => assert_eq!(*path, "trae.network.proxy.effectiveMode"),
                other => panic!("未知产品：{other}"),
            }
        }
        // 每个槽位**都**要在语义表里有声明，否则跨产品比对会漏归并。
        for slot in CLIENT_SLOTS {
            assert!(
                PROXY_FIELDS.iter().any(|(_, key, _)| *key == slot.key),
                "槽位 {} 缺代理语义声明",
                slot.key
            );
        }
        // 路径表与语义表**分离**：语义表里没有重复的 `(槽位, 路径)`。
        let mut pairs: Vec<(&str, &str)> = PROXY_FIELDS
            .iter()
            .map(|(_, slot, path)| (*slot, *path))
            .collect();
        pairs.sort_unstable();
        pairs.dedup();
        assert_eq!(pairs.len(), PROXY_FIELDS.len(), "语义表有重复项");
    }

    /// 跨产品比对：代理字段按映射表归并，不再各报一条 `onlyA` / `onlyB`。
    #[test]
    fn 跨产品比对归并代理字段() {
        // A 侧用**嵌套**形态（WorkBuddy / VS Code 的 `http` → `proxy`），
        // B 侧用**字面含点键**形态（Trae 的键本身就叫 `trae.network.proxy.effectiveMode`）
        // —— 两种在拍平后都归一到同一组点号路径，必须都能被摘除。
        let a = json!({
            "http": { "proxy": "http://127.0.0.1:7890", "proxySupport": "on" },
            "claw": { "a": 1 }
        });
        let b = json!({
            "trae.network.proxy.effectiveMode": "manual",
            "window.autoDetectColorScheme": true
        });
        let out = diff_values(&a, "workbuddy-cn", "workbuddy", &b, "trae-code-cn", "trae");

        let groups = out["proxyGroups"]
            .as_array()
            .expect("proxyGroups 必须是数组");
        assert_eq!(groups.len(), 1, "跨产品必须归并出一组代理");
        assert_eq!(groups[0]["semantic"], json!("proxy"));
        assert_eq!(groups[0]["a"].as_array().unwrap().len(), 2, "A 侧两条字段");
        assert_eq!(groups[0]["b"].as_array().unwrap().len(), 1, "B 侧一条字段");
        assert_eq!(groups[0]["a"][0]["path"], json!("http.proxy"));
        assert_eq!(groups[0]["a"][0]["present"], json!(true));
        assert_eq!(groups[0]["b"][0]["value"], json!("manual"));
        assert_eq!(groups[0]["same"], json!(false), "两侧有效值集合不同");

        // 代理字段**不再**出现在三类里。
        let paths: Vec<&str> = ["onlyA", "onlyB", "different"]
            .iter()
            .flat_map(|bucket| out[*bucket].as_array().unwrap().iter())
            .map(|entry| entry["path"].as_str().unwrap())
            .collect();
        assert!(
            !paths.iter().any(|path| path.contains("proxy")),
            "代理字段必须被归并：{paths:?}"
        );
        // 非代理字段照常三类分类。
        assert!(out["onlyA"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"] == json!("claw.a")));
        assert!(out["onlyB"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"] == json!("window.autoDetectColorScheme")));

        // 一侧没有代理字段：仍给出归并组（`present: false`），不当成「仅 B 有」。
        let out2 = diff_values(
            &json!({}),
            "workbuddy-cn",
            "workbuddy",
            &json!({ "trae.network.proxy.effectiveMode": "system" }),
            "trae-code-cn",
            "trae",
        );
        let groups2 = out2["proxyGroups"].as_array().unwrap();
        assert_eq!(groups2.len(), 1);
        assert_eq!(groups2[0]["a"][0]["present"], json!(false));
        assert_eq!(groups2[0]["a"][0]["value"], Value::Null);
        assert!(
            out2["onlyB"].as_array().unwrap().is_empty(),
            "代理字段不得落进 onlyB"
        );

        // 同产品：不做归并（代理字段走普通三类口径）。
        let same = diff_values(
            &a,
            "workbuddy-cn",
            "workbuddy",
            &a,
            "workbuddy-global",
            "workbuddy",
        );
        assert!(same["proxyGroups"].as_array().unwrap().is_empty());
        assert_eq!(same["counts"]["same"], json!(3));
        assert_eq!(same["counts"]["onlyA"], json!(0));
    }
}
