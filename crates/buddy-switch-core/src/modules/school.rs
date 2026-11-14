//! 开学季活动任务（小程序域，**CN 专有**）。
//!
//! 对照参考实现 `scripts/school_open_day_2026.py` + `internal/scheduler/school.go`。
//! 活动域名 `https://www.codebuddy.cn`（= CN 的 `region_spec.billing_base`）。
//!
//! 流程：GET 任务列表（含 `in_period`）→ 逐任务 `viewed` 激活 → 触发完成判据
//! （报告事件 / share-complete）→ 回读 → `claim` 领奖 → 活动收尾抽空转盘次数。
//!
//! **下线跳过（最重要的一条）**：`in_period == false` → 各段**全量跳过**并以**正常
//! 成功**退出，不得记为失败、不得产生 ERROR 日志。跳过结果**统一由
//! [`school_skip_summary`] 产出**（单一来源），避免「产线内联 JSON」与「断言函数」各写一份
//! 而脱钩。
//!
//! `task_student_verify`（学生认证）是人工环节，直接跳过。
//!
//! **可测性缝**：所有网络请求都经 [`SchoolIo`] 取值点发出。生产入口
//! [`run_school_cycle_for`] 注入账号与 [`RealSchoolIo`]（真实 HTTP）；
//! [`run_school_cycle_for_with`] 允许单测注入账号与桩 IO，从而**驱动真实入口**且不发网络。

use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use crate::modules::account::{account_display_name, load_accounts_for};
use crate::modules::config::{authed_json_request_for, load_checkin_config, now_ms};
use crate::modules::refresh::ensure_fresh_token_for;
use crate::modules::region::{region_spec, Region};

/// 开学季活动唯一标识（事件体 `activityId` 字段值）。
pub const SCHOOL_ACTIVITY_ID: &str = "school_open_day_2026";
/// 写动作之间的间隔（与参考脚本 `--gap` 默认 1.5s 对齐）。
const SCHOOL_GAP: Duration = Duration::from_millis(1500);

const TASKS_PATH: &str = "/portal/activity/school/tasks";
const CONFIG_PATH: &str = "/portal/activity/school/config";
const SHARE_PATH: &str = "/portal/activity/school/tasks/share-complete";
const WHEEL_PATH: &str = "/portal/activity/school/wheel/draw";
const REPORT_PATH: &str = "/v2/report";

/// 开学季专家的内置回落（分类专家列表拉取失败时的兜底，参考脚本同款）。
const FALLBACK_EXPERT_ID: &str = "ex_jB0dyFIQJEWa";
const FALLBACK_EXPERT_NAME: &str = "论小舟";

/// 单个 school 任务的完成判据类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchoolAction {
    /// 小程序对话 3 次（mini `chat_request_send` + activityId）。
    MiniChat,
    /// 开学季专家对话（`expert_actual_use`）。
    Expert,
    /// 分享活动（`share-complete` 端点）。
    Share,
    /// 桌面端对话 1 次（桌面 6 连事件组 + activityId）。
    DesktopSeq,
}

/// 任务 code → 完成判据；`task_student_verify`（人工）与未知任务返回 `None`（保守跳过）。
pub fn school_task_action(code: &str) -> Option<SchoolAction> {
    match code {
        "chat_3_times" => Some(SchoolAction::MiniChat),
        "expert_use" => Some(SchoolAction::Expert),
        "share_invite" => Some(SchoolAction::Share),
        "desktop_chat_1_time" => Some(SchoolAction::DesktopSeq),
        // task_student_verify 为人工环节；其余未在清单内的任务保守跳过。
        _ => None,
    }
}

/// 从任务列表里挑出可处理的任务（跳过人工 / 未知任务）。
pub fn plan_school_tasks(tasks: &[Value]) -> Vec<(String, SchoolAction)> {
    tasks
        .iter()
        .filter_map(|task| {
            let code = task.get("task_code").and_then(Value::as_str)?;
            let action = school_task_action(code)?;
            Some((code.to_string(), action))
        })
        .collect()
}

/// 活动处于哪一阶段：进行期执行，非进行期**全量跳过并正常退出**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchoolPhase {
    Run,
    Skip,
}

/// 由 `in_period` 判定阶段。
pub fn school_phase(in_period: bool) -> SchoolPhase {
    if in_period {
        SchoolPhase::Run
    } else {
        SchoolPhase::Skip
    }
}

/// 活动下线时的汇总（**正常成功**，不是失败）。
///
/// 这是「活动下线 → 全量跳过」结果的**唯一来源**：产线跳过分支直接返回本函数结果，
/// 单测断言也以本函数为准，二者是同一条路径，不会各自演进而脱钩。
pub fn school_skip_summary() -> Value {
    json!({"status": "ok", "skipped": "not_in_period", "accounts": []})
}

/// 成功判据：HTTP 200 且业务 `code` 为 0（或缺失）。
fn school_ok(status: u16, resp: &Value) -> bool {
    status == 200 && matches!(resp.get("code").and_then(Value::as_i64), Some(0) | None)
}

/// 小程序域请求头附加项（带小程序 UA 与站点 Origin/Referer）。
fn school_extra(region: Region) -> HashMap<String, String> {
    const MP_UA: &str = "Mozilla/5.0 (Linux; Android 14; MicroMessenger/8.0.49 WeChat/0.8.0 MiniProgramEnv/android; wkbrowser xweb)";
    let base = region_spec(region).billing_base;
    let mut headers = HashMap::new();
    headers.insert("user-agent".to_string(), MP_UA.to_string());
    headers.insert("origin".to_string(), base.to_string());
    headers.insert("referer".to_string(), format!("{base}/"));
    headers
}

// ---------------------------------------------------------------------------
// HTTP 取值点（可注入）
// ---------------------------------------------------------------------------

/// 单次请求的返回值：`(HTTP 状态码, 响应体)`。
///
/// 保留原始状态码是关键——「活动下线(200 + code≠0)」「转盘余额为 0(409)」等**正常态**
/// 判定依赖它；普通 `http_request` 会把状态码折进 `code`，无法区分。
pub type SchoolResponse = (u16, Value);

/// 开学季域 HTTP 取值点（依赖注入缝）。
///
/// 生产实现 [`RealSchoolIo`] 走真实网络；单测注入桩实现，**绝不发起真实请求**。
/// 这使得单测可以驱动真实入口 [`run_school_cycle_for_with`]，而不是只测孤立纯函数。
///
/// `Send + Sync` 超级约束：调度器在 `tokio::spawn` 里调用，要求 `&dyn SchoolIo` 可跨线程。
pub trait SchoolIo: Send + Sync {
    /// 发起一次请求。`method` 为 `"GET"` / `"POST"`；`body` 仅 POST 携带。
    fn request<'a>(
        &'a self,
        region: Region,
        method: &'a str,
        path: &'a str,
        body: Option<Value>,
        account: &'a Value,
    ) -> Pin<Box<dyn Future<Output = SchoolResponse> + Send + 'a>>;
}

/// 生产用取值点：真实 HTTP（附加小程序 UA / Origin / Referer）。
pub struct RealSchoolIo;

impl SchoolIo for RealSchoolIo {
    fn request<'a>(
        &'a self,
        region: Region,
        method: &'a str,
        path: &'a str,
        body: Option<Value>,
        account: &'a Value,
    ) -> Pin<Box<dyn Future<Output = SchoolResponse> + Send + 'a>> {
        Box::pin(async move {
            let url = format!("{}{path}", region_spec(region).billing_base);
            authed_json_request_for(region, &url, method, body, account, &school_extra(region))
                .await
        })
    }
}

async fn school_get(
    io: &dyn SchoolIo,
    region: Region,
    path: &str,
    account: &Value,
) -> SchoolResponse {
    io.request(region, "GET", path, None, account).await
}

async fn school_post(
    io: &dyn SchoolIo,
    region: Region,
    path: &str,
    body: Value,
    account: &Value,
) -> SchoolResponse {
    io.request(region, "POST", path, Some(body), account).await
}

// ---------------------------------------------------------------------------
// 完成判据事件（全部带 activityId）
// ---------------------------------------------------------------------------

/// 小程序域 `chat_request_send`（点亮 chat_3_times）。
fn mini_chat_event(uid: &str, cid: &str, at_ms: i64) -> Value {
    json!({
        "eventCode": "chat_request_send",
        "timestamp": at_ms,
        "reportDelay": 0,
        "source": "mini_program",
        "ideName": "wx_app_cloud",
        "ideType": "WorkBuddy_MP",
        "extName": "workbuddy-mp",
        "extVersion": "SaaS",
        "mode": "chat",
        "conversationId": cid,
        "requestId": cid,
        "inputLength": 12,
        "activityId": SCHOOL_ACTIVITY_ID,
        "mentionContexts": [],
        "mentionContextCount": 0,
        "userId": uid,
    })
}

/// `expert_actual_use`（点亮 expert_use）。
fn expert_event(uid: &str, cid: &str, at_ms: i64) -> Value {
    json!({
        "eventCode": "expert_actual_use",
        "timestamp": at_ms,
        "reportDelay": 0,
        "source": "mini_program",
        "ideName": "wx_app_cloud",
        "ideType": "WorkBuddy_MP",
        "extName": "workbuddy-mp",
        "extVersion": "SaaS",
        "userId": uid,
        "id": FALLBACK_EXPERT_ID,
        "name": FALLBACK_EXPERT_ID,
        "expertTitle": FALLBACK_EXPERT_NAME,
        "type": "send_message",
        "characterCount": 12,
        "expertType": "agent",
        "conversationId": cid,
        "activityId": SCHOOL_ACTIVITY_ID,
    })
}

/// 桌面端成功对话 6 连事件链（点亮 desktop_chat_1_time），每事件注入 activityId。
fn desktop_seq_events(uid: &str, cid: &str, at_ms: i64) -> Vec<Value> {
    let base =
        |code: &str| json!({"eventCode": code, "userId": uid, "activityId": SCHOOL_ACTIVITY_ID});
    let mut events = Vec::new();

    let mut created = base("agent_task_created");
    created["source"] = json!("LOCAL");
    created["name"] = json!("working");
    created["task_target"] = json!("local");
    created["mode"] = json!("craft");
    created["requestModelId"] = json!("fast-model");
    created["requestModelName"] = json!("fast-model");
    created["conversationId"] = json!(cid);
    created["messageId"] = json!(cid);
    events.push(created);

    let mut send = base("chat_message_send");
    send["messageId"] = json!(format!("{cid}-assistant"));
    send["historyCount"] = json!(0);
    send["currentStepCount"] = json!(1);
    send["traceId"] = json!(cid);
    send["rootRequestId"] = json!(cid);
    send["parentConversationId"] = json!(cid);
    send["agentName"] = json!("cli");
    send["agentType"] = json!("main");
    events.push(send);

    let mut req = base("chat_request_send");
    req["inputLength"] = json!(24);
    req["mode"] = json!("craft");
    req["conversationId"] = json!(cid);
    req["requestId"] = json!(cid);
    req["traceId"] = json!(cid);
    req["rootRequestId"] = json!(cid);
    req["parentConversationId"] = json!(cid);
    req["agentName"] = json!("cli");
    req["agentType"] = json!("main");
    req["timestamp"] = json!(at_ms);
    events.push(req);

    let mut resp = base("chat_message_response");
    resp["messageId"] = json!(format!("{cid}-assistant"));
    resp["responseModelId"] = json!("fast-model");
    resp["isSuccessful"] = json!(true);
    resp["finishReason"] = json!("stop");
    resp["traceId"] = json!(cid);
    resp["conversationId"] = json!(cid);
    resp["rootRequestId"] = json!(cid);
    resp["parentConversationId"] = json!(cid);
    resp["agentName"] = json!("cli");
    resp["agentType"] = json!("main");
    events.push(resp);

    let mut status = base("chat_message_status");
    status["messageId"] = json!(format!("{cid}-assistant"));
    status["messageErrorCode"] = json!("0");
    status["traceId"] = json!(cid);
    status["rootRequestId"] = json!(cid);
    status["parentConversationId"] = json!(cid);
    events.push(status);

    let mut done = base("chat_request_response");
    done["mode"] = json!("craft");
    done["isSuccessful"] = json!(true);
    done["finishReason"] = json!("stop");
    done["rootRequestId"] = json!(cid);
    done["parentConversationId"] = json!(cid);
    events.push(done);

    events
}

// ---------------------------------------------------------------------------
// 单任务处理
// ---------------------------------------------------------------------------

/// 激活任务（pending → viewed）。
async fn post_viewed(
    io: &dyn SchoolIo,
    region: Region,
    account: &Value,
    code: &str,
) -> SchoolResponse {
    school_post(
        io,
        region,
        &format!("/portal/activity/school/tasks/{code}/viewed"),
        json!({}),
        account,
    )
    .await
}

/// 领奖（`/tasks/{code}/claim`，无 body）。
async fn post_claim(
    io: &dyn SchoolIo,
    region: Region,
    account: &Value,
    code: &str,
) -> SchoolResponse {
    school_post(
        io,
        region,
        &format!("/portal/activity/school/tasks/{code}/claim"),
        json!({}),
        account,
    )
    .await
}

/// 触发一次完成判据（share / report），返回 (状态, 响应)。
async fn trigger_judgement(
    io: &dyn SchoolIo,
    region: Region,
    account: &Value,
    uid: &str,
    action: SchoolAction,
) -> SchoolResponse {
    match action {
        SchoolAction::Share => {
            school_post(
                io,
                region,
                SHARE_PATH,
                json!({"channel": "wechat"}),
                account,
            )
            .await
        }
        SchoolAction::MiniChat => {
            let cid = format!("wbmp-{}", now_ms());
            let ev = mini_chat_event(uid, &cid, now_ms());
            school_post(io, region, REPORT_PATH, json!([ev]), account).await
        }
        SchoolAction::Expert => {
            let cid = format!("wbexp-{}", now_ms());
            let ev = expert_event(uid, &cid, now_ms());
            school_post(io, region, REPORT_PATH, json!([ev]), account).await
        }
        SchoolAction::DesktopSeq => {
            let cid = format!("wbdesk-{}", now_ms());
            let events = desktop_seq_events(uid, &cid, now_ms());
            school_post(io, region, REPORT_PATH, json!(events), account).await
        }
    }
}

/// 处理单个 school 任务：viewed 激活 → 判据触发 → 回读 → claim。
async fn process_school_task(
    io: &dyn SchoolIo,
    region: Region,
    account: &Value,
    code: &str,
    action: SchoolAction,
    task: Option<&Value>,
) -> Value {
    let uid = account
        .get("uid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let status = task
        .and_then(|t| t.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    if status == "completed" || status == "claimed" {
        return json!({"result": "already"});
    }

    // 1) pending → viewed 激活（「接任务」）。
    if status == "pending" {
        let _ = post_viewed(io, region, account, code).await;
        tokio::time::sleep(SCHOOL_GAP).await;
    }

    let cur = task
        .and_then(|t| t.get("progress"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let target = task
        .and_then(|t| t.get("target_count"))
        .and_then(Value::as_i64)
        .unwrap_or(1);
    // 每次事件独立 +1；share 一次即完成，desktop 6 连一次 = 一次对话。
    let rounds = match action {
        SchoolAction::Share | SchoolAction::DesktopSeq => 1,
        SchoolAction::MiniChat | SchoolAction::Expert => (target - cur).max(1),
    };
    for _ in 0..rounds {
        let _ = trigger_judgement(io, region, account, &uid, action).await;
        tokio::time::sleep(SCHOOL_GAP).await;
    }

    // 2) 回读确认；completed/claimed 或进度达 target → claim。
    let (status_code, resp) = school_get(io, region, TASKS_PATH, account).await;
    if !school_ok(status_code, &resp) {
        return json!({"result": "error", "reason": "refetch_failed"});
    }
    let tasks = resp
        .pointer("/data/tasks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let after = tasks
        .iter()
        .find(|t| t.get("task_code").and_then(Value::as_str) == Some(code));
    let after_status = after
        .and_then(|t| t.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase();
    let after_progress = after
        .and_then(|t| t.get("progress"))
        .and_then(Value::as_i64)
        .unwrap_or(cur);
    if after_status == "claimed" {
        return json!({"result": "already"});
    }
    if after_status == "completed" || after_progress >= target {
        let (claim_status, claim_resp) = post_claim(io, region, account, code).await;
        if school_ok(claim_status, &claim_resp) {
            return json!({"result": "claimed"});
        }
        return json!({"result": "error", "reason": "claim_failed"});
    }
    json!({"result": "pending", "progress": after_progress})
}

/// 活动收尾抽奖段：抽空转盘次数（余额归零 / 服务端 409 no chance 即停）。
async fn run_school_lottery(io: &dyn SchoolIo, region: Region, account: &Value) -> Value {
    let (status, resp) = school_get(io, region, CONFIG_PATH, account).await;
    if !school_ok(status, &resp) {
        return json!({"result": "error"});
    }
    if resp.pointer("/data/in_period").and_then(Value::as_bool) != Some(true) {
        return json!({"result": "skipped", "reason": "not_in_period"});
    }
    let mut balance = resp
        .pointer("/data/chance/balance")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let mut drawn = 0i64;
    while balance > 0 {
        let draw_uuid = uuid::Uuid::new_v4().to_string();
        let (draw_status, draw_resp) = school_post(
            io,
            region,
            WHEEL_PATH,
            json!({"draw_uuid": draw_uuid}),
            account,
        )
        .await;
        // 余额为 0 时上游返回 HTTP 409 + code=40900 "no chance"：边界，正常结束。
        if draw_status == 409 {
            break;
        }
        if !school_ok(draw_status, &draw_resp) {
            break;
        }
        let next = draw_resp
            .pointer("/data/chance_balance")
            .and_then(Value::as_i64)
            .unwrap_or(balance - 1);
        if next >= balance {
            break; // 余额未递减：服务端异常，防死循环。
        }
        balance = next;
        drawn += 1;
        tokio::time::sleep(SCHOOL_GAP).await;
    }
    json!({"result": "ok", "drawn": drawn})
}

// ---------------------------------------------------------------------------
// 主循环
// ---------------------------------------------------------------------------

/// 对全部账号执行一轮开学季任务（CN）。
pub async fn run_school_cycle() -> Value {
    run_school_cycle_for(Region::Cn).await
}

/// 按 region 执行一轮开学季任务（生产入口）。CN 专有；Global 返回结构化 unsupported。
///
/// 生产路径：加载账号 + 注入 [`RealSchoolIo`]（真实 HTTP），转调
/// [`run_school_cycle_for_with`]。二者共享同一段主循环逻辑（含下线跳过分支）。
pub async fn run_school_cycle_for(region: Region) -> Value {
    if region != Region::Cn {
        return json!({"status": "unsupported", "reason": "school is CN only"});
    }
    run_school_cycle_for_with(region, load_accounts_for(region), &RealSchoolIo).await
}

/// 可注入版本：账号与 HTTP 取值点均由调用方提供，便于单测**驱动真实入口**且不发网络。
pub async fn run_school_cycle_for_with(
    region: Region,
    accounts: Vec<Value>,
    io: &dyn SchoolIo,
) -> Value {
    if region != Region::Cn {
        return json!({"status": "unsupported", "reason": "school is CN only"});
    }
    if accounts.is_empty() {
        return json!({"status": "no_accounts"});
    }
    let checkin_cfg = load_checkin_config();
    let mut items: Vec<Value> = Vec::new();
    for account in accounts {
        let acc = ensure_fresh_token_for(region, account.clone(), &checkin_cfg).await;
        let (status, resp) = school_get(io, region, TASKS_PATH, &acc).await;
        if !school_ok(status, &resp) {
            items.push(json!({"email": account_display_name(&acc), "result": "error"}));
            continue;
        }
        let in_period = resp
            .pointer("/data/in_period")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        // 活动下线 → 各段全量跳过并以正常成功退出（结果单一来源：school_skip_summary）。
        if school_phase(in_period) == SchoolPhase::Skip {
            return school_skip_summary();
        }
        let tasks = resp
            .pointer("/data/tasks")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut task_results: Vec<Value> = Vec::new();
        for (code, action) in plan_school_tasks(&tasks) {
            let task = tasks
                .iter()
                .find(|t| t.get("task_code").and_then(Value::as_str) == Some(code.as_str()));
            let result = process_school_task(io, region, &acc, &code, action, task).await;
            task_results.push(json!({"code": code, "result": result}));
        }
        let lottery = run_school_lottery(io, region, &acc).await;
        items.push(json!({
            "email": account_display_name(&acc),
            "result": "ok",
            "tasks": task_results,
            "lottery": lottery,
        }));
    }
    json!({"status": "ok", "accounts": items})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded_tasks() -> Vec<Value> {
        vec![
            json!({"task_code": "task_student_verify", "status": "pending"}),
            json!({"task_code": "chat_3_times", "status": "pending", "target_count": 3}),
            json!({"task_code": "expert_use", "status": "in_progress", "target_count": 1}),
            json!({"task_code": "share_invite", "status": "pending", "target_count": 1}),
            json!({"task_code": "desktop_chat_1_time", "status": "completed", "target_count": 1}),
            json!({"task_code": "unknown_future_task", "status": "pending"}),
        ]
    }

    #[test]
    fn plan_skips_manual_and_unknown_and_keeps_the_four_reportable() {
        let plan = plan_school_tasks(&seeded_tasks());
        let codes: Vec<&str> = plan.iter().map(|(c, _)| c.as_str()).collect();
        assert_eq!(
            codes,
            vec![
                "chat_3_times",
                "expert_use",
                "share_invite",
                "desktop_chat_1_time"
            ]
        );
        assert!(
            !codes.contains(&"task_student_verify"),
            "人工任务 task_student_verify 必须被跳过"
        );
        assert!(!codes.contains(&"unknown_future_task"));
        assert_eq!(plan[0].1, SchoolAction::MiniChat);
        assert_eq!(plan[1].1, SchoolAction::Expert);
        assert_eq!(plan[2].1, SchoolAction::Share);
        assert_eq!(plan[3].1, SchoolAction::DesktopSeq);
    }

    #[test]
    fn out_of_period_skips_everything_and_is_success() {
        // in_period=false → 阶段为 Skip，计划全量跳过，汇总标记为成功（status=ok）。
        assert_eq!(school_phase(false), SchoolPhase::Skip);
        assert_eq!(school_phase(true), SchoolPhase::Run);
        let summary = school_skip_summary();
        assert_eq!(summary["status"], "ok");
        assert_eq!(summary["skipped"], "not_in_period");
        assert_eq!(summary["accounts"], json!([]));
    }

    #[test]
    fn school_action_table_maps_known_codes_only() {
        assert_eq!(
            school_task_action("chat_3_times"),
            Some(SchoolAction::MiniChat)
        );
        assert_eq!(school_task_action("expert_use"), Some(SchoolAction::Expert));
        assert_eq!(
            school_task_action("share_invite"),
            Some(SchoolAction::Share)
        );
        assert_eq!(
            school_task_action("desktop_chat_1_time"),
            Some(SchoolAction::DesktopSeq)
        );
        assert_eq!(school_task_action("task_student_verify"), None);
        assert_eq!(school_task_action("nope"), None);
    }

    #[test]
    fn report_events_carry_activity_id() {
        let mini = mini_chat_event("uid-1", "wbmp-1", 1);
        assert_eq!(mini["activityId"], SCHOOL_ACTIVITY_ID);
        assert_eq!(mini["userId"], "uid-1");
        let expert = expert_event("uid-1", "wbexp-1", 1);
        assert_eq!(expert["activityId"], SCHOOL_ACTIVITY_ID);
        assert_eq!(expert["eventCode"], "expert_actual_use");
        let seq = desktop_seq_events("uid-1", "wbdesk-1", 1);
        assert_eq!(seq.len(), 6);
        assert!(seq.iter().all(|e| e["activityId"] == SCHOOL_ACTIVITY_ID));
        assert_eq!(seq[0]["eventCode"], "agent_task_created");
        assert_eq!(seq[5]["eventCode"], "chat_request_response");
    }

    // -----------------------------------------------------------------------
    // 可测性缝：驱动**真实入口** run_school_cycle_for_with，注入桩 IO（绝不发网络）。
    // -----------------------------------------------------------------------

    /// 活动下线桩：任何请求都返回 `in_period=false` 的空任务列表。
    struct OfflineIo;

    impl SchoolIo for OfflineIo {
        fn request<'a>(
            &'a self,
            _region: Region,
            _method: &'a str,
            _path: &'a str,
            _body: Option<Value>,
            _account: &'a Value,
        ) -> Pin<Box<dyn Future<Output = SchoolResponse> + Send + 'a>> {
            Box::pin(async move {
                (
                    200,
                    json!({"code": 0, "data": {"in_period": false, "tasks": []}}),
                )
            })
        }
    }

    /// 关键回归：活动下线时**真实入口**必须走 `school_skip_summary()` 分支并原样返回。
    ///
    /// 这条用例是「断言挂接产线」的证明——若把 `run_school_cycle_for_with` 里的
    /// `if school_phase(in_period) == SchoolPhase::Skip` 改为 `if false`，
    /// 该账号会走进行期流程，最终返回 `{"status":"ok","accounts":[ … ]}`，与本断言不符而**变红**。
    #[tokio::test]
    async fn offline_activity_drives_real_entry_to_skip_summary() {
        // 账号无 refresh token → ensure_fresh_token_for 不发刷新请求（无网络）。
        let accounts = vec![json!({
            "uid": "u-offline",
            "email": "offline@example.com",
            "access_token": "at-token",
        })];
        let out = run_school_cycle_for_with(Region::Cn, accounts, &OfflineIo).await;
        // 活动下线 → 全量跳过、正常成功；且与单一来源函数**逐字一致**。
        assert_eq!(out, school_skip_summary());
    }

    /// 对照：进行期账号（有任务）不会被误判为下线——真实入口返回 accounts 明细而非 skip 汇总。
    #[tokio::test]
    async fn in_period_activity_does_not_return_skip_summary() {
        struct InPeriodIo;
        impl SchoolIo for InPeriodIo {
            fn request<'a>(
                &'a self,
                _region: Region,
                _method: &'a str,
                _path: &'a str,
                _body: Option<Value>,
                _account: &'a Value,
            ) -> Pin<Box<dyn Future<Output = SchoolResponse> + Send + 'a>> {
                // 任务列表：已 claimed 的 chat_3_times（避免触发写动作）；config 返回 in_period=true 且余额 0。
                Box::pin(async move {
                    (
                        200,
                        json!({
                            "code": 0,
                            "data": {
                                "in_period": true,
                                "tasks": [
                                    {"task_code": "chat_3_times", "status": "claimed", "target_count": 3}
                                ],
                                "chance": {"balance": 0}
                            }
                        }),
                    )
                })
            }
        }
        let accounts = vec![json!({
            "uid": "u-in-period",
            "email": "inperiod@example.com",
            "access_token": "at-token",
        })];
        let out = run_school_cycle_for_with(Region::Cn, accounts, &InPeriodIo).await;
        assert_ne!(out, school_skip_summary(), "进行期不得返回下线汇总");
        assert_eq!(out["status"], "ok");
        assert_eq!(out["accounts"][0]["result"], "ok");
    }

    // -----------------------------------------------------------------------
    // C14 / C15：转盘 409 no chance 属正常态 + chance/prizes 字段健壮性。
    // 均驱动真实入口 run_school_cycle_for_with，注入按路径分发的桩 IO（不发网络）。
    // -----------------------------------------------------------------------

    /// 按路径分发的桩 IO：任务列表（进行期、空任务）+ 自定义 config 响应 + 转盘响应。
    struct ScriptedIo {
        config: Value,
        wheel: SchoolResponse,
    }

    impl SchoolIo for ScriptedIo {
        fn request<'a>(
            &'a self,
            _region: Region,
            method: &'a str,
            path: &'a str,
            _body: Option<Value>,
            _account: &'a Value,
        ) -> Pin<Box<dyn Future<Output = SchoolResponse> + Send + 'a>> {
            Box::pin(async move {
                match (method, path) {
                    ("GET", p) if p == TASKS_PATH => (
                        200,
                        json!({"code": 0, "data": {"in_period": true, "tasks": []}}),
                    ),
                    ("GET", p) if p == CONFIG_PATH => (200, self.config.clone()),
                    ("POST", p) if p == WHEEL_PATH => self.wheel.clone(),
                    _ => (200, json!({"code": 0, "data": {}})),
                }
            })
        }
    }

    fn one_account(uid: &str) -> Vec<Value> {
        vec![json!({"uid": uid, "email": format!("{uid}@example.com"), "access_token": "at"})]
    }

    /// C14：转盘返回 HTTP 409 + code=40900 "no chance" 必须被视为**正常态**（不计失败）。
    #[tokio::test]
    async fn lottery_409_no_chance_is_normal_not_failure() {
        let io = ScriptedIo {
            config: json!({"code": 0, "data": {"in_period": true, "chance": {"balance": 1}}}),
            wheel: (409, json!({"code": 40900, "msg": "no chance"})),
        };
        let out = run_school_cycle_for_with(Region::Cn, one_account("u-409"), &io).await;
        assert_eq!(out["status"], "ok", "{out}");
        let lottery = &out["accounts"][0]["lottery"];
        assert_ne!(
            lottery["result"], "error",
            "409 no chance 不得记为失败: {out}"
        );
        assert_eq!(lottery["result"], "ok", "{out}");
        assert_eq!(lottery["drawn"], 0, "无次数不得抽出任何奖: {out}");
    }

    /// C15：config 的 `chance` / `prizes` 缺失、`null`、类型异常时**不得 panic**，须可读降级。
    #[tokio::test]
    async fn malformed_chance_and_prizes_degrade_without_panic() {
        let cases = vec![
            // chance 缺失
            json!({"code": 0, "data": {"in_period": true}}),
            // chance 为 null
            json!({"code": 0, "data": {"in_period": true, "chance": null}}),
            // chance / prizes 类型异常（字符串）
            json!({"code": 0, "data": {"in_period": true, "chance": "oops", "prizes": "not-array"}}),
            // balance 类型异常 + prizes 元素异常
            json!({"code": 0, "data": {"in_period": true, "chance": {"balance": "many"}, "prizes": [null, 3, {"x": 1}]}}),
        ];
        for config in cases {
            let io = ScriptedIo {
                config: config.clone(),
                wheel: (200, json!({"code": 0, "data": {"chance_balance": 0}})),
            };
            let out = run_school_cycle_for_with(Region::Cn, one_account("u-mal"), &io).await;
            assert_eq!(out["status"], "ok", "cfg={config}");
            assert_eq!(
                out["accounts"][0]["lottery"]["result"], "ok",
                "cfg={config}"
            );
        }
    }
}
