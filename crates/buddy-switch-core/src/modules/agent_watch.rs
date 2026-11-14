//! 外部 Agent 会话监听（P15 · S1）：`hook 事件 → 会话状态` 的**纯状态机**。
//!
//! ## 为什么单独一个模块，而不是塞进 `companion.rs`
//!
//! [`super::companion`] 是**只读快照的契约**（`CompanionItem` / `CompanionSnapshot` /
//! `LiveOperation` + `snapshot_from_operations`）—— 它回答「怎么把一组操作渲染成窗口」。
//! 本模块回答另一个问题：「**外部客户端的 hook 事件怎么变成一组操作**」。
//! 两者是**生产者 / 消费者**关系：本模块产出 [`LiveOperation`]，`companion` 消费它。
//! 混在一起会让「契约」与「事件语义」互相污染 —— 改一个客户端的事件映射，不该动契约文件。
//!
//! ## ★ 零周期任务（`P15-SPIKE-RESULTS §2 Q2`）
//!
//! 本模块只有两个入口：[`AgentWatch::ingest`]（事件到达时）与 [`AgentWatch::operations`]
//! （读快照时）。「过期」是**读取时惰性判定**的，不是后台定时清理 —— 因此新增本能力
//! **不需要**在 `background.rs` 的 `TASKS` 注册表里登记任何东西
//! （把「漏登记 ⇒ 任务永不执行」的事故面**归零**）。
//!
//! ## ★ 四态不扩枚举（`P13-REFERENCE-PARITY §8.6`）
//!
//! 外部实现有 6 态（`unknown/running/wait/done/error/aborted`），本仓只有 4 态
//! （[`CompanionState`]）。这里**做映射**而不是扩契约：内部的 [`WatchPhase`] 保留
//! 「已中止」与「失败」的区分（只影响 `hint` 文案），对外一律折叠进
//! [`CompanionState::Failed`]。
//!
//! ⚠️ **诚实边界**：纯 hook 来源**几乎不会出现「失败」** —— 客户端的
//! `Stop` / `SessionEnd` / `Interrupt` 只产生「已完成 / 已中止」；
//! 「失败」只在客户端显式报告失败时才出现。UI 文案不得暗示「失败」是常见态。
//!
//! ## ★ 不读客户端文件（与外部实现同一条纪律）
//!
//! 会话状态**只**来自 hook 事件。本模块不读客户端数据库、不读会话文件、不读 transcript ——
//! 这是「轻量」的来源，也是「客户端退出后状态会停在最后已知值」这条已知限制的根因。

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

use super::companion::{CompanionState, LiveOperation};

/// WorkBuddy **国内版**的来源 id（与 hook 命令的 `--source` 取值一一对应）。
///
/// ⚠️ 取值**不带空格**：它会被嵌进客户端 `settings.json` 里的一条命令串，
/// 带空格的参数会被按空格拆开。展示名由前端做映射（`shared.companion.source.*`）。
pub const SOURCE_WORKBUDDY_CN: &str = "workbuddy-cn";
/// WorkBuddy **国际版**的来源 id。
///
/// 与国内版**分家**：两边是两个独立的客户端、两套 `settings.json`，
/// 用户可能同时开着 —— 混在一个 source 下就分不清哪条会话来自哪个版本。
pub const SOURCE_WORKBUDDY_GLOBAL: &str = "workbuddy-global";
/// Claude Code 来源 id（`~/.claude/settings.json` 的 command hooks）。
pub const SOURCE_CLAUDE: &str = "claude";
/// CodeBuddy CLI 来源 id（`~/.codebuddy/settings.json` 的 command hooks）。
pub const SOURCE_CODEBUDDY: &str = "codebuddy";
/// Codex 来源 id。
///
/// ⚠️ **只作来源，不作安装目标**：本仓对它的 hook 配置**路径与 schema 未取证**
/// （本机未安装、只有二手文档），写错形态会污染用户配置。等取证后再接安装面。
pub const SOURCE_CODEX: &str = "codex";
/// Trae **国内版**来源 id。
///
/// ⚠️ 同 [`SOURCE_CODEX`]：Trae 的 hook 是**文件式**（`hooks.json`），
/// 其**事件名与 handler 结构未取证** ⇒ 只作来源，不写它的配置。
pub const SOURCE_TRAE_CN: &str = "trae-cn";
/// Trae **国际版**来源 id。
pub const SOURCE_TRAE_GLOBAL: &str = "trae-global";
/// Qoder **国内版**来源 id。
///
/// ⚠️ **本仓对 Qoder 是否有 hooks 机制没有任何取证** ⇒ 只作来源。
pub const SOURCE_QODER_CN: &str = "qoder-cn";
/// Qoder **国际版**来源 id。
pub const SOURCE_QODER_GLOBAL: &str = "qoder-global";

/// **全部可作为 hook 来源**的客户端 id —— **唯一事实来源**。
///
/// ## 它为什么必须是一张表
///
/// 来源 id 出现在**四处**：① hook 命令的 `--source` 白名单（[`canonical_source`]）；
/// ② 客户端配置里的命令串；③ payload 里的 `agent_source`；④ 前端展示名映射
/// （`companion-row.tsx` 的 `SOURCE_LABEL_KEY` + `shared.companion.source.*` 词条）。
/// 四处散着写 ⇒ 加一个客户端时**必然漏一处**，而漏掉的症状是**静默的**：
/// 新来源被回落成 WorkBuddy（见 [`canonical_source`] 的调用方），窗口里显示成别人的会话。
/// 故收敛成一张表，并用门禁 `scripts/check-agent-sources.cjs` 钉住 ①↔④ 一致。
///
/// ⚠️ 加一个来源时**同时**要改：本表 + 前端 `SOURCE_LABEL_KEY` + zh/en 两套词条。
pub const HOOK_SOURCES: &[&str] = &[
    SOURCE_WORKBUDDY_CN,
    SOURCE_WORKBUDDY_GLOBAL,
    SOURCE_CLAUDE,
    SOURCE_CODEBUDDY,
    SOURCE_CODEX,
    SOURCE_TRAE_CN,
    SOURCE_TRAE_GLOBAL,
    SOURCE_QODER_CN,
    SOURCE_QODER_GLOBAL,
];

/// 把任意串**收敛成登记在册的来源 id**；不在表里 ⇒ `None`。
///
/// ## 为什么是「整段精确匹配」
///
/// 与统一网关命名空间同一条纪律：**禁前缀 / 禁 `contains` 兜底**。
/// 带兜底的实现会把 `codex-desktop` 认成 `codex`、把 `trae-cn-x` 认成 `trae-cn` ——
/// 那等于让一个谁都不认识的来源**冒充**成某个真实客户端，比拒绝更难查。
pub fn canonical_source(raw: &str) -> Option<&'static str> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    HOOK_SOURCES.iter().copied().find(|id| *id == raw)
}

/// **已终态**的会话在窗口里保留多久（惰性过期）。
///
/// 30 分钟：足够让用户离开键盘一趟回来还看得到「刚才那个跑完了」，
/// 又不至于让窗口被历史任务堆满。
pub const COMPLETED_TTL_MS: i64 = 30 * 60 * 1000;

/// **非终态**的会话静默多久后不再渲染。
///
/// 45 分钟：hook 是「有活动才有事件」的，超过这个窗口仍无任何事件 ⇒
/// 任务多半已经随客户端退出而消失（本仓**不做**宿主进程探测，见 spike §2 Q2 的残留风险）。
/// 此时**不渲染**比渲染一个永远转圈的「运行中」诚实。
pub const STALE_TTL_MS: i64 = 45 * 60 * 1000;

/// 会话表上限；超出时按 `updated_at` 淘汰最旧的。
///
/// 上限只为给内存一个有界的天花板 —— 正常使用远到不了（惰性过期已经筛掉了绝大多数）。
pub const MAX_SESSIONS: usize = 64;

/// 会话标题截断的**字符**数（不是字节）。
const TITLE_MAX_CHARS: usize = 240;

/// 待确认项文本截断的**字符**数。
const WAIT_TEXT_MAX_CHARS: usize = 120;

/// 内部相位（比对外契约多两档，见模块文档）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WatchPhase {
    Running,
    Waiting,
    Done,
    /// 被中断（用户主动 `Interrupt` / 客户端退出），对外折叠成 [`CompanionState::Failed`]。
    ///
    /// ⚠️ **这里刻意没有独立的 `Failed` 变体**：本仓当前的**事件集**（客户端 hook）里
    /// **不存在**「显式报告失败」的事件 —— 参考实现里的 `error` 来自 transcript / webhook，
    /// 不在 hook 里。真接了那条链路再加；现在不留一个永不构造的变体（死代码纪律）。
    Aborted,
}

impl WatchPhase {
    /// 是否已终态（终态的会话走「保留期」而不是「静默期」）。
    fn is_terminal(self) -> bool {
        matches!(self, WatchPhase::Done | WatchPhase::Aborted)
    }

    /// 对外四态映射（见模块文档：不扩枚举，只折叠）。
    fn to_companion(self) -> CompanionState {
        match self {
            WatchPhase::Running => CompanionState::Running,
            WatchPhase::Waiting => CompanionState::Waiting,
            WatchPhase::Done => CompanionState::Done,
            WatchPhase::Aborted => CompanionState::Failed,
        }
    }
}

/// 归一化后的 hook 事件种类。
///
/// ⚠️ 这是**本仓的**词汇，不是客户端的：客户端事件名（`PreToolUse` / `Notification`…）
/// 到本枚举的映射在 [`HookKind::from_hook`]，是唯一允许看客户端事件名的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HookKind {
    /// 不认识的事件 ⇒ 整条丢弃（**不猜**）。
    #[default]
    Unknown,
    /// 新一轮开始：`SessionStart` / `UserPromptSubmit`。
    Start,
    /// 有进展但不需要人：`PreToolUse`（非提问类工具）。
    Step,
    /// **需要人**：提问类工具的 `PreToolUse`、`PermissionRequest`、确认类 `Notification`。
    Wait,
    /// 待确认项被回答：`PostToolUse` / `PostToolUseFailure`。
    Resolve,
    /// 正常结束：`Stop` / `SessionEnd`。
    End,
    /// 被中断：`Interrupt`。
    Abort,
    /// 有活动但状态不变：非确认类 `Notification`。
    Activity,
}

impl HookKind {
    /// 客户端事件名 → 本仓词汇。**唯一**允许出现客户端事件名的地方。
    ///
    /// 事件集与外部参考实现逐条对齐（`P13 §4.3 P15b`）：本仓先接 WorkBuddy 与 Codex
    /// 共用的那 8 个；`PreCompact` 等不改变展示状态的**刻意不接**（无数据不渲染）。
    fn from_hook(event: &str, tool: &str, payload: &Value) -> Self {
        match event {
            "SessionStart" | "UserPromptSubmit" => HookKind::Start,
            "PreToolUse" => {
                if is_question_tool(tool) {
                    HookKind::Wait
                } else {
                    HookKind::Step
                }
            }
            "PostToolUse" | "PostToolUseFailure" => HookKind::Resolve,
            "PermissionRequest" => HookKind::Wait,
            "Notification" => {
                if is_confirmation_notification(payload) {
                    HookKind::Wait
                } else {
                    HookKind::Activity
                }
            }
            "Stop" | "SessionEnd" => HookKind::End,
            "Interrupt" => HookKind::Abort,
            _ => HookKind::Unknown,
        }
    }
}

/// 提问类工具名（客户端把「问用户」实现成一次工具调用）。
///
/// 判据与外部参考实现一致：**全等**，或以 `__` / `.` 分隔后**以它结尾**
/// （客户端常给工具加 `mcp__<server>__` 前缀）。
/// 写成列表而不是正则：这几条的形态由客户端固定，加上限更少的依赖。
const QUESTION_TOOLS: &[&str] = &[
    "request_user_input",
    "request_user_input_async",
    "AskUserQuestion",
    "ask_user_question",
    "RequestUserInput",
];

/// 见 [`QUESTION_TOOLS`]。
fn is_question_tool(tool: &str) -> bool {
    QUESTION_TOOLS.iter().any(|name| {
        tool == *name || tool.ends_with(&format!("__{name}")) || tool.ends_with(&format!(".{name}"))
    })
}

/// 这条 `Notification` 是否意味着「等你确认」。
///
/// ⚠️ 这条判据是**启发式**：客户端把「需要人」的语义塞在自由文本里（`notification_type`
/// 或 `message` / `title`），没有稳定字段。三条规则按**优先级**短路，
/// 且 `auth_success` **先**排除 —— 它是纯提示，把它当「待确认」会让窗口一直亮着。
fn is_confirmation_notification(payload: &Value) -> bool {
    let kind = payload
        .get("notification_type")
        .or_else(|| payload.get("notificationType"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if kind == "auth_success" {
        return false;
    }
    if kind == "idle_prompt"
        || kind.contains("permission")
        || kind.contains("confirm")
        || kind.contains("approval")
        || kind.contains("credential")
    {
        return true;
    }
    let text = format!(
        "{} {}",
        payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        payload
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    );
    let lower = text.to_ascii_lowercase();
    text.contains('?')
        || text.contains('？')
        || text.contains("确认")
        || text.contains("允许")
        || text.contains("拒绝")
        || text.contains("凭证")
        || lower.contains("allow ")
        || lower.contains("deny ")
        || lower.contains("credential")
}

/// 一条**归一化后**的 hook 事件（[`AgentWatch::ingest`] 的唯一入参）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEvent {
    /// 来源 id（如 [`SOURCE_WORKBUDDY_CN`]）；空串会被 [`AgentWatch::ingest`] 拒绝。
    pub source: String,
    /// 客户端侧的会话 id；空串会被拒绝（没有它就无法稳定去重）。
    pub session_id: String,
    /// 归一化事件种类。
    pub kind: HookKind,
    /// 事件时间（毫秒时间戳）。
    pub ts: i64,
    /// 会话标题（可选）。
    pub title: Option<String>,
    /// 工具调用 id（可选）—— 待确认项的**去重键**。
    pub call_id: Option<String>,
    /// 工具名（可选）—— 进 `hint`，让人知道「在等什么」。
    pub tool: Option<String>,
}

/// 一个被监听的会话（内部结构，不对外）。
#[derive(Debug, Clone)]
struct WatchedSession {
    title: String,
    phase: WatchPhase,
    updated_at: i64,
    /// 待确认项：`(call_id, 文本)`；按 `call_id` 去重。
    pending: Vec<(String, String)>,
    /// 有进展但不需要人的步骤计数（只用于 `hint` 的密度判断，不进契约）。
    steps: usize,
}

/// 会话状态机。**纯内存、无 IO、无全局状态** —— 由宿主持有一份。
#[derive(Debug, Default)]
pub struct AgentWatch {
    /// 键 = `"{source}:{session_id}"`。
    ///
    /// `BTreeMap` 而不是 `HashMap`：遍历序稳定 ⇒ 单测断言不必依赖哈希顺序，
    /// 也让「同样的输入产出同样的输出」成为可验证的性质。
    sessions: BTreeMap<String, WatchedSession>,
}

impl AgentWatch {
    /// 空表。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前表里有多少会话（**含**已过期但尚未被淘汰的）—— 供单测与诊断用。
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    /// 是否为空表。
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// 吃掉一条事件；返回它是否被接受。
    ///
    /// **被拒绝的四类**（返回 `false`，且**不改任何状态**）：
    /// ① `source` / `session_id` 为空串；② [`HookKind::Unknown`]；
    /// ③ **时间回退**（`ts < 该会话的 updated_at`）—— hook 是子进程发的，
    /// 乱序与重放都真实存在，让它改状态会把「已完成」拉回「运行中」；
    /// ④ 终态会话收到**非 [`HookKind::Start`]** 事件（同上，防止事后到达的
    /// `PostToolUse` 把已结束的一轮重新点亮）。
    pub fn ingest(&mut self, event: AgentEvent) -> bool {
        if event.source.trim().is_empty() || event.session_id.trim().is_empty() {
            return false;
        }
        if event.kind == HookKind::Unknown {
            return false;
        }
        let key = format!("{}:{}", event.source, event.session_id);

        let existing = self.sessions.get(&key);
        // ★ 时间回退丢弃（规则 ③）。
        if let Some(existing) = existing {
            if event.ts < existing.updated_at {
                return false;
            }
            // ★ 终态不再被非 Start 事件唤醒（规则 ④）。
            if existing.phase.is_terminal() && event.kind != HookKind::Start {
                return false;
            }
        }

        let mut session = match existing {
            Some(existing) => existing.clone(),
            None => WatchedSession {
                title: String::new(),
                phase: WatchPhase::Running,
                updated_at: event.ts,
                pending: Vec::new(),
                steps: 0,
            },
        };

        // `Start` 即新一轮：清掉上一轮的待确认项与步骤计数。
        if event.kind == HookKind::Start {
            session.pending.clear();
            session.steps = 0;
        }

        if let Some(title) = event.title.as_deref() {
            let title = title.trim();
            if !title.is_empty() {
                session.title = truncate_chars(title, TITLE_MAX_CHARS);
            }
        }

        match event.kind {
            HookKind::Start | HookKind::Activity => {
                session.phase = if session.pending.is_empty() {
                    WatchPhase::Running
                } else {
                    WatchPhase::Waiting
                };
            }
            HookKind::Step => {
                session.steps = session.steps.saturating_add(1);
                session.phase = if session.pending.is_empty() {
                    WatchPhase::Running
                } else {
                    WatchPhase::Waiting
                };
            }
            HookKind::Wait => {
                let call_id = event
                    .call_id
                    .clone()
                    .filter(|id| !id.trim().is_empty())
                    .unwrap_or_else(|| format!("wait:{}", event.ts));
                if !session.pending.iter().any(|(id, _)| id == &call_id) {
                    session.pending.push((call_id, wait_text(&event)));
                }
                session.phase = WatchPhase::Waiting;
            }
            HookKind::Resolve => {
                if let Some(call_id) = event.call_id.as_deref() {
                    session.pending.retain(|(id, _)| id != call_id);
                }
                session.phase = if session.pending.is_empty() {
                    WatchPhase::Running
                } else {
                    WatchPhase::Waiting
                };
            }
            HookKind::End => {
                session.phase = WatchPhase::Done;
                session.pending.clear();
            }
            HookKind::Abort => {
                session.phase = WatchPhase::Aborted;
                session.pending.clear();
            }
            HookKind::Unknown => unreachable!("已在上面拒掉"),
        }

        session.updated_at = event.ts;
        self.sessions.insert(key, session);
        self.prune();
        true
    }

    /// 丢弃某个来源的全部会话（「客户端已退出」/ 关闭该来源的监听时调用）。
    ///
    /// 返回被丢掉的条数。
    pub fn drop_source(&mut self, source: &str) -> usize {
        let before = self.sessions.len();
        let prefix = format!("{source}:");
        self.sessions.retain(|key, _| !key.starts_with(&prefix));
        before - self.sessions.len()
    }

    /// 撤掉**一条**会话。
    ///
    /// 用途：**「没真正发生」的操作**。典型是批量签到因「已有任务在跑」被 `skipped`
    /// 掉 —— 此时刚写进去的那条「运行中」必须**整行撤掉**，而不是把一个没发生的事
    /// 显示成「已完成」或「已中止」（两种都是编造）。
    ///
    /// 与 [`AgentWatch::drop_source`] 的区别：那个按来源整片清，这个只清一条。
    /// 返回是否真的删掉了。
    pub fn forget(&mut self, source: &str, session_id: &str) -> bool {
        self.sessions
            .remove(&format!("{source}:{session_id}"))
            .is_some()
    }

    /// 产出**当前该渲染**的操作列表（`updated_at` **降序**）。
    ///
    /// 两档惰性过期（见 [`COMPLETED_TTL_MS`] / [`STALE_TTL_MS`]）在这里判定，
    /// **不改状态** —— 所以反复调用同一时刻是幂等的，过期条目也不会被「复活」。
    pub fn operations(&self, now: i64) -> Vec<LiveOperation> {
        let mut live: Vec<LiveOperation> = self
            .sessions
            .iter()
            .filter_map(|(key, session)| {
                let age = now.saturating_sub(session.updated_at);
                let ttl = if session.phase.is_terminal() {
                    COMPLETED_TTL_MS
                } else {
                    STALE_TTL_MS
                };
                if age > ttl {
                    return None;
                }
                // id 里的 `:` 分隔符由 `key` 保证（`source:session_id`）；
                // `source` 与 `session_id` 里若本身含 `:` 也只是让 id 更长，不影响唯一性。
                Some(LiveOperation {
                    id: key.clone(),
                    source: session_key_source(key),
                    title: session.title.clone(),
                    hint: hint_for(session, now),
                    state: session.phase.to_companion(),
                })
            })
            .collect();
        // ★ `updated_at` 降序；同刻时按 id 升序 —— **全字段确定性**排序，
        // 否则同一批数据两次快照的顺序可能不同（前端会看到行在跳）。
        live.sort_by(|a, b| {
            let a_ts = self.sessions.get(&a.id).map(|s| s.updated_at).unwrap_or(0);
            let b_ts = self.sessions.get(&b.id).map(|s| s.updated_at).unwrap_or(0);
            b_ts.cmp(&a_ts).then_with(|| a.id.cmp(&b.id))
        });
        live
    }

    /// 超出 [`MAX_SESSIONS`] 时淘汰最旧的（终态优先）。
    fn prune(&mut self) {
        while self.sessions.len() > MAX_SESSIONS {
            let victim = self
                .sessions
                .iter()
                .min_by(|(a_key, a), (b_key, b)| {
                    // 终态优先淘汰 ⇒ 用「是否终态」当第一键（`true` 先）。
                    let a_term = a.phase.is_terminal();
                    let b_term = b.phase.is_terminal();
                    b_term
                        .cmp(&a_term)
                        .then_with(|| a.updated_at.cmp(&b.updated_at))
                        // 同刻同态时按 key 定序，保证淘汰是确定性的（不是随机的）。
                        .then_with(|| a_key.cmp(b_key))
                })
                .map(|(key, _)| key.clone());
            match victim {
                Some(key) => {
                    self.sessions.remove(&key);
                }
                None => break,
            }
        }
    }
}

/// 进程内共享的会话表。
///
/// ## 为什么是全局单例，而不是让宿主各自持有
///
/// 「谁往表里写」只有一处（[`super::agent_hook_rx`] 的 HTTP handler），
/// 「谁读」也只有一处（`companion_status` 的 provider）。两边在不同模块里，
/// 若各自持有一份句柄，就得在 Tauri 的 `State` 与后台任务之间来回传 —— 那层传参是纯噪音，
/// 且**漏传一次就变成「收得到事件但读不到」**（本项目最贵的一类静默故障）。
///
/// ⚠️ 两个宿主（桌面 `src-tauri` / webui `buddy-switch-server`）是**两个进程**，
/// 各自有一份 `OnceLock` —— 这正是期望行为：它们本来就监听各自的端口、服务各自的入口。
pub fn shared() -> &'static Mutex<AgentWatch> {
    static SHARED: OnceLock<Mutex<AgentWatch>> = OnceLock::new();
    SHARED.get_or_init(|| Mutex::new(AgentWatch::new()))
}

/// 本工具**自己**发起的操作（签到这个量级的长操作）在悬浮窗里的来源 id。
///
/// 与客户端 hook 的来源（`workbuddy-cn` 等）**刻意分开**：窗口里一行就能看出
/// 「这是客户端里的任务」还是「这是本工具正在做的事」。
pub const SOURCE_THIS_TOOL: &str = "buddy-switch";

/// 记录一条**本工具自己的操作**（这类操作没有客户端 hook，但有明确的起止）。
///
/// ## 为什么复用同一张表
///
/// 悬浮窗对「任务」的呈现只有一种形态（四态 + 标题 + 时间）。给本工具的操作另开一套
/// 展示逻辑，等于让用户学两套 —— 而复用是零成本的：[`AgentWatch`] 已经能表达
/// 「开始 / 等待 / 结束 / 中止」，这也是 `P9-ARCH §1.3` 当初把数据源写成
/// 「`switch-progress` / `checkin-progress` / `trae-checkin-progress`」的用意。
///
/// ## 幂等
///
/// 同一个 `task_id` 反复写是**安全**的：`Start` 只重置待确认项与步骤计数，
/// 终态之后再写非 `Start` 会被 [`AgentWatch::ingest`] 的「终态不被唤醒」规则拒掉。
pub fn record_local(task_id: &str, title: &str, state: CompanionState, now: i64) -> bool {
    shared()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .ingest(local_event(task_id, title, state, now))
}

/// [`record_local`] 的事件构造 —— **纯函数**，单测据此在隔离的表上验证（不碰全局单例）。
fn local_event(task_id: &str, title: &str, state: CompanionState, now: i64) -> AgentEvent {
    let kind = match state {
        CompanionState::Running => HookKind::Start,
        CompanionState::Waiting => HookKind::Wait,
        CompanionState::Done => HookKind::End,
        CompanionState::Failed => HookKind::Abort,
    };
    AgentEvent {
        source: SOURCE_THIS_TOOL.to_string(),
        session_id: task_id.to_string(),
        kind,
        ts: now,
        title: Some(title.to_string()),
        call_id: None,
        tool: None,
    }
}

/// 从 `"source:session_id"` 里取回 `source`（`session_id` 自身可能含 `:`，
/// 所以**只切第一个**）。
fn session_key_source(key: &str) -> String {
    key.split_once(':')
        .map(|(source, _)| source)
        .unwrap_or(key)
        .to_string()
}

/// 待确认项的展示文本：优先工具名，其次通用兜底。
fn wait_text(event: &AgentEvent) -> String {
    let tool = event.tool.as_deref().unwrap_or_default().trim();
    if tool.is_empty() {
        String::new()
    } else {
        truncate_chars(tool, WAIT_TEXT_MAX_CHARS)
    }
}

/// `hint` = 相对时间（+ 终态补充说明）。
///
/// 相对时间由**后端**算（前端只渲染 `hint` 字符串，见 `companion-row.tsx` 的
/// `shared.companion.info.time` 用法）⇒ 每次 `operations()` 都会重新算，
/// 所以窗口里显示的年龄会随轮询自然前进。
fn hint_for(session: &WatchedSession, now: i64) -> String {
    let relative = relative_time(now, session.updated_at);
    let suffix = match session.phase {
        WatchPhase::Waiting => "等待确认",
        WatchPhase::Aborted => "已中止",
        WatchPhase::Running | WatchPhase::Done => "",
    };
    if suffix.is_empty() {
        relative
    } else {
        format!("{relative} · {suffix}")
    }
}

/// 毫秒时间戳 → 中文相对时间。
///
/// 刻意保持**粗粒度**（分钟 / 小时 / 天）：这个字符串每次轮询都会重算，
/// 秒级精度只会让窗口内容无意义地跳动。
fn relative_time(now: i64, ts: i64) -> String {
    let diff = now.saturating_sub(ts).max(0);
    let minutes = diff / 60_000;
    if minutes < 1 {
        return "刚刚".to_string();
    }
    if minutes < 60 {
        return format!("{minutes} 分钟前");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours} 小时前");
    }
    format!("{} 天前", hours / 24)
}

/// 按**字符**截断（不是字节）—— 中文标题按字节切会切出半个字符。
fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    text.chars().take(max).collect()
}

// ---------------------------------------------------------------------------
// hook payload → AgentEvent
// ---------------------------------------------------------------------------

/// 把客户端送来的 hook payload 解析成一条 [`AgentEvent`]。
///
/// 返回 `None` = 这条 payload **没有可用的会话标识**（宿主应当回 2xx 但什么都不做，
/// 不能当成错误 —— 客户端会因非 2xx 重试甚至报错）。
///
/// `default_source` 由**接收路径**给出（hook 命令的 `--source`），
/// payload 里的 `agent_source` 优先 —— 这样同一台机器上多个客户端的 hook
/// 可以共用一个接收端。
pub fn parse_event(default_source: &str, payload: &Value, now: i64) -> Option<AgentEvent> {
    let source = first_str(payload, &["agent_source", "agentSource", "source"])
        .unwrap_or(default_source)
        .to_string();
    let session_id = first_str(
        payload,
        &[
            "session_id",
            "sessionId",
            "conversation_id",
            "conversationId",
        ],
    )?
    .to_string();
    let event_name =
        first_str(payload, &["hook_event_name", "hookEventName", "event"]).unwrap_or_default();
    let tool = first_str(payload, &["tool_name", "toolName", "tool"]).unwrap_or_default();
    let kind = HookKind::from_hook(event_name, tool, payload);
    let ts = payload_ts(payload).unwrap_or(now);
    let title =
        first_str(payload, &["session_title", "sessionTitle", "prompt"]).map(str::to_string);
    let call_id =
        first_str(payload, &["tool_use_id", "toolUseId", "call_id", "callId"]).map(str::to_string);
    Some(AgentEvent {
        source,
        session_id,
        kind,
        ts,
        title,
        call_id,
        tool: if tool.is_empty() {
            None
        } else {
            Some(tool.to_string())
        },
    })
}

/// 取第一个「非空白字符串」候选键的值。
fn first_str<'a>(payload: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| payload.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// 解析事件时间戳。
///
/// **只认数字**：客户端给的是 epoch 毫秒（少数给秒）。字符串时间戳（ISO8601）
/// 本仓不做解析 —— 缺依赖，且客户端两代实现都用数字。
/// 判据：小于 `1e12`（约公元 33658 年的毫秒值）就当成**秒**再乘 1000。
fn payload_ts(payload: &Value) -> Option<i64> {
    let raw = payload
        .get("timestamp")
        .or_else(|| payload.get("ts"))
        .or_else(|| payload.get("created_at"))?;
    let value = match raw {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64))?,
        _ => return None,
    };
    if value <= 0 {
        return None;
    }
    Some(if value < 1_000_000_000_000 {
        value.saturating_mul(1000)
    } else {
        value
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(kind: HookKind, ts: i64) -> AgentEvent {
        AgentEvent {
            source: SOURCE_WORKBUDDY_CN.to_string(),
            session_id: "s-1".to_string(),
            kind,
            ts,
            title: None,
            call_id: None,
            tool: None,
        }
    }

    /// ★ 空表 ⇒ **没有任何操作**（不造假、不占位）。
    #[test]
    fn empty_watch_yields_no_operations() {
        let watch = AgentWatch::new();
        assert!(watch.operations(1_000).is_empty());
        assert!(watch.is_empty());
    }

    /// ★ 四态**都能被事件驱动**（不是「恰好 4 条」—— 条数由事件决定）。
    #[test]
    fn all_four_states_are_reachable_from_events() {
        let mut watch = AgentWatch::new();
        // running
        assert!(watch.ingest(event(HookKind::Start, 1_000)));
        // waiting
        let mut wait = event(HookKind::Wait, 1_100);
        wait.call_id = Some("call-1".to_string());
        wait.tool = Some("AskUserQuestion".to_string());
        assert!(watch.ingest(wait));
        // done
        assert!(watch.ingest(event(HookKind::End, 1_200)));

        let ops = watch.operations(1_300);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].state, CompanionState::Done);

        // 另一条会话：aborted → 对外是 Failed
        let mut aborted = event(HookKind::Abort, 1_400);
        aborted.session_id = "s-2".to_string();
        assert!(watch.ingest(aborted));
        let ops = watch.operations(1_500);
        let aborted_row = ops
            .iter()
            .find(|op| op.id.ends_with("s-2"))
            .expect("中止的会话必须在表里");
        assert_eq!(aborted_row.state, CompanionState::Failed);
        assert!(
            aborted_row.hint.contains("已中止"),
            "hint 必须区分「中止」与「失败」：{}",
            aborted_row.hint
        );
    }

    /// ★ 待确认 → 回答后**回到运行中**（不是停在「待确认」）。
    ///
    /// 可证伪性：把 `Resolve` 分支改成不重算 phase（留下 `Waiting`）⇒ 变红。
    #[test]
    fn resolve_returns_to_running_after_wait() {
        let mut watch = AgentWatch::new();
        let mut wait = event(HookKind::Wait, 1_000);
        wait.call_id = Some("call-1".to_string());
        assert!(watch.ingest(wait));
        assert_eq!(watch.operations(1_000)[0].state, CompanionState::Waiting);

        let mut resolve = event(HookKind::Resolve, 1_100);
        resolve.call_id = Some("call-1".to_string());
        assert!(watch.ingest(resolve));
        assert_eq!(watch.operations(1_100)[0].state, CompanionState::Running);
    }

    /// 同时有**两个**待确认项时，答掉一个**不能**提前回到运行中。
    ///
    /// 可证伪性：把 `Resolve` 改成无条件置 `Running` ⇒ 变红。
    #[test]
    fn one_of_two_pending_items_keeps_waiting() {
        let mut watch = AgentWatch::new();
        for (index, call) in ["c-1", "c-2"].iter().enumerate() {
            let mut wait = event(HookKind::Wait, 1_000 + index as i64);
            wait.call_id = Some((*call).to_string());
            watch.ingest(wait);
        }
        assert_eq!(watch.operations(1_000)[0].state, CompanionState::Waiting);

        let mut resolve = event(HookKind::Resolve, 1_100);
        resolve.call_id = Some("c-1".to_string());
        watch.ingest(resolve);
        assert_eq!(
            watch.operations(1_100)[0].state,
            CompanionState::Waiting,
            "还剩 c-2 没回答，必须仍是待确认"
        );
    }

    /// ★ 缺 `session_id` / 空来源 / 未知事件 ⇒ **一律拒绝且不改状态**。
    #[test]
    fn malformed_events_are_rejected() {
        let mut watch = AgentWatch::new();

        let mut no_session = event(HookKind::Start, 1_000);
        no_session.session_id = "   ".to_string();
        assert!(!watch.ingest(no_session));

        let mut no_source = event(HookKind::Start, 1_000);
        no_source.source = String::new();
        assert!(!watch.ingest(no_source));

        assert!(!watch.ingest(event(HookKind::Unknown, 1_000)));
        assert!(watch.is_empty(), "被拒绝的事件不得留下任何痕迹");
    }

    /// ★ **时间回退**的事件被丢弃（hook 乱序 / 重放）。
    ///
    /// 可证伪性：删掉 `ingest` 里的 `event.ts < existing.updated_at` 判断 ⇒ 变红
    /// （一条旧的 `Resolve` 会把已完成的一轮拉回运行中）。
    #[test]
    fn out_of_order_events_are_dropped() {
        let mut watch = AgentWatch::new();
        assert!(watch.ingest(event(HookKind::End, 5_000)));
        // 一条更早的 Step 迟到 ⇒ 必须被丢弃，终态不被改写
        assert!(!watch.ingest(event(HookKind::Step, 4_000)));
        assert_eq!(watch.operations(5_000)[0].state, CompanionState::Done);
    }

    /// ★ 终态**不被非 Start 事件唤醒**，但 `Start` 可以开新一轮。
    #[test]
    fn terminal_session_restarts_only_on_start() {
        let mut watch = AgentWatch::new();
        watch.ingest(event(HookKind::End, 1_000));
        assert!(
            !watch.ingest(event(HookKind::Step, 2_000)),
            "已结束的一轮不接受后续步骤"
        );
        assert_eq!(watch.operations(2_000)[0].state, CompanionState::Done);

        assert!(
            watch.ingest(event(HookKind::Start, 3_000)),
            "Start 必须能开新一轮"
        );
        assert_eq!(watch.operations(3_000)[0].state, CompanionState::Running);
    }

    /// ★ **已完成的会话超保留期后不再渲染**（惰性过期，不是后台清理）。
    ///
    /// 可证伪性：把 `operations` 里的 TTL 判断删掉 ⇒ 变红。
    #[test]
    fn completed_sessions_expire_from_the_view() {
        let mut watch = AgentWatch::new();
        watch.ingest(event(HookKind::End, 1_000));

        assert_eq!(
            watch.operations(1_000 + COMPLETED_TTL_MS).len(),
            1,
            "刚好到期仍应显示"
        );
        assert!(
            watch.operations(1_000 + COMPLETED_TTL_MS + 1).is_empty(),
            "超过保留期必须消失"
        );
        assert_eq!(watch.len(), 1, "惰性过期**不删数据**，只是不渲染");
    }

    /// ★ **静默的运行中会话**超期后不再渲染（不显示一个永远转圈的行）。
    #[test]
    fn silent_running_sessions_expire_from_the_view() {
        let mut watch = AgentWatch::new();
        watch.ingest(event(HookKind::Start, 1_000));
        assert_eq!(watch.operations(1_000).len(), 1);
        assert!(
            watch.operations(1_000 + STALE_TTL_MS + 1).is_empty(),
            "静默超过 STALE_TTL_ms 的运行中会话必须消失（本仓不做宿主进程探测，见 spike §2 Q2）"
        );
    }

    /// 同一条 `Wait` 重复到达 ⇒ 待确认项**不重复入队**（去重键 = `call_id`）。
    #[test]
    fn duplicate_wait_is_deduplicated() {
        let mut watch = AgentWatch::new();
        for ts in [1_000, 1_100, 1_200] {
            let mut wait = event(HookKind::Wait, ts);
            wait.call_id = Some("c-1".to_string());
            watch.ingest(wait);
        }
        // 答掉它 ⇒ 立刻回到运行中（说明「只有一个待确认项」）
        let mut resolve = event(HookKind::Resolve, 1_300);
        resolve.call_id = Some("c-1".to_string());
        watch.ingest(resolve);
        assert_eq!(watch.operations(1_300)[0].state, CompanionState::Running);
    }

    /// ★ 超过上限时**淘汰最旧的终态**，且淘汰是**确定性的**（同输入同结果）。
    #[test]
    fn sessions_are_capped_and_eviction_is_deterministic() {
        let mut watch = AgentWatch::new();
        for index in 0..(MAX_SESSIONS + 8) {
            let mut start = event(HookKind::Start, 1_000 + index as i64);
            start.session_id = format!("s-{index:03}");
            watch.ingest(start);
        }
        assert_eq!(watch.len(), MAX_SESSIONS);
        // 最早的那几条必须已经不在表里
        // ⚠️ 断言**具体的 id**，不是 `!= "workbuddy:s-000"` ——
        // 后者在来源常量改名后会**恒真**（永远绿），而这正是「断言失效」的典型形态。
        assert!(watch
            .operations(9_999)
            .iter()
            .all(|op| op.id != "workbuddy-cn:s-000"));
    }

    /// 来源逐字透传，且 `drop_source` 只清该来源。
    #[test]
    fn drop_source_only_clears_that_source() {
        let mut watch = AgentWatch::new();
        watch.ingest(event(HookKind::Start, 1_000));
        let mut other = event(HookKind::Start, 1_000);
        other.source = SOURCE_CODEX.to_string();
        other.session_id = "c-1".to_string();
        watch.ingest(other);
        assert_eq!(watch.len(), 2);

        assert_eq!(watch.drop_source(SOURCE_CODEX), 1);
        assert_eq!(watch.operations(1_000).len(), 1);
        assert_eq!(watch.operations(1_000)[0].source, SOURCE_WORKBUDDY_CN);
    }

    /// ★ 排序：`updated_at` **降序**；完全相同时按 id 升序（全字段确定性）。
    #[test]
    fn operations_are_sorted_newest_first() {
        let mut watch = AgentWatch::new();
        for (session, ts) in [("old", 1_000), ("mid", 2_000), ("new", 3_000)] {
            let mut start = event(HookKind::Start, ts);
            start.session_id = session.to_string();
            watch.ingest(start);
        }
        let ids: Vec<String> = watch
            .operations(3_000)
            .into_iter()
            .map(|op| op.id)
            .collect();
        assert_eq!(
            ids,
            vec!["workbuddy-cn:new", "workbuddy-cn:mid", "workbuddy-cn:old"],
            "最新的必须在最前"
        );
    }

    /// `hint` 随「现在」推进（同一快照在不同时刻给出不同年龄）。
    ///
    /// ⚠️ 只测**可达范围**：运行中的会话最多活 [`STALE_TTL_MS`]（45 分钟），
    /// 所以「小时 / 天」两档在 `operations` 这条路上**永远走不到** ——
    /// 它们由 [`relative_time_covers_every_bucket`] 直接测函数本身。
    #[test]
    fn hint_advances_with_now() {
        let mut watch = AgentWatch::new();
        watch.ingest(event(HookKind::Start, 0));
        assert_eq!(watch.operations(30_000)[0].hint, "刚刚");
        assert_eq!(watch.operations(5 * 60_000)[0].hint, "5 分钟前");
        assert_eq!(watch.operations(STALE_TTL_MS)[0].hint, "45 分钟前");
        assert!(
            watch.operations(STALE_TTL_MS + 1).is_empty(),
            "超期即不渲染 —— 这正是「小时」档在 operations 里不可达的原因"
        );
    }

    /// 相对时间的各档边界（纯函数，直接测）。
    #[test]
    fn relative_time_covers_every_bucket() {
        let minute = 60_000;
        assert_eq!(relative_time(0, 0), "刚刚");
        assert_eq!(relative_time(minute - 1, 0), "刚刚");
        assert_eq!(relative_time(minute, 0), "1 分钟前");
        assert_eq!(relative_time(59 * minute, 0), "59 分钟前");
        assert_eq!(relative_time(60 * minute, 0), "1 小时前");
        assert_eq!(relative_time(23 * 60 * minute, 0), "23 小时前");
        assert_eq!(relative_time(24 * 60 * minute, 0), "1 天前");
        assert_eq!(relative_time(72 * 60 * minute, 0), "3 天前");
        // 本机时钟回拨（`now < ts`）⇒ 钳到「刚刚」，绝不给出负数年龄。
        assert_eq!(relative_time(0, 5_000), "刚刚");
    }

    /// ★ 事件名 → 本仓词汇的映射（**唯一**允许看客户端事件名的地方）。
    #[test]
    fn client_event_names_map_to_our_vocabulary() {
        let payload = json!({});
        assert_eq!(
            HookKind::from_hook("SessionStart", "", &payload),
            HookKind::Start
        );
        assert_eq!(
            HookKind::from_hook("UserPromptSubmit", "", &payload),
            HookKind::Start
        );
        assert_eq!(
            HookKind::from_hook("PreToolUse", "Read", &payload),
            HookKind::Step
        );
        assert_eq!(
            HookKind::from_hook("PreToolUse", "mcp__wb__AskUserQuestion", &payload),
            HookKind::Wait,
            "提问类工具必须映射成待确认（含 mcp__ 前缀）"
        );
        assert_eq!(
            HookKind::from_hook("PreToolUse", "mcp__x__request_user_input_async", &payload),
            HookKind::Wait
        );
        assert_eq!(
            HookKind::from_hook("PostToolUse", "", &payload),
            HookKind::Resolve
        );
        assert_eq!(
            HookKind::from_hook("PostToolUseFailure", "", &payload),
            HookKind::Resolve
        );
        assert_eq!(
            HookKind::from_hook("PermissionRequest", "", &payload),
            HookKind::Wait
        );
        assert_eq!(HookKind::from_hook("Stop", "", &payload), HookKind::End);
        assert_eq!(
            HookKind::from_hook("SessionEnd", "", &payload),
            HookKind::End
        );
        assert_eq!(
            HookKind::from_hook("Interrupt", "", &payload),
            HookKind::Abort
        );
        assert_eq!(
            HookKind::from_hook("PreCompact", "", &payload),
            HookKind::Unknown
        );
        assert_eq!(
            HookKind::from_hook("SomethingElse", "", &payload),
            HookKind::Unknown
        );
    }

    /// `Notification` 的三条短路规则（含 `auth_success` 的**先排除**）。
    #[test]
    fn confirmation_notification_is_classified() {
        // 确认类
        assert!(is_confirmation_notification(
            &json!({"notification_type": "permission_request"})
        ));
        assert!(is_confirmation_notification(
            &json!({"message": "是否允许访问凭证？"})
        ));
        assert!(is_confirmation_notification(
            &json!({"message": "Allow file write"})
        ));
        // 非确认类
        assert!(
            !is_confirmation_notification(&json!({"notification_type": "auth_success"})),
            "auth_success 是纯提示，当成待确认会让窗口一直亮着"
        );
        assert!(!is_confirmation_notification(
            &json!({"message": "任务已开始"})
        ));
    }

    /// `parse_event`：真实形态的 payload 逐字段解析。
    #[test]
    fn parse_event_reads_real_payload_shape() {
        let payload = json!({
            "hook_event_name": "PreToolUse",
            "session_id": "abc-123",
            "tool_name": "AskUserQuestion",
            "tool_use_id": "call-9",
            "session_title": "重构会话管线",
            "timestamp": 1_700_000_000_000i64,
        });
        let event = parse_event(SOURCE_WORKBUDDY_CN, &payload, 0).expect("必须解析成功");
        assert_eq!(event.source, SOURCE_WORKBUDDY_CN);
        assert_eq!(event.session_id, "abc-123");
        assert_eq!(event.kind, HookKind::Wait);
        assert_eq!(event.call_id.as_deref(), Some("call-9"));
        assert_eq!(event.title.as_deref(), Some("重构会话管线"));
        assert_eq!(event.ts, 1_700_000_000_000);
    }

    /// ★ **秒级**时间戳被换算成毫秒（客户端两代实现都用数字，但单位不一致）。
    #[test]
    fn parse_event_normalizes_second_precision_timestamps() {
        let payload =
            json!({"hook_event_name": "Stop", "session_id": "s", "timestamp": 1_700_000_000i64});
        assert_eq!(
            parse_event(SOURCE_CODEX, &payload, 0).unwrap().ts,
            1_700_000_000_000
        );
    }

    /// 缺 `session_id` ⇒ `None`（宿主应回 2xx 但什么都不做）。
    #[test]
    fn parse_event_returns_none_without_session_id() {
        assert!(parse_event(SOURCE_WORKBUDDY_CN, &json!({"hook_event_name": "Stop"}), 0).is_none());
        assert!(parse_event(
            SOURCE_WORKBUDDY_CN,
            &json!({"hook_event_name": "Stop", "session_id": "  "}),
            0
        )
        .is_none());
    }

    /// payload 里的 `agent_source` **优先于**默认来源（一个接收端服务多个客户端）。
    #[test]
    fn parse_event_prefers_payload_source() {
        let payload =
            json!({"agent_source": "codex", "session_id": "s", "hook_event_name": "Stop"});
        assert_eq!(
            parse_event(SOURCE_WORKBUDDY_CN, &payload, 0)
                .unwrap()
                .source,
            "codex"
        );
    }

    /// 中文标题按**字符**截断，不切出半个字。
    #[test]
    fn truncate_chars_never_splits_a_character() {
        assert_eq!(truncate_chars("中文标题", 10), "中文标题");
        let truncated = truncate_chars(&"会".repeat(300), TITLE_MAX_CHARS);
        assert_eq!(truncated.chars().count(), TITLE_MAX_CHARS);
    }

    /// 端到端：一串真实顺序的事件 → 窗口内容（含 id 前缀与来源回读）。
    #[test]
    fn happy_path_produces_a_renderable_row() {
        let mut watch = AgentWatch::new();
        let payloads = [
            json!({"hook_event_name": "SessionStart", "session_id": "s-9", "timestamp": 1_000i64}),
            json!({"hook_event_name": "UserPromptSubmit", "session_id": "s-9", "session_title": "修悬浮窗", "timestamp": 1_100i64}),
            json!({"hook_event_name": "PreToolUse", "session_id": "s-9", "tool_name": "Read", "tool_use_id": "c1", "timestamp": 1_200i64}),
            json!({"hook_event_name": "Stop", "session_id": "s-9", "timestamp": 1_300i64}),
        ];
        for payload in payloads {
            let event = parse_event(SOURCE_WORKBUDDY_CN, &payload, 0).expect("解析");
            assert!(watch.ingest(event), "事件必须被接受：{payload}");
        }
        let ops = watch.operations(1_300);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].id, "workbuddy-cn:s-9");
        assert_eq!(ops[0].source, "workbuddy-cn");
        assert_eq!(ops[0].title, "修悬浮窗");
        assert_eq!(ops[0].state, CompanionState::Done);
    }

    /// ★ **本工具自己的操作走同一张表**（签到这类没有客户端 hook 的长操作）。
    ///
    /// 依据：`P9-ARCH §1.3` 把数据源写成「`switch-progress` / `checkin-progress` /
    /// `trae-checkin-progress`」三个 —— 后两个在实现里长期缺席。本用例把
    /// 「本工具的操作也能进窗口」这条性质钉住。
    ///
    /// 可证伪性：把 `local_event` 的 `source` 改成客户端来源 ⇒ 下面第一条来源断言变红
    /// （窗口里就分不清「客户端里的任务」与「本工具在做的事」）。
    #[test]
    fn local_tasks_share_the_same_table() {
        let mut watch = AgentWatch::new();

        assert!(watch.ingest(local_event(
            "checkin",
            "Trae 签到（共 3 个账号）",
            CompanionState::Running,
            1_000
        )));
        let ops = watch.operations(1_000);
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].id, format!("{SOURCE_THIS_TOOL}:checkin"));
        assert_eq!(ops[0].source, SOURCE_THIS_TOOL);
        assert_eq!(ops[0].state, CompanionState::Running);
        assert_eq!(ops[0].title, "Trae 签到（共 3 个账号）");

        // 进度更新：同一个 task_id 再写「运行中」⇒ 仍是一行，标题被更新
        assert!(watch.ingest(local_event(
            "checkin",
            "Trae 签到：第 2 个",
            CompanionState::Running,
            1_100
        )));
        let ops = watch.operations(1_100);
        assert_eq!(ops.len(), 1, "同一个 task_id 不得叠出第二行");
        assert_eq!(ops[0].title, "Trae 签到：第 2 个");

        // 结束
        assert!(watch.ingest(local_event(
            "checkin",
            "Trae 签到：成功 3",
            CompanionState::Done,
            1_200
        )));
        assert_eq!(watch.operations(1_200)[0].state, CompanionState::Done);
        assert_eq!(watch.operations(1_200)[0].title, "Trae 签到：成功 3");

        // 终态之后：非 Start 被拒（幂等），Start 开新一轮
        assert!(!watch.ingest(local_event(
            "checkin",
            "重复完成",
            CompanionState::Done,
            1_300
        )));
        assert!(watch.ingest(local_event(
            "checkin",
            "第二轮",
            CompanionState::Running,
            1_400
        )));
        assert_eq!(watch.operations(1_400)[0].state, CompanionState::Running);
    }

    /// ★ 来源注册表：**每个常量都在表里、且表里没有重复**。
    ///
    /// 可证伪性：往 [`HOOK_SOURCES`] 里塞一个重复项 / 删掉一条 ⇒ 变红。
    /// 这条钉的是「加客户端时忘了进表」——那会让新来源在 `--source` 白名单里被拒，
    /// 症状是**静默回落成 WorkBuddy**（见 [`canonical_source`] 的调用方）。
    #[test]
    fn hook_sources_cover_every_constant_without_duplicates() {
        for constant in [
            SOURCE_WORKBUDDY_CN,
            SOURCE_WORKBUDDY_GLOBAL,
            SOURCE_CLAUDE,
            SOURCE_CODEBUDDY,
            SOURCE_CODEX,
            SOURCE_TRAE_CN,
            SOURCE_TRAE_GLOBAL,
            SOURCE_QODER_CN,
            SOURCE_QODER_GLOBAL,
        ] {
            assert!(
                HOOK_SOURCES.contains(&constant),
                "来源常量 {constant} 不在 HOOK_SOURCES 表里 —— 它会被 --source 白名单拒掉"
            );
        }
        let mut sorted = HOOK_SOURCES.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            HOOK_SOURCES.len(),
            "来源表里有重复项：{HOOK_SOURCES:?}"
        );
    }

    /// ★ 来源 id **不带空格**（它会被嵌进客户端配置的命令串，带空格会被按空格拆开）。
    #[test]
    fn hook_source_ids_are_shell_safe() {
        for id in HOOK_SOURCES {
            assert!(
                !id.contains(char::is_whitespace),
                "来源 id 不得含空白：{id:?}"
            );
            assert_eq!(id.trim(), *id, "来源 id 不得有首尾空白：{id:?}");
        }
    }

    /// ★ **整段精确匹配**：前缀 / 后缀 / 含子串的形态一律不认。
    ///
    /// 可证伪性：把 `canonical_source` 换成 `HOOK_SOURCES.iter().find(|id| raw.contains(**id))`
    /// ⇒ 下面每一条「必须 None」的断言变红（`trae-cn-x` 会被认成 `trae-cn`）。
    #[test]
    fn canonical_source_is_exact_match_only() {
        for id in HOOK_SOURCES {
            assert_eq!(canonical_source(id), Some(*id), "表内 id 必须自认：{id}");
        }
        // ⚠️ 输入里必须含「比基准长一截」的形状，否则前缀实现与精确实现在现有输入集上**等价**
        //    （本仓的既有教训：断言会恒绿）。
        for unknown in [
            "codex-desktop",
            "trae-cn-x",
            "wb-cn-x",
            "claude-code",
            "qoder-cn-cli",
            "codebuddy-ide",
            "trae",
            "qoder",
            "",
            "   ",
        ] {
            assert_eq!(
                canonical_source(unknown),
                None,
                "{unknown:?} 不该被认成任何来源"
            );
        }
    }

    /// 首尾空白可容错（hook 命令的参数可能被加引号），中间空白不行。
    #[test]
    fn canonical_source_trims_outer_whitespace_only() {
        assert_eq!(canonical_source("  trae-cn  "), Some(SOURCE_TRAE_CN));
        assert_eq!(canonical_source("trae cn"), None);
    }

    /// 四态 → hook 种类的映射，以及**往返一致**（写进去什么状态，读出来还是那个状态）。
    #[test]
    fn local_state_maps_to_the_right_hook_kind() {
        let cases = [
            (CompanionState::Running, HookKind::Start),
            (CompanionState::Waiting, HookKind::Wait),
            (CompanionState::Done, HookKind::End),
            (CompanionState::Failed, HookKind::Abort),
        ];
        for (state, expect_kind) in cases {
            let event = local_event("t", "标题", state, 1_000);
            assert_eq!(event.kind, expect_kind, "状态 {state:?} 的事件种类不对");
            let mut watch = AgentWatch::new();
            assert!(watch.ingest(event));
            assert_eq!(
                watch.operations(1_000)[0].state,
                state,
                "状态 {state:?} 经状态机往返后必须还是它"
            );
        }
    }
}
