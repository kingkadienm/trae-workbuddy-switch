//! 成长任务数据 + 读写 API（对照参考实现 `internal/panel/tasks.go` + `internal/upstream/tasks.go`）。
//!
//! 端点（全部 CN 专有；Global 无成长任务体系，入口处门控）：
//! - 列表 / 接受：chat 域（`copilot.tencent.com`）`/v2/activity/growth/tasks[/accept]`
//! - 小程序口径：同路径 + 头 `X-Client-Platform: miniprogram`（mp 列表是默认列表的超集，合并按 code 去重）
//! - 领奖：web 域（`www.workbuddy.cn`）`/activity/growth/tasks/{code}/claim`；mp 任务先打 chat 域
//!   + mp 头，HTTP 400 时降级 web 域（同 `cat.rs` 的降级口径）
//!
//! 语义要点（与 panel 一致）：accept 是「报名」不产生进度；claim 仅在达标后可领，
//! 重复领上游幂等返回 0 奖励（不是错误）。

use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::modules::config::authed_json_request_for;
use crate::modules::region::{region_spec, Region};

use super::events::{
    chat_request_send_event, expert_list_body, real_chat_body_model, server_request_id_ok,
};

// ---------------------------------------------------------------------------
// 端点常量
// ---------------------------------------------------------------------------

/// 任务列表（chat 域 growth 接口）。
const TASKS_PATH: &str = "/v2/activity/growth/tasks";
/// 接受任务。
const TASKS_ACCEPT_PATH: &str = "/v2/activity/growth/tasks/accept";
/// 领奖（任务码在路径里；web 域与 mp chat 域各一份）。
const CLAIM_PATH_FMT: &str = "/activity/growth/tasks/{code}/claim";
/// 领奖 web 域基址（CN 硬编码，同 `cat.rs` 的 CLAIM_WEB_BASE 口径）。
pub const CLAIM_WEB_BASE: &str = "https://www.workbuddy.cn";
/// 活跃上报（billing 域）。
const REPORT_PATH: &str = "/v2/report";
/// 专家市场列表（chat 域）。
const EXPERT_LIST_PATH: &str = "/portal/operation-platform/market/expert/list";
/// 真实对话（chat 域 SSE）。
const CHAT_PATH: &str = "/v2/chat/completions";
/// 领养第一只 Buddy。
const BUDDY_FIRST_PATH: &str = "/activity/growth/buddy/first";
/// 同意 Buddy 协议。
const BUDDY_AGREEMENT_PATH: &str = "/activity/growth/buddy/agreement";
/// 外观主题设置。
const APPEARANCE_SET_PATH: &str = "/v2/user-asset/appearance/set";

/// accept 批量分片大小（panel 口径：保守每批 20）。
const ACCEPT_BATCH: usize = 20;
/// 批间节流（对齐脚本 1.05s 口径，避免上游风控）。
pub const ACCEPT_BATCH_GAP: Duration = Duration::from_millis(1050);
/// 上报动作之间的间隔（防频控）。
pub const ACTION_GAP: Duration = Duration::from_millis(1050);
/// 达标回读的有界轮询（上游异步计分：事件上报后数秒才刷新进度）。
pub const CLAIM_POLL_ATTEMPTS: u32 = 4;
pub const CLAIM_POLL_GAP: Duration = Duration::from_secs(3);

// ---------------------------------------------------------------------------
// 任务模型
// ---------------------------------------------------------------------------

/// 单个成长任务（字段名与 panel `Task` 对齐；序列化给前端用 camelCase）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrowthTask {
    pub task_code: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub task_desc: String,
    /// 奖励积分（上游 `reward_credit`）。
    #[serde(default)]
    pub credit: i64,
    /// 奖励能量（上游 `reward_energy`）。
    #[serde(default)]
    pub energy: i64,
    #[serde(default)]
    pub has_reward: bool,
    #[serde(default)]
    pub reward_buddy: bool,
    #[serde(default)]
    pub task_type: String,
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub jump_url: String,
    /// 上游标记未解锁（locked 期间不该被自动化尝试）。
    #[serde(default)]
    pub locked: bool,
    /// 目标次数（恒输出：0 是有效进度值）。
    #[serde(default)]
    pub target: i64,
    /// 当前进度。
    #[serde(default)]
    pub current: i64,
    #[serde(default)]
    pub accept_status: String,
    #[serde(default)]
    pub status: String,
    /// 进度达标且未领取（本地推算）。
    #[serde(default)]
    pub claimable: bool,
    /// 已领取（`accept_status == "claimed"`）。
    #[serde(default)]
    pub claimed: bool,
}

impl GrowthTask {
    pub fn claimed(&self) -> bool {
        self.claimed
    }
}

/// 小程序口径专属任务码（默认列表不出现，accept/claim 要求 mp 头）。
pub const MP_TASK_CODES: &[&str] = &[
    "school_season",
    "Sequential_Tasks_1",
    "Sequential_Tasks_2",
    "Sequential_Tasks_3",
    "Sequential_Tasks_4",
    "Sequential_Tasks_5",
    "Sequential_Tasks_6",
    "Sequential_Tasks_7",
];

pub fn is_mp_task_code(code: &str) -> bool {
    MP_TASK_CODES.contains(&code.trim())
}

fn mp_extra() -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("X-Client-Platform".into(), "miniprogram".into());
    m
}

fn web_claim_extra(_code: &str) -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("Origin".into(), CLAIM_WEB_BASE.to_string());
    m.insert(
        "Referer".into(),
        format!("{CLAIM_WEB_BASE}/profile/growth-center"),
    );
    m.insert("x-client-platform".into(), "web".into());
    m
}

fn escaped(code: &str) -> String {
    code.replace('%', "%25")
        .replace('/', "%2F")
        .replace('\\', "%5C")
}

// ---------------------------------------------------------------------------
// 纯解析（对照 Go parseGrowthTasks）
// ---------------------------------------------------------------------------

/// 解析任务列表响应（`data.tasks[]`，progress 可能是 `{current,target}` 对象覆盖平铺字段）。
pub fn parse_task_items(data: &Value) -> Vec<GrowthTask> {
    let Some(arr) = data.pointer("/data/tasks").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter().filter_map(parse_task_item).collect()
}

/// 解析单个任务条目（字段缺失给默认值；progress 对象优先）。
pub fn parse_task_item(item: &Value) -> Option<GrowthTask> {
    let get = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let claimed = get("accept_status") == "claimed";
    let mut current = item.get("current").and_then(Value::as_i64).unwrap_or(0);
    let mut target = item.get("target").and_then(Value::as_i64).unwrap_or(0);
    if let Some(progress) = item.get("progress") {
        let (pc, pt) = (
            progress.get("current").and_then(Value::as_i64),
            progress.get("target").and_then(Value::as_i64),
        );
        if pt.unwrap_or(0) > 0 || pc.unwrap_or(0) > 0 {
            current = pc.unwrap_or(current);
            target = pt.unwrap_or(target);
        }
    }
    Some(GrowthTask {
        task_code: get("task_code"),
        title: get("title"),
        description: get("description"),
        task_desc: get("task_desc"),
        credit: item.get("reward_credit").and_then(Value::as_i64).unwrap_or(0),
        energy: item.get("reward_energy").and_then(Value::as_i64).unwrap_or(0),
        has_reward: item.get("has_reward").and_then(Value::as_bool).unwrap_or(false),
        reward_buddy: item.get("reward_buddy").and_then(Value::as_bool).unwrap_or(false),
        task_type: get("task_type"),
        tag: get("tag"),
        jump_url: get("jump_url"),
        locked: item.get("locked").and_then(Value::as_bool).unwrap_or(false),
        target,
        current,
        accept_status: get("accept_status"),
        status: get("status"),
        claimable: !claimed && target > 0 && current >= target,
        claimed,
    })
}

/// 任务是否「未完成且可自动化」（队列判据；与 panel `growthPending` 一致）：
/// - claimed / locked 不出待办（Sequential 族每日零点解锁一环，locked 期间 accept 不落账）
/// - 达标未领（current>=target）也不出待办（panel 口径）
/// - 无对应自动动作的不出待办
pub fn is_growth_pending(t: &GrowthTask, automatable: bool) -> bool {
    if t.claimed || t.locked || !automatable {
        return false;
    }
    if t.target > 0 && t.current >= t.target {
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// HTTP 取值点（可注入；保状态码，400 降级 / 401 语义都靠它）
// ---------------------------------------------------------------------------

/// 单次请求返回值：`(HTTP 状态码, 响应体 JSON)`（保留状态码供 400 降级判定）。
pub type GrowthResponse = (u16, Value);

/// 成长任务域 HTTP 取值点（依赖注入缝；同 `cat::CatIo` 模式）。
pub trait GrowthIo: Send + Sync {
    /// 一次 JSON 请求（生产实现走 `authed_json_request_for`，401 自动刷新）。
    fn json<'a>(
        &'a self,
        region: Region,
        url: &'a str,
        method: &'a str,
        body: Option<Value>,
        account: &'a Value,
        extra: &'a HashMap<String, String>,
    ) -> Pin<Box<dyn Future<Output = GrowthResponse> + Send + 'a>>;

    /// 真实对话（SSE）：POST chat 域 `/v2/chat/completions`，返回
    /// `(conversation_id, 服务端 requestId)`。JOIN 类事件必须用服务端 id。
    fn sse_chat<'a>(
        &'a self,
        region: Region,
        account: &'a Value,
        expert_id: &'a str,
        body: &'a Value,
    ) -> Pin<Box<dyn Future<Output = Result<(String, String), String>> + Send + 'a>>;
}

/// 生产实现：真实 HTTP。
#[derive(Clone, Copy)]
pub struct RealGrowthIo;

impl GrowthIo for RealGrowthIo {
    fn json<'a>(
        &'a self,
        region: Region,
        url: &'a str,
        method: &'a str,
        body: Option<Value>,
        account: &'a Value,
        extra: &'a HashMap<String, String>,
    ) -> Pin<Box<dyn Future<Output = GrowthResponse> + Send + 'a>> {
        Box::pin(async move {
            authed_json_request_for(region, url, method, body, account, extra).await
        })
    }

    fn sse_chat<'a>(
        &'a self,
        region: Region,
        account: &'a Value,
        expert_id: &'a str,
        body: &'a Value,
    ) -> Pin<Box<dyn Future<Output = Result<(String, String), String>> + Send + 'a>> {
        Box::pin(async move {
            real_sse_chat(region, account, expert_id, body).await
        })
    }
}

/// 真实 SSE 对话：发请求（读干响应体），从文本里提取服务端 requestId。
///
/// 走 `http_request_raw` 拿**全量文本**（`authed_json_request_for` 会把非 JSON 体截断到
/// 500 字符，SSE 首帧里的 requestId 可能取不到）；401 时复刻其一次刷新重试。
async fn real_sse_chat(
    region: Region,
    account: &Value,
    expert_id: &str,
    body: &Value,
) -> Result<(String, String), String> {
    use crate::modules::account::{build_auth_headers, envelope_token_error};
    use crate::modules::config::http_request_raw;

    if let Some(err) = envelope_token_error(account) {
        return Err(err);
    }

    let sse_headers = |account: &Value, conversation_id: &str, now_ms: i64| {
        let mut headers = build_auth_headers(account);
        headers.insert("User-Agent".into(), "WorkBuddy/5.5.6 WorkBuddy/5.5.6 CLI/2.137.1".into());
        headers.insert("X-Domain".into(), region_spec(region).chat_base.to_string());
        headers.insert("X-Product".into(), "SaaS".into());
        headers.insert("Accept".into(), "text/event-stream".into());
        headers.insert("X-Conversation-ID".into(), conversation_id.to_string());
        headers.insert("X-Request-ID".into(), now_ms.to_string());
        headers.insert("X-Agent-Intent".into(), "craft".into());
        headers.insert("X-Agent-Type".into(), "main".into());
        if !expert_id.is_empty() {
            headers.insert("X-Expert-Id".into(), expert_id.to_string());
        }
        headers
    };

    let url = format!("{}{CHAT_PATH}", region_spec(region).chat_base);
    let conversation_id = format!("buddyswitch-conv-{}", uuid::Uuid::new_v4());
    let headers = sse_headers(account, &conversation_id, crate::modules::config::now_ms());
    let (status, _h, text) = http_request_raw(&url, "POST", Some(body.clone()), Some(&headers), None, true).await;

    if status == 401 {
        let has_refresh = !account
            .get("refresh_token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty();
        if has_refresh {
            let refreshed = crate::modules::refresh::refresh_account_token_for(region, account.clone()).await;
            let headers = sse_headers(&refreshed, &conversation_id, crate::modules::config::now_ms());
            let (status, _h, text) =
                http_request_raw(&url, "POST", Some(body.clone()), Some(&headers), None, true).await;
            if status != 200 {
                return Err(resp_err_text(status, &text));
            }
            let server_id = sse_server_request_id(&text).ok_or_else(|| "SSE 中未找到服务端 requestId".to_string())?;
            return Ok((conversation_id, server_id));
        }
        return Err(resp_err_text(status, &text));
    }
    if status != 200 {
        return Err(resp_err_text(status, &text));
    }
    let server_id = sse_server_request_id(&text)
        .ok_or_else(|| "SSE 中未找到服务端 requestId".to_string())?;
    Ok((conversation_id, server_id))
}

/// 从 SSE 文本里提取服务端 requestId（第一个 `"id":"..."` 且匹配 `(cmb-)?[0-9a-f]{32}`）。
fn sse_server_request_id(text: &str) -> Option<String> {
    let i = text.find("\"id\":\"")?;
    let rest = &text[i + 6..];
    let end = rest.find('"').unwrap_or(0);
    let id = &rest[..end];
    if server_request_id_ok(id) {
        Some(id.to_string())
    } else {
        None
    }
}

fn resp_err_text(status: u16, text: &str) -> String {
    let snippet: String = text.chars().take(160).collect();
    format!("对话失败 http={status}: {snippet}")
}

// ---------------------------------------------------------------------------
// 账号字段取值
// ---------------------------------------------------------------------------

pub fn acc_uid(account: &Value) -> String {
    account
        .get("uid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}
pub fn acc_nickname(account: &Value) -> String {
    account
        .get("nickname")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}
pub fn acc_enterprise_id(account: &Value) -> String {
    account
        .get("enterpriseId")
        .and_then(Value::as_str)
        .or_else(|| account.get("enterprise_id").and_then(Value::as_str))
        .unwrap_or("")
        .to_string()
}

// ---------------------------------------------------------------------------
// 端点辅助
// ---------------------------------------------------------------------------

/// 拉任务列表（`mp=true` 走小程序口径头；mp 列表是默认列表的超集）。
pub async fn list_tasks(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    mp: bool,
) -> Result<Vec<GrowthTask>, String> {
    get_tasks(io, region, account, mp).await
}

async fn get_tasks(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    mp: bool,
) -> Result<Vec<GrowthTask>, String> {
    let url = format!("{}{TASKS_PATH}", region_spec(region).chat_base);
    let extra = if mp { mp_extra() } else { HashMap::new() };
    let (status, resp) = io.json(region, &url, "GET", None, account, &extra).await;
    if status != 200 || resp.get("code").and_then(Value::as_i64) == Some(401) {
        return Err(resp_to_err(status, &resp));
    }
    Ok(parse_task_items(&resp))
}

fn resp_to_err(status: u16, resp: &Value) -> String {
    let msg = resp
        .get("message")
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| resp.get("msg").and_then(Value::as_str).map(String::from));
    match msg {
        Some(m) => format!("http {status}: {m}"),
        None => format!("http {status}"),
    }
}

/// 拉任务列表并按 code 定位；未找到返回 None（不视为错误）。
/// 双口径：mp 专属码在默认列表查不到时回落 mp 列表（仅对已登记 mp 码）。
pub async fn task_by_code(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    code: &str,
) -> Result<Option<GrowthTask>, String> {
    let tasks = get_tasks(io, region, account, false).await?;
    if let Some(t) = tasks.iter().find(|t| t.task_code == code) {
        return Ok(Some(t.clone()));
    }
    if is_mp_task_code(code) {
        return task_by_code_mp(io, region, account, code).await;
    }
    Ok(None)
}

/// 小程序口径定位任务。
pub async fn task_by_code_mp(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    code: &str,
) -> Result<Option<GrowthTask>, String> {
    let tasks = get_tasks(io, region, account, true).await?;
    Ok(tasks.into_iter().find(|t| t.task_code == code))
}

/// 达标回读：未达标时有界轮询等待（上游异步计分）。
pub async fn task_by_code_waiting(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    code: &str,
) -> Result<Option<GrowthTask>, String> {
    let mut current = task_by_code(io, region, account, code).await?;
    if matches!(
        &current,
        Some(t) if t.claimed || t.claimable
    ) {
        return Ok(current);
    }
    for _ in 0..CLAIM_POLL_ATTEMPTS - 1 {
        tokio::time::sleep(CLAIM_POLL_GAP).await;
        if let Ok(Some(next)) = task_by_code(io, region, account, code).await {
            if next.claimed || next.claimable {
                current = Some(next);
                break;
            }
            current = Some(next);
        }
    }
    Ok(current)
}

// ---------------------------------------------------------------------------
// 接受
// ---------------------------------------------------------------------------

/// 接受任务（报名；幂等）。
pub async fn accept_tasks(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    codes: &[String],
) -> Result<(), String> {
    accept_tasks_with(io, region, account, codes, false).await
}

/// 接受小程序口径任务（mp 头；缺头实测 task not found）。
pub async fn accept_tasks_mp(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    codes: &[String],
) -> Result<(), String> {
    accept_tasks_with(io, region, account, codes, true).await
}

async fn accept_tasks_with(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    codes: &[String],
    mp: bool,
) -> Result<(), String> {
    if codes.is_empty() {
        return Ok(());
    }
    let url = format!("{}{TASKS_ACCEPT_PATH}", region_spec(region).chat_base);
    let extra = if mp { mp_extra() } else { HashMap::new() };
    for chunk in codes.chunks(ACCEPT_BATCH) {
        let body = json!({"task_codes": chunk});
        let (status, resp) = io.json(region, &url, "POST", Some(body), account, &extra).await;
        if status != 200 {
            return Err(resp_to_err(status, &resp));
        }
        tokio::time::sleep(ACCEPT_BATCH_GAP).await;
    }
    Ok(())
}

/// 接受该账号全部尚未接受的任务（默认 + mp 两轮；与 panel `taskAcceptAll` 同口径）。
pub async fn accept_all(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
) -> Value {
    let pending = |tasks: &Vec<GrowthTask>| -> Vec<String> {
        tasks
            .iter()
            .filter(|t| {
                !t.claimed
                    && !t.locked
                    && t.accept_status != "accepted"
                    && t.accept_status != "completed"
            })
            .map(|t| t.task_code.clone())
            .collect()
    };

    let mut accepted = 0usize;
    let mut failed: Vec<String> = Vec::new();

    if let Ok(tasks) = get_tasks(io, region, account, false).await {
        let codes = pending(&tasks);
        match accept_tasks(io, region, account, &codes).await {
            Ok(()) => accepted += codes.len(),
            Err(e) => {
                failed.extend(codes);
                eprintln!("[growth] accept_all: {e}");
            }
        }
    }
    if let Ok(tasks) = get_tasks(io, region, account, true).await {
        let codes = pending(&tasks);
        if !codes.is_empty() {
            match accept_tasks_mp(io, region, account, &codes).await {
                Ok(()) => accepted += codes.len(),
                Err(e) => {
                    failed.extend(codes);
                    eprintln!("[growth] accept_all(mp): {e}");
                }
            }
        }
    }

    let mut out = json!({"accepted": accepted, "failed": failed});
    if !failed.is_empty() {
        out["message"] = json!("部分任务接受失败（上游拒绝），可重试");
    }
    out
}

// ---------------------------------------------------------------------------
// 领奖
// ---------------------------------------------------------------------------

/// 领取任务奖励。返回 `{credit, energy, already_claimed}`；失败时 Err(文案)。
pub async fn claim_task(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    code: &str,
) -> Result<Value, String> {
    let path = CLAIM_PATH_FMT.replace("{code}", &escaped(code));
    let (credit, energy, already) = if is_mp_task_code(code) {
        // mp 任务：chat 域 + mp 头；400 时降级 web 域。
        let url = format!("{}{path}", region_spec(region).chat_base);
        let (status, resp) = io
            .json(region, &url, "POST", None, account, &mp_extra())
            .await;
        if status == 400 {
            claim_web(io, region, account, code).await?
        } else {
            parse_claim(status, &resp)?
        }
    } else {
        claim_web(io, region, account, code).await?
    };
    if credit == 0 && energy == 0 && !already {
        // 0+0 且非 already_claimed：视同「已领取过」而不是失败（与 panel 口径一致）。
        return Ok(json!({"credit": 0, "energy": 0, "already_claimed": true}));
    }
    Ok(json!({"credit": credit, "energy": energy, "already_claimed": already}))
}

async fn claim_web(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    code: &str,
) -> Result<(i64, i64, bool), String> {
    let path = CLAIM_PATH_FMT.replace("{code}", &escaped(code));
    let url = format!("{CLAIM_WEB_BASE}{path}");
    let (status, resp) = io
        .json(region, &url, "POST", None, account, &web_claim_extra(code))
        .await;
    parse_claim(status, &resp)
}

fn parse_claim(status: u16, resp: &Value) -> Result<(i64, i64, bool), String> {
    if status != 200 {
        return Err(resp_to_err(status, resp));
    }
    let data = resp.get("data").unwrap_or(resp);
    let already = data.get("already_claimed").and_then(Value::as_bool).unwrap_or(false);
    let credit = data.get("credit").and_then(Value::as_i64).unwrap_or(0);
    let energy = data.get("energy").and_then(Value::as_i64).unwrap_or(0);
    Ok((credit, energy, already))
}

// ---------------------------------------------------------------------------
// 动作组用到的上游原子能力（事件/对话/市场/领养/外观/上报）
// ---------------------------------------------------------------------------

/// 上报一批桌面指纹事件（chat 域 `/v2/report`，body = 事件数组）。
pub async fn report_desktop_events(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    events: Value,
) -> Result<(), String> {
    let url = format!("{}{REPORT_PATH}", region_spec(region).chat_base);
    let (status, resp) = io.json(region, &url, "POST", Some(events), account, &HashMap::new()).await;
    if status != 200 {
        return Err(resp_to_err(status, &resp));
    }
    Ok(())
}

/// 上报 mp 指纹事件（billing 域 `/v2/report` + mp 产品头；对照 Go ReportMPEvent）。
pub async fn report_mp_events(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    events: Value,
) -> Result<(), String> {
    let url = format!("{}{REPORT_PATH}", region_spec(region).billing_base);
    let mut extra = mp_extra();
    extra.insert("X-Client-Product".into(), "workbuddy-mp".into());
    extra.insert("X-Client-Version".into(), "2.4.0".into());
    extra.insert("X-Platform".into(), "wechatmp".into());
    let (status, resp) = io.json(region, &url, "POST", Some(events), account, &extra).await;
    if status != 200 {
        return Err(resp_to_err(status, &resp));
    }
    Ok(())
}

/// 上报一条 billing 域 `chat_request_send`（活跃/对话补报；userId 必填）。
pub async fn report_chat_event(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    cid: &str,
    request_id: &str,
    model_id: &str,
    model_name: &str,
) -> Result<(), String> {
    let now_ms = crate::modules::config::now_ms();
    let event =
        chat_request_send_event(&acc_uid(account), cid, request_id, model_id, model_name, now_ms);
    let url = format!("{}{REPORT_PATH}", region_spec(region).billing_base);
    let (status, resp) = io
        .json(region, &url, "POST", Some(json!([event])), account, &HashMap::new())
        .await;
    if status != 200 {
        return Err(resp_to_err(status, &resp));
    }
    Ok(())
}

/// 上报单条 web 域事件（Library_read；billing 域 `/v2/report` + web 头）。
pub async fn report_web_event(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    event: Value,
) -> Result<(), String> {
    let url = format!("{}{REPORT_PATH}", region_spec(region).billing_base);
    let mut extra = HashMap::new();
    extra.insert("x-client-platform".into(), "web".into());
    let (status, resp) = io
        .json(region, &url, "POST", Some(json!([event])), account, &extra)
        .await;
    if status != 200 {
        return Err(resp_to_err(status, &resp));
    }
    Ok(())
}

/// 拉取专家市场真实专家列表（expert_actual_use 判据要求 id 真实存在）。
pub async fn market_expert_list(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    expert_type: &str,
) -> Result<Vec<super::events::MarketExpert>, String> {
    let url = format!("{}{EXPERT_LIST_PATH}", region_spec(region).chat_base);
    let (status, resp) = io
        .json(region, &url, "POST", Some(expert_list_body(expert_type)), account, &HashMap::new())
        .await;
    if status != 200 {
        return Err(resp_to_err(status, &resp));
    }
    Ok(super::events::MarketExpert::from_json(
        resp.get("data").unwrap_or(&resp),
    ))
}

/// 真实对话一次（可带专家 id），返回 (conversation_id, 服务端 requestId)。
pub async fn real_chat(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    expert_id: &str,
) -> Result<(String, String), String> {
    io.sse_chat(region, account, expert_id, &real_chat_body_model("fast-model"))
        .await
}

/// 指定模型的真实对话（Model_chat / 夜猫子夜间补足）。
pub async fn real_chat_model(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    model: &str,
) -> Result<(String, String), String> {
    io.sse_chat(region, account, "", &real_chat_body_model(model))
        .await
}

/// 领养第一只 Buddy（前置：同意协议）。
/// 返回：`Ok(Some(()))` 领养成功；`Ok(None)` 门槛未过（400 + first_buddy 关键词，
/// 正常态不重试，同 panel IsBuddyTaskIncomplete）；`Err` 其它失败。
pub async fn buddy_first(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
) -> Result<Option<()>, String> {
    let base = region_spec(region).chat_base;
    let agreement_url = format!("{base}{BUDDY_AGREEMENT_PATH}");
    let (status, resp) = io
        .json(
            region,
            &agreement_url,
            "POST",
            Some(json!({"agree": true})),
            account,
            &HashMap::new(),
        )
        .await;
    if status != 200 {
        return Err(resp_to_err(status, &resp));
    }
    let first_url = format!("{base}{BUDDY_FIRST_PATH}");
    let (status, resp) = io
        .json(region, &first_url, "POST", Some(json!({})), account, &HashMap::new())
        .await;
    if status != 200 {
        let text = format!("{status} {}", resp.to_string());
        let lowered = text.to_lowercase();
        if status == 400 && lowered.contains("first_buddy") {
            return Ok(None); // 门槛未过：正常态，不重试（与 panel IsBuddyTaskIncomplete 一致）
        }
        return Err(resp_to_err(status, &resp));
    }
    Ok(Some(()))
}

/// 设置外观主题（Hp_Appearance 留痕 API）。
pub async fn set_appearance_theme(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    resource_key: &str,
) -> Result<(), String> {
    let url = format!("{}{APPEARANCE_SET_PATH}", region_spec(region).chat_base);
    let body = super::events::appearance_set_body(resource_key);
    let (status, resp) = io.json(region, &url, "POST", Some(body), account, &HashMap::new()).await;
    if status != 200 {
        return Err(resp_to_err(status, &resp));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 单测（纯函数 + Io 注入）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::Future;

    fn item(task_code: &str, current: i64, target: i64, accept_status: &str) -> Value {
        json!({
            "task_code": task_code,
            "current": current,
            "target": target,
            "accept_status": accept_status,
            "reward_credit": 100,
            "reward_energy": 5
        })
    }

    #[test]
    fn parse_progress_object_overrides_flat() {
        let data = json!({"data": {"tasks": [
            {
                "task_code": "t1", "current": 0, "target": 0,
                "progress": {"current": 3, "target": 5}
            }
        ]}});
        let tasks = parse_task_items(&data);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].current, 3);
        assert_eq!(tasks[0].target, 5);
        assert!(tasks[0].claimable == false);
    }

    #[test]
    fn parse_claimable_and_claimed() {
        let data = json!({"data": {"tasks": [
            {"task_code": "done", "current": 5, "target": 5, "accept_status": "completed"},
            {"task_code": "claim", "current": 5, "target": 5, "accept_status": "accepted"},
            {"task_code": "got", "current": 5, "target": 5, "accept_status": "claimed"}
        ]}});
        let tasks = parse_task_items(&data);
        assert_eq!(tasks[0].claimable, true);
        assert_eq!(tasks[1].claimable, true);
        assert_eq!(tasks[2].claimed, true);
        assert_eq!(tasks[2].claimable, false);
    }

    #[test]
    fn mp_codes_are_registered() {
        assert!(is_mp_task_code("school_season"));
        assert!(is_mp_task_code("Sequential_Tasks_4"));
        assert!(!is_mp_task_code("chat_5"));
        assert!(!is_mp_task_code(""));
    }

    #[test]
    fn pending_excludes_claimed_and_locked() {
        let t = GrowthTask {
            task_code: "x".into(),
            current: 0,
            target: 1,
            locked: true,
            ..Default::default()
        };
        assert!(!is_growth_pending(&t, true), "locked 不出待办");
        let t2 = GrowthTask {
            task_code: "x".into(),
            current: 0,
            target: 1,
            claimed: true,
            ..Default::default()
        };
        assert!(!is_growth_pending(&t2, true));
        let t3 = GrowthTask {
            task_code: "x".into(),
            current: 0,
            target: 1,
            ..Default::default()
        };
        assert!(is_growth_pending(&t3, true));
        assert!(!is_growth_pending(&t3, false), "无自动动作的不可自动化");
    }

    // ---------------- Io 注入桩 ----------------

    struct StubIo {
        responses: Vec<GrowthResponse>,
        calls: std::sync::Mutex<Vec<(String, String, Option<Value>)>>,
    }

    impl StubIo {
        fn new(responses: Vec<GrowthResponse>) -> Self {
            Self {
                responses,
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }
        fn call_urls(&self) -> Vec<String> {
            self.calls.lock().unwrap().iter().map(|c| c.0.clone()).collect()
        }
        fn call_bodies(&self) -> Vec<Option<Value>> {
            self.calls.lock().unwrap().iter().map(|c| c.2.clone()).collect()
        }
    }

    impl GrowthIo for StubIo {
        fn json<'a>(
            &'a self,
            _region: Region,
            url: &'a str,
            method: &'a str,
            body: Option<Value>,
            _account: &'a Value,
            _extra: &'a HashMap<String, String>,
        ) -> Pin<Box<dyn Future<Output = GrowthResponse> + Send + 'a>> {
            Box::pin(async move {
                let mut calls = self.calls.lock().unwrap();
                let idx = calls.len();
                calls.push((url.to_string(), method.to_string(), body));
                drop(calls);
                self.responses
                    .get(idx)
                    .cloned()
                    .unwrap_or((200, json!({"code": 0, "data": {}})))
            })
        }

        fn sse_chat<'a>(
            &'a self,
            _region: Region,
            _account: &'a Value,
            expert_id: &'a str,
            _body: &'a Value,
        ) -> Pin<Box<dyn Future<Output = Result<(String, String), String>> + Send + 'a>> {
            Box::pin(async move {
                Ok((
                    "conv-x".into(),
                    format!("cmb-stub-{}", expert_id.len()),
                ))
            })
        }
    }

    fn cn_account() -> Value {
        json!({"uid": "u1", "nickname": "测试", "access_token": "at"})
    }

    fn tasks_resp(tasks: Vec<Value>) -> GrowthResponse {
        (200, json!({"code": 0, "data": {"tasks": tasks}}))
    }

    /// claim：非 mp 码直接打 web 域。
    #[tokio::test]
    async fn claim_non_mp_goes_to_web_domain() {
        let io = StubIo::new(vec![
            (200, json!({"code": 0, "data": {"credit": 100, "energy": 5}})),
        ]);
        let out = claim_task(&io, Region::Cn, &cn_account(), "chat_5").await.unwrap();
        assert_eq!(out["credit"], 100);
        let url = io.call_urls().pop().unwrap();
        assert!(url.starts_with("https://www.workbuddy.cn"), "{url}");
        assert!(url.contains("/activity/growth/tasks/chat_5/claim"));
    }

    /// claim：mp 码先打 chat 域，400 时降级 web 域。
    #[tokio::test]
    async fn claim_mp_falls_back_to_web_on_400() {
        let io = StubIo::new(vec![
            (400, json!({"code": 40001, "message": "chat claim not allowed"})),
            (200, json!({"code": 0, "data": {"credit": 10, "energy": 1}})),
        ]);
        let out = claim_task(&io, Region::Cn, &cn_account(), "school_season").await.unwrap();
        assert_eq!(out["credit"], 10);
        let urls = io.call_urls();
        assert_eq!(urls.len(), 2);
        assert!(urls[0].starts_with("https://copilot.tencent.com"), "{urls:?}");
        assert!(urls[1].starts_with("https://www.workbuddy.cn"), "{urls:?}");
    }

    /// accept_all：默认列表 2 个未接受 + mp 列表 1 个 → accepted=3，分批 body 正确。
    #[tokio::test]
    async fn accept_all_covers_default_and_mp_rounds() {
        let io = StubIo::new(vec![
            tasks_resp(vec![
                item("chat_5", 0, 5, "not_accepted"),
                item("first_buddy", 0, 1, ""),
                item("done_task", 5, 5, "claimed"),
            ]),
            (200, json!({"code": 0})), // accept 默认轮
            tasks_resp(vec![item("Sequential_Tasks_1", 0, 1, "not_accepted")]),
            (200, json!({"code": 0})), // accept mp 轮
        ]);
        let out = accept_all(&io, Region::Cn, &cn_account()).await;
        assert_eq!(out["accepted"], 3, "{out}");
        assert_eq!(out["failed"], json!([]), "{out}");
    }

    /// task_by_code：默认列表查无 → mp 专属码回落 mp 列表。
    #[tokio::test]
    async fn task_by_code_mp_fallback() {
        let io = StubIo::new(vec![
            tasks_resp(vec![item("chat_5", 0, 5, "not_accepted")]),
            tasks_resp(vec![item("Sequential_Tasks_1", 2, 10, "accepted")]),
        ]);
        let found = task_by_code(&io, Region::Cn, &cn_account(), "Sequential_Tasks_1")
            .await
            .unwrap();
        assert_eq!(found.unwrap().task_code, "Sequential_Tasks_1");
        // 非 mp 码查无即 None（不多打一次上游）。
        let io2 = StubIo::new(vec![tasks_resp(vec![])]);
        let missing = task_by_code(&io2, Region::Cn, &cn_account(), "chat_5").await.unwrap();
        assert!(missing.is_none());
        assert_eq!(io2.call_urls().len(), 1);
    }

    /// 上报事件数组的 body 形状（report_desktop_events / report_mp_events）。
    #[tokio::test]
    async fn report_events_send_array_body() {
        let io = StubIo::new(vec![
            (200, json!({"code": 0})),
            (200, json!({"code": 0})),
        ]);
        let events_arr = json!([{"eventCode": "e1"}, {"eventCode": "e2"}]);
        report_desktop_events(&io, Region::Cn, &cn_account(), events_arr.clone()).await.unwrap();
        report_mp_events(&io, Region::Cn, &cn_account(), events_arr.clone()).await.unwrap();
        let urls = io.call_urls();
        assert!(urls[0].ends_with("/v2/report"));
        assert!(urls[1].ends_with("/v2/report"));
        assert_eq!(io.call_bodies()[0], Some(events_arr));
    }

    /// 领奖业务失败透出可读错误。
    #[tokio::test]
    async fn claim_business_error_is_surfaced() {
        let io = StubIo::new(vec![(403, json!({"code": 40301, "message": "task not completed"}))]);
        let err = claim_task(&io, Region::Cn, &cn_account(), "chat_5").await.unwrap_err();
        assert!(err.contains("task not completed"), "{err}");
    }
}
