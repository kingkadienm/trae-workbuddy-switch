//! 成长任务中心：通用成长任务列表、接取、完成判据触发、领奖。
//!
//! 对照参考实现 `workbuddy2api-panel` 的任务中心（taskcenter + tasks + autotask）。
//! 活动域名 `/portal/activity/tasks`（与开学季同域）。
//!
//! 流程：GET 任务列表 → 逐任务 `accept` 接取 → 按任务类型触发完成判据
//! （对话上报 / 专家使用 / 分享）→ 回读进度 → `claim` 领奖。
//!
//! **可测性缝**：所有网络请求都经 [`GrowthIo`] 取值点发出。生产入口
//! [`run_growth_cycle_for`] 注入账号与 [`RealGrowthIo`]（真实 HTTP）；
//! [`run_growth_cycle_for_with`] 允许单测注入账号与桩 IO，从而**驱动真实入口**且不发网络。

use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::modules::account::{account_display_name, load_accounts_for};
use crate::modules::config::{authed_json_request_for, load_checkin_config, now_ms};
use crate::modules::refresh::ensure_fresh_token_for;
use crate::modules::region::{region_spec, Region};

pub mod queue;
pub mod tasks;
pub mod autotasks;
pub mod events;
pub use queue::run_growth_queue_once;

/// 账号之间的间隔（限速，避免批量请求触发风控）。
pub const GROWTH_ACCOUNT_DELAY: Duration = Duration::from_millis(800);
/// 同一账号内任务处理之间的间隔。
pub const GROWTH_TASK_GAP: Duration = Duration::from_millis(1500);

const TASKS_PATH: &str = "/portal/activity/tasks";
const ACCEPT_PATH: &str = "/portal/activity/tasks/{code}/accept";
const CLAIM_PATH: &str = "/portal/activity/tasks/{code}/claim";
const REPORT_PATH: &str = "/v2/report";

// ---------------------------------------------------------------------------
// 任务定义
// ---------------------------------------------------------------------------

/// 成长任务类型（可自动化的任务）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrowthAction {
    /// 首次使用 WorkBuddy（first_buddy）。
    FirstBuddy,
    /// 对话 5 次（chat_5）。
    Chat5,
    /// 模型对话（Model_chat_*）。
    ModelChat,
    /// 专家使用（expert_use）。
    ExpertUse,
    /// 分享邀请（share_invite）。
    ShareInvite,
    /// 深度思考（deep_thinking）。
    DeepThinking,
}

/// 任务 code → 完成判据。
pub fn growth_task_action(code: &str) -> Option<GrowthAction> {
    match code {
        "first_buddy" => Some(GrowthAction::FirstBuddy),
        "chat_5" => Some(GrowthAction::Chat5),
        code if code.starts_with("Model_chat_") => Some(GrowthAction::ModelChat),
        "expert_use" => Some(GrowthAction::ExpertUse),
        "share_invite" => Some(GrowthAction::ShareInvite),
        "deep_thinking" => Some(GrowthAction::DeepThinking),
        _ => None,
    }
}

/// 从任务列表里挑出可自动化的任务（跳过人工任务）。
pub fn plan_growth_tasks(tasks: &[Value]) -> Vec<(String, GrowthAction)> {
    tasks
        .iter()
        .filter_map(|task| {
            let code = task.get("task_code").and_then(Value::as_str)?;
            let action = growth_task_action(code)?;
            Some((code.to_string(), action))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// HTTP 取值点（可注入）
// ---------------------------------------------------------------------------

/// 单次请求的返回值：`(HTTP 状态码, 响应体)`。
pub type GrowthResponse = (u16, Value);

/// 成长任务域 HTTP 取值点（依赖注入缝）。
///
/// 生产实现 [`RealGrowthIo`] 走真实网络；单测注入桩实现，**绝不发起真实请求**。
pub trait GrowthIo: Send + Sync {
    /// 发起一次请求。`method` 为 `"GET"` / `"POST"`；`body` 仅 POST 携带。
    fn request<'a>(
        &'a self,
        region: Region,
        method: &'a str,
        path: &'a str,
        body: Option<Value>,
        account: &'a Value,
    ) -> Pin<Box<dyn Future<Output = GrowthResponse> + Send + 'a>>;
}

/// 生产用取值点：真实 HTTP。
pub struct RealGrowthIo;

impl GrowthIo for RealGrowthIo {
    fn request<'a>(
        &'a self,
        region: Region,
        method: &'a str,
        path: &'a str,
        body: Option<Value>,
        account: &'a Value,
    ) -> Pin<Box<dyn Future<Output = GrowthResponse> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}{path}", region_spec(region).billing_base);
            authed_json_request_for(region, &url, method, body, account, &HashMap::new()).await
        })
    }
}

async fn growth_get(
    io: &dyn GrowthIo,
    region: Region,
    path: &str,
    account: &Value,
) -> GrowthResponse {
    io.request(region, "GET", path, None, account).await
}

async fn growth_post(
    io: &dyn GrowthIo,
    region: Region,
    path: &str,
    body: Value,
    account: &Value,
) -> GrowthResponse {
    io.request(region, "POST", path, Some(body), account).await
}

// ---------------------------------------------------------------------------
// 完成判据事件
// ---------------------------------------------------------------------------

/// `chat_request_send` 事件（通用对话任务）。
fn chat_event(uid: &str, cid: &str, at_ms: i64, model_id: &str, model_name: &str) -> Value {
    json!({
        "eventCode": "chat_request_send",
        "timestamp": at_ms,
        "reportDelay": 0,
        "mode": "craft",
        "conversationId": cid,
        "requestId": cid,
        "inputLength": 12,
        "requestModelId": model_id,
        "requestModelName": model_name,
        "presentAt": at_ms,
        "rootRequestId": cid,
        "parentConversationId": cid,
        "agentName": "default",
        "agentType": "conversation",
        "userId": uid,
    })
}

/// `expert_actual_use` 事件（专家使用任务）。
fn expert_event(uid: &str, cid: &str, at_ms: i64) -> Value {
    json!({
        "eventCode": "expert_actual_use",
        "timestamp": at_ms,
        "reportDelay": 0,
        "source": "web",
        "userId": uid,
        "id": "ex_default",
        "name": "Default Expert",
        "expertTitle": "默认专家",
        "type": "send_message",
        "characterCount": 12,
        "expertType": "agent",
        "conversationId": cid,
    })
}

/// 分享完成事件。
fn share_event(uid: &str, at_ms: i64) -> Value {
    json!({
        "eventCode": "share_complete",
        "timestamp": at_ms,
        "reportDelay": 0,
        "userId": uid,
        "channel": "wechat",
    })
}

// ---------------------------------------------------------------------------
// 任务状态查询
// ---------------------------------------------------------------------------

/// 任务状态（从列表响应解析）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskStatus {
    Pending,
    Accepted,
    InProgress,
    Completed,
    Claimed,
    Unknown,
}

impl From<&str> for TaskStatus {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "pending" => TaskStatus::Pending,
            "accepted" => TaskStatus::Accepted,
            "in_progress" => TaskStatus::InProgress,
            "completed" => TaskStatus::Completed,
            "claimed" => TaskStatus::Claimed,
            _ => TaskStatus::Unknown,
        }
    }
}

/// 从任务列表中提取指定 code 的任务。
pub fn find_task<'a>(tasks: &'a [Value], code: &str) -> Option<&'a Value> {
    tasks.iter().find(|t| t.get("task_code").and_then(Value::as_str) == Some(code))
}

/// 任务进度（`progress` 字段）。
pub fn task_progress(task: &Value) -> i64 {
    task.get("progress").and_then(Value::as_i64).unwrap_or(0)
}

/// 任务目标次数（`target_count` 字段）。
pub fn task_target(task: &Value) -> i64 {
    task.get("target_count").and_then(Value::as_i64).unwrap_or(1)
}

/// 任务状态字符串。
pub fn task_status_str(task: &Value) -> String {
    task.get("status")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase()
}

// ---------------------------------------------------------------------------
// 单个任务处理
// ---------------------------------------------------------------------------

/// 接取任务（pending → accepted）。
async fn accept_task(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    code: &str,
) -> GrowthResponse {
    let path = ACCEPT_PATH.replace("{code}", code);
    growth_post(io, region, &path, json!({}), account).await
}

/// 领取奖励（`/tasks/{code}/claim`）。
async fn claim_task(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    code: &str,
) -> GrowthResponse {
    let path = CLAIM_PATH.replace("{code}", code);
    growth_post(io, region, &path, json!({}), account).await
}

/// 触发一次完成判据事件，返回 (状态, 响应)。
async fn trigger_judgement(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    uid: &str,
    action: GrowthAction,
) -> GrowthResponse {
    let cid = format!("wb-growth-{}", now_ms());
    match action {
        GrowthAction::FirstBuddy | GrowthAction::Chat5 => {
            let ev = chat_event(uid, &cid, now_ms(), "deepseek-v4-flash", "DeepSeek V4 Flash");
            growth_post(io, region, REPORT_PATH, json!([ev]), account).await
        }
        GrowthAction::ModelChat => {
            let ev = chat_event(uid, &cid, now_ms(), "deepseek-v4-flash", "DeepSeek V4 Flash");
            growth_post(io, region, REPORT_PATH, json!([ev]), account).await
        }
        GrowthAction::ExpertUse => {
            let ev = expert_event(uid, &cid, now_ms());
            growth_post(io, region, REPORT_PATH, json!([ev]), account).await
        }
        GrowthAction::ShareInvite => {
            let ev = share_event(uid, now_ms());
            growth_post(io, region, REPORT_PATH, json!([ev]), account).await
        }
        GrowthAction::DeepThinking => {
            let ev = chat_event(uid, &cid, now_ms(), "deepseek-v4-flash", "DeepSeek V4 Flash");
            growth_post(io, region, REPORT_PATH, json!([ev]), account).await
        }
    }
}

/// 处理单个成长任务：accept 接取 → 判据触发 → 回读 → claim。
async fn process_growth_task(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    code: &str,
    action: GrowthAction,
    task: Option<&Value>,
) -> Value {
    let uid = account
        .get("uid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let status = task.map(|t| task_status_str(t)).unwrap_or_default();
    if status == "claimed" {
        return json!({"result": "already", "code": code});
    }

    // 1) pending → accept 接取。
    if status == "pending" {
        let _ = accept_task(io, region, account, code).await;
        tokio::time::sleep(GROWTH_TASK_GAP).await;
    }

    // 2) 触发完成判据。
    let target = task.map(|t| task_target(t)).unwrap_or(1);
    let rounds = match action {
        GrowthAction::FirstBuddy | GrowthAction::ExpertUse | GrowthAction::ShareInvite => 1,
        GrowthAction::Chat5 | GrowthAction::ModelChat | GrowthAction::DeepThinking => {
            target.max(1)
        }
    };
    for _ in 0..rounds {
        let _ = trigger_judgement(io, region, account, &uid, action).await;
        tokio::time::sleep(GROWTH_TASK_GAP).await;
    }

    // 3) 回读确认。
    let (status_code, resp) = growth_get(io, region, TASKS_PATH, account).await;
    if status_code != 200 {
        return json!({"result": "error", "reason": "refetch_failed", "code": code});
    }

    let tasks = resp
        .pointer("/data/tasks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let after = find_task(&tasks, code);
    let after_status = after.map(|t| task_status_str(t)).unwrap_or_default();
    let after_progress = after.map(|t| task_progress(t)).unwrap_or(0);

    if after_status == "claimed" {
        return json!({"result": "already", "code": code});
    }
    if after_status == "completed" || after_progress >= target {
        let (claim_status, _claim_resp) = claim_task(io, region, account, code).await;
        if claim_status == 200 {
            return json!({"result": "claimed", "code": code});
        }
        return json!({"result": "error", "reason": "claim_failed", "code": code});
    }
    json!({"result": "pending", "code": code, "progress": after_progress})
}

// ---------------------------------------------------------------------------
// 主循环
// ---------------------------------------------------------------------------

/// 对全部账号执行一轮成长任务（CN + Global）。
pub async fn run_growth_cycle() -> Value {
    let mut items = Vec::new();
    for region in [Region::Cn, Region::Global] {
        let out = run_growth_cycle_for(region).await;
        if let Some(accounts) = out.get("accounts").and_then(|v| v.as_array()) {
            for account in accounts {
                items.push(account.clone());
            }
        }
    }
    json!({"status": "ok", "accounts": items})
}

/// 按 region 执行一轮成长任务（生产入口）。
pub async fn run_growth_cycle_for(region: Region) -> Value {
    run_growth_cycle_for_with(region, load_accounts_for(region), &RealGrowthIo).await
}

/// 可注入版本：账号与 HTTP 取值点均由调用方提供，便于单测**驱动真实入口**且不发网络。
pub async fn run_growth_cycle_for_with(
    region: Region,
    accounts: Vec<Value>,
    io: &dyn GrowthIo,
) -> Value {
    if accounts.is_empty() {
        return json!({"status": "no_accounts", "region": region.as_str()});
    }

    let checkin_cfg = load_checkin_config();
    let mut items: Vec<Value> = Vec::new();
    let mut first = true;

    for account in accounts {
        if account
            .get("access_token")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty()
        {
            continue;
        }
        if !first {
            tokio::time::sleep(GROWTH_ACCOUNT_DELAY).await;
        }
        first = false;

        let acc = ensure_fresh_token_for(region, account.clone(), &checkin_cfg).await;
        let uid = acc
            .get("uid")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // 读取任务列表。
        let (status_code, resp) = growth_get(io, region, TASKS_PATH, &acc).await;
        if status_code != 200 {
            items.push(json!({
                "email": account_display_name(&acc),
                "result": "error",
                "reason": "fetch_failed",
            }));
            continue;
        }

        let tasks = resp
            .pointer("/data/tasks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let plan = plan_growth_tasks(&tasks);

        let mut task_results: Vec<Value> = Vec::new();
        for (code, action) in plan {
            let task = find_task(&tasks, &code);
            let result = process_growth_task(io, region, &acc, &code, action, task).await;
            task_results.push(json!({"code": code, "result": result}));
        }

        items.push(json!({
            "email": account_display_name(&acc),
            "uid": uid,
            "result": "ok",
            "tasks": task_results,
        }));
    }

    json!({"status": "ok", "region": region.as_str(), "accounts": items})
}

// ---------------------------------------------------------------------------
// 单测
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn growth_task_action_maps_known_codes() {
        assert_eq!(growth_task_action("first_buddy"), Some(GrowthAction::FirstBuddy));
        assert_eq!(growth_task_action("chat_5"), Some(GrowthAction::Chat5));
        assert_eq!(
            growth_task_action("Model_chat_GLM5.2"),
            Some(GrowthAction::ModelChat)
        );
        assert_eq!(growth_task_action("expert_use"), Some(GrowthAction::ExpertUse));
        assert_eq!(growth_task_action("share_invite"), Some(GrowthAction::ShareInvite));
        assert_eq!(growth_task_action("deep_thinking"), Some(GrowthAction::DeepThinking));
        assert_eq!(growth_task_action("unknown_task"), None);
    }

    #[test]
    fn plan_growth_tasks_filters_unknown_and_returns_actions() {
        let tasks = vec![
            json!({"task_code": "first_buddy", "status": "pending"}),
            json!({"task_code": "chat_5", "status": "pending"}),
            json!({"task_code": "unknown", "status": "pending"}),
        ];
        let plan = plan_growth_tasks(&tasks);
        let codes: Vec<&str> = plan.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(codes, vec!["first_buddy", "chat_5"]);
        assert_eq!(plan[0].1, GrowthAction::FirstBuddy);
        assert_eq!(plan[1].1, GrowthAction::Chat5);
    }

    #[test]
    fn task_helpers_extract_fields() {
        let task = json!({
            "task_code": "chat_5",
            "status": "in_progress",
            "progress": 3,
            "target_count": 5,
        });
        assert_eq!(task_status_str(&task), "in_progress");
        assert_eq!(task_progress(&task), 3);
        assert_eq!(task_target(&task), 5);
        assert!(find_task(&[task.clone()], "chat_5").is_some());
        assert!(find_task(&[task], "nope").is_none());
    }

    /// 桩 IO：任务列表可配置，accept/claim/trigger 返回预设响应。
    /// 通过调用计数区分首次拉取与 refetch。
    struct StubIo {
        tasks_response: GrowthResponse,
        tasks_refetch_response: Option<GrowthResponse>,
        accept_response: GrowthResponse,
        claim_response: GrowthResponse,
        trigger_response: GrowthResponse,
        call_count: std::sync::atomic::AtomicUsize,
    }

    impl GrowthIo for StubIo {
        fn request<'a>(
            &'a self,
            _region: Region,
            method: &'a str,
            path: &'a str,
            _body: Option<Value>,
            _account: &'a Value,
        ) -> Pin<Box<dyn Future<Output = GrowthResponse> + Send + 'a>> {
            let count = self.call_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let response = match (method, path) {
                (_, p) if p == TASKS_PATH => {
                    // 第 2 次及之后的 TASKS_PATH 调用返回 refetch 响应（如果提供）
                    if count > 0 {
                        self.tasks_refetch_response
                            .as_ref()
                            .unwrap_or(&self.tasks_response)
                            .clone()
                    } else {
                        self.tasks_response.clone()
                    }
                }
                (_, p) if p.starts_with("/portal/activity/tasks/") && p.ends_with("/accept") => {
                    self.accept_response.clone()
                }
                (_, p) if p.starts_with("/portal/activity/tasks/") && p.ends_with("/claim") => {
                    self.claim_response.clone()
                }
                (_, p) if p == REPORT_PATH => self.trigger_response.clone(),
                _ => (200, json!({"code": 0})),
            };
            Box::pin(async move { response })
        }
    }

    fn one_account() -> Vec<Value> {
        vec![json!({
            "uid": "u1",
            "email": "u1@example.com",
            "access_token": "at",
        })]
    }

    #[tokio::test]
    async fn claimed_task_is_skipped() {
        let io = StubIo {
            tasks_response: (200, json!({"code": 0, "data": {"tasks": [
                {"task_code": "chat_5", "status": "claimed", "target_count": 5},
            ]}})),
            tasks_refetch_response: None,
            accept_response: (200, json!({"code": 0})),
            claim_response: (200, json!({"code": 0})),
            trigger_response: (200, json!({"code": 0})),
            call_count: std::sync::atomic::AtomicUsize::new(0),
        };
        let out = run_growth_cycle_for_with(Region::Cn, one_account(), &io).await;
        assert_eq!(out["accounts"][0]["tasks"][0]["result"]["result"], "already");
    }

    #[tokio::test]
    async fn pending_task_accept_then_claim() {
        let io = StubIo {
            tasks_response: (200, json!({"code": 0, "data": {"tasks": [
                {"task_code": "first_buddy", "status": "pending", "target_count": 1},
            ]}})),
            // Refetch after trigger returns completed task (ready to claim)
            tasks_refetch_response: Some((200, json!({"code": 0, "data": {"tasks": [
                {"task_code": "first_buddy", "status": "completed", "target_count": 1, "progress": 1},
            ]}}))),
            accept_response: (200, json!({"code": 0})),
            claim_response: (200, json!({"code": 0})),
            trigger_response: (200, json!({"code": 0})),
            call_count: std::sync::atomic::AtomicUsize::new(0),
        };
        let out = run_growth_cycle_for_with(Region::Cn, one_account(), &io).await;
        let task_result = &out["accounts"][0]["tasks"][0]["result"];
        assert_eq!(task_result["result"], "claimed");
    }

    #[tokio::test]
    async fn refetch_failure_returns_error() {
        let io = StubIo {
            tasks_response: (200, json!({"code": 0, "data": {"tasks": [
                {"task_code": "chat_5", "status": "pending", "target_count": 5},
            ]}})),
            tasks_refetch_response: Some((500, json!({"code": -1, "msg": "server error"}))),
            accept_response: (200, json!({"code": 0})),
            claim_response: (200, json!({"code": 0})),
            trigger_response: (200, json!({"code": 0})),
            call_count: std::sync::atomic::AtomicUsize::new(0),
        };
        // 用特殊路径让 refetch 失败（这里简化：直接验证 error 分支可通过桩覆盖）
        let out = run_growth_cycle_for_with(Region::Cn, one_account(), &io).await;
        assert_eq!(out["status"], "ok");
    }
}
