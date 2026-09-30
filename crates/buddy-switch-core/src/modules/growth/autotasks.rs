//! 19 个可自动任务动作 + 「一键完成全部」（对照 panel `autotask.go`，顺序=依赖序）。
//!
//! 语义与 panel 一致：
//! - 所有动作幂等：已 claimed / 已达标 直接跳过，不重复消耗上游配额
//! - mp 口径任务（school_season / Sequential 族）accept 带**登记回读验证**（2 次尝试），
//!   因上游存在 200+OK 但 accept 未真正登记的形态
//! - 达标回读走 `task_by_code_waiting`（4×3s 有界轮询，上游异步计分）
//! - 单账号任务动作互斥（与一键完成、队列共用 per-account 锁）

use std::collections::HashMap;
use std::sync::{Mutex as StdMutex, OnceLock};
use std::time::Duration;

use serde_json::{json, Value};

use crate::modules::config::now_ms;
use crate::modules::region::Region;

use super::events::{
    automation_create_event, appearance_skin_apply_event, buddy_app_sequence,
    design_canvas_sequence, desktop_chat_sequence, desktop_events_array, expert_actual_use_event,
    expert_actual_use_local, expert_summon_sequence, library_read_event, mini_chat_model_event,
    mini_expert_use_event, mini_playbook_events, playbook_prompt_sequence, school_chat_times_event,
    school_season_chat_event, skill_info_event, template_use_sequence, MarketExpert,
    TEMPLATE_GROUPS, TEMPLATE_NAMES,
};
use super::tasks::{
    accept_tasks, accept_tasks_mp, acc_enterprise_id, acc_nickname, acc_uid, buddy_first,
    claim_task, is_mp_task_code, list_tasks, real_chat, real_chat_model, report_chat_event,
    report_desktop_events, report_mp_events, report_web_event, set_appearance_theme,
    task_by_code, task_by_code_mp, task_by_code_waiting, ACTION_GAP, GrowthIo, GrowthTask,
};

// ---------------------------------------------------------------------------
// 动作表（顺序即执行顺序：先解锁依赖项）
// ---------------------------------------------------------------------------

/// 一个可自动化的任务动作（task_code, 说明, 是否尝试型）。
pub struct AutoAction {
    pub code: &'static str,
    pub desc: &'static str,
    /// 尝试型：上游未证实可脚本化（窗口外/未解锁时跑了可能不点亮）。
    pub attempt: bool,
}

/// 19 个动作（panel `autoActions` 同序同集）。
pub const AUTO_ACTIONS: [AutoAction; 19] = [
    AutoAction { code: "chat_5", desc: "上报 5 条对话活跃事件（自动补足差额）", attempt: false },
    AutoAction { code: "first_buddy", desc: "上报解锁 → 同意协议 → 领取第一只 Buddy（+300 分）", attempt: false },
    AutoAction { code: "Model_chat_GLM5.2", desc: "接受任务 → glm-5.2 真实对话一次 → 对齐模型上报", attempt: false },
    AutoAction { code: "RichMeow_Chat", desc: "桌面指纹事件链上报（纯 API 可点亮）", attempt: false },
    AutoAction { code: "Buddy_App", desc: "上报「进入 Buddy 应用」事件链", attempt: false },
    AutoAction { code: "Buddy_App_QQ", desc: "上报「进入企鹅教师助手」事件链", attempt: false },
    AutoAction { code: "automation_1", desc: "上报「定时任务创建」事件", attempt: false },
    AutoAction { code: "Library_read", desc: "上报「读资料库介绍」事件", attempt: false },
    AutoAction { code: "template_5", desc: "上报「使用模板创建任务」事件组 ×5", attempt: false },
    AutoAction { code: "playbook_prompt", desc: "上报「灵感案例做同款发送 Prompt」事件组", attempt: false },
    AutoAction { code: "create_canvas", desc: "上报「设计创意画布创建」事件组（+300 分）", attempt: false },
    AutoAction { code: "expert_5", desc: "真实专家召唤+使用链 ×5", attempt: false },
    AutoAction { code: "Expert_team_use_3", desc: "真实专家团召唤+使用链 ×3", attempt: false },
    AutoAction { code: "Hp_Appearance", desc: "设置主题 API + 皮肤生效事件", attempt: false },
    AutoAction { code: "skill_1", desc: "真实对话 + skill_info 技能加载事件", attempt: false },
    AutoAction { code: "Expert_lighthouse", desc: "真实轻量云专家召唤+使用链", attempt: false },
    AutoAction { code: "black_cat", desc: "夜猫子：23:00–08:00 窗口内 glm-5.2 对话补足", attempt: true },
    AutoAction { code: "school_season", desc: "校园日（小程序口径）：accept → mini 对话+activityId 上报 → 领奖", attempt: false },
    AutoAction { code: "Sequential_Tasks_1", desc: "小程序首对话（小程序口径）", attempt: false },
];

/// 全部任务码（含 Sequential_Tasks_2..7，供队列/扫描判「可自动化」）。
pub const ALL_TASK_CODES: &[&str] = &[
    "chat_5",
    "first_buddy",
    "Model_chat_GLM5.2",
    "RichMeow_Chat",
    "Buddy_App",
    "Buddy_App_QQ",
    "automation_1",
    "Library_read",
    "template_5",
    "playbook_prompt",
    "create_canvas",
    "expert_5",
    "Expert_team_use_3",
    "Hp_Appearance",
    "skill_1",
    "Expert_lighthouse",
    "black_cat",
    "school_season",
    "Sequential_Tasks_1",
    "Sequential_Tasks_2",
    "Sequential_Tasks_3",
    "Sequential_Tasks_4",
    "Sequential_Tasks_5",
    "Sequential_Tasks_6",
    "Sequential_Tasks_7",
];

const UNKNOWN_ACTION_INDEX: usize = 1 << 20;

/// 任务在动作表中的顺序（队列执行按依赖序排；未知返回大值，排最后）。
pub fn auto_action_index(code: &str) -> usize {
    for (i, c) in ALL_TASK_CODES.iter().enumerate() {
        if c.trim() == code.trim() {
            return i;
        }
    }
    UNKNOWN_ACTION_INDEX
}

/// 该任务码是否有可执行动作。
pub fn has_auto_action(code: &str) -> bool {
    auto_action_index(code) < UNKNOWN_ACTION_INDEX
}

/// 动作说明文案（展示用）。
pub fn auto_action_desc(code: &str) -> Option<&'static str> {
    AUTO_ACTIONS.iter().find(|a| a.code == code).map(|a| a.desc)
}

/// 是否尝试型动作。
pub fn is_attempt_action(code: &str) -> bool {
    AUTO_ACTIONS.iter().any(|a| a.code == code && a.attempt)
}

// ---------------------------------------------------------------------------
// per-account 互斥（单任务 / 一键完成 / 队列 共用；防同账号并发重跑）
// ---------------------------------------------------------------------------

/// 账号忙闲登记：`0` = 空闲，`1` = 有动作在跑。
static ACCOUNT_LOCKS: OnceLock<StdMutex<HashMap<String, StdMutex<u32>>>> = OnceLock::new();

fn locks() -> &'static StdMutex<HashMap<String, StdMutex<u32>>> {
    ACCOUNT_LOCKS.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// 尝试获取账号锁（互斥）；已被占用返回 None。
pub fn try_lock_account(uid: &str) -> Option<()> {
    let mut map = locks().lock().ok()?;
    let slot = map.entry(uid.to_string()).or_insert(StdMutex::new(0));
    let mut guard = slot.try_lock().ok()?;
    if *guard != 0 {
        return None;
    }
    *guard = 1;
    Some(())
}

/// 释放账号锁。
pub fn unlock_account(uid: &str) {
    if let Ok(map) = locks().lock() {
        if let Some(slot) = map.get(uid) {
            if let Ok(mut guard) = slot.lock() {
                *guard = 0;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 上下文 + 结果
// ---------------------------------------------------------------------------

/// 单账号动作执行上下文。
pub struct ActionCtx<'a> {
    pub io: &'a dyn GrowthIo,
    pub region: Region,
    pub account: &'a Value,
    pub uid: String,
    pub nickname: String,
    pub enterprise_id: String,
}

impl<'a> ActionCtx<'a> {
    pub fn new(io: &'a dyn GrowthIo, region: Region, account: &'a Value) -> Self {
        Self {
            io,
            region,
            account,
            uid: acc_uid(account),
            nickname: acc_nickname(account),
            enterprise_id: acc_enterprise_id(account),
        }
    }

    pub fn now_ms(&self) -> i64 {
        now_ms()
    }

    fn report_desktop(
        &self,
        events: &[(String, Value)],
    ) -> impl std::future::Future<Output = Result<(), String>> + '_ {
        let arr = desktop_events_array(&self.uid, &self.nickname, self.now_ms(), events);
        report_desktop_events(self.io, self.region, self.account, arr)
    }

    fn report_mp(&self, events: &[Value]) -> impl std::future::Future<Output = Result<(), String>> + '_ {
        let arr = super::events::mp_events_array(&self.uid, &self.nickname, self.now_ms(), events);
        report_mp_events(self.io, self.region, self.account, arr)
    }
}

/// 任务进度的可读表示（回读对比用；对照 Go taskProgressText）。
pub fn task_progress_text(t: Option<&GrowthTask>) -> String {
    match t {
        None => "?".into(),
        Some(t) => {
            if t.target > 0 {
                format!("{}/{}", t.current, t.target)
            } else if t.claimed {
                "claimed".into()
            } else {
                t.accept_status.clone()
            }
        }
    }
}

fn suffix8(s: &str) -> &str {
    &s[s.len().saturating_sub(8)..]
}

/// mp 任务写动作间隔（accept/上报/领奖之间，防频控；panel 2s 口径）。
const MP_ACTION_GAP: Duration = Duration::from_secs(2);
/// 专家召唤链间隔（真实使用节奏；panel 6s 口径）。
const EXPERT_SUMMON_GAP: Duration = Duration::from_secs(6);
/// 夜猫子单次对话间隔（panel 4s 口径）。
const NIGHT_CHAT_GAP: Duration = Duration::from_secs(4);

// ---------------------------------------------------------------------------
// 单任务动作入口
// ---------------------------------------------------------------------------

/// 一键完成单个任务（panel `accountTaskAuto` 语义）：
/// 前置读 → 执行 → 回读 → 达标自动领奖。结果 JSON：
/// `{ok, skipped?, message, progress_before, progress_after?, claimable?, attempt?,
///  claimed?, credit?, energy?, claim_error?}`。
///
/// 失败（无动作 / 无任务 / 执行硬错误）返回 Err(可读文案)。
pub async fn run_auto_action(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    code: &str,
) -> Result<Value, String> {
    if region == Region::Global {
        return Err("成长任务体系仅 CN（该区域无成长任务）".into());
    }
    let code = code.trim();
    if auto_action_index(code) >= UNKNOWN_ACTION_INDEX {
        return Err("该任务需要客户端内交互（无对应接口），无法自动完成；请按任务说明在官方客户端操作".into());
    }
    let uid = acc_uid(account);
    if try_lock_account(&uid).is_none() {
        return Err("该账号有任务动作正在执行中，请等本轮结束后再试".into());
    }
    let result = run_auto_action_inner(io, region, account, code).await;
    unlock_account(&uid);
    result
}

/// 内部执行（不拿账号锁；队列路径已持有锁时直接调用）。
pub async fn run_auto_action_inner(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
    code: &str,
) -> Result<Value, String> {
    let ctx = ActionCtx::new(io, region, account);
    let before = task_by_code(io, region, account, code).await?;
    let Some(before) = before else {
        return Err("该账号没有此任务".into());
    };
    if before.claimed {
        return Ok(json!({
            "ok": true,
            "skipped": true,
            "message": "该任务已领取过奖励"
        }));
    }

    let exec = dispatch_run(&ctx, code).await;
    let msg = match exec {
        Ok(m) => m,
        Err(e) => return Err(format!("执行失败: {e}")),
    };

    // 回读验证：上报 200 ≠ 计分（上游可能静默丢弃 + 计分异步）。
    let is_mp = is_mp_task_code(code);
    let after: Option<GrowthTask> = if is_mp {
        task_by_code_mp(io, region, account, code).await.unwrap_or(None)
    } else {
        task_by_code_waiting(io, region, account, code).await.unwrap_or(None)
    };

    let mut resp = json!({
        "ok": true,
        "message": msg,
        "progress_before": task_progress_text(Some(&before)),
        "progress_after": task_progress_text(after.as_ref()),
        "claimable": after.as_ref().map(|t| t.claimable).unwrap_or(false),
        "attempt": is_attempt_action(code),
        "verify_supported": true
    });

    // 达标即自动领奖（把「完成→领奖」收敛成一步）。
    if let Some(t) = &after {
        if t.claimable {
            match claim_task(io, region, account, code).await {
                Ok(c) => {
                    let credit = c["credit"].as_i64().unwrap_or(0);
                    let energy = c["energy"].as_i64().unwrap_or(0);
                    resp["claimed"] = json!(true);
                    resp["credit"] = json!(credit);
                    resp["energy"] = json!(energy);
                    if credit > 0 || energy > 0 {
                        resp["message"] =
                            json!(format!("{msg}；已自动领奖 +{credit} 分 +{energy} 能"));
                    } else {
                        resp["message"] = json!(format!("{msg}；奖励此前已领取"));
                    }
                }
                Err(e) => {
                    resp["claim_error"] = json!(e);
                    resp["message"] = json!(format!("{msg}；达标但领奖失败，可在任务列表手动点「领取」重试"));
                }
            }
        }
    }
    Ok(resp)
}

// ---------------------------------------------------------------------------
// 动作分发
// ---------------------------------------------------------------------------

async fn dispatch_run(ctx: &ActionCtx<'_>, code: &str) -> Result<String, String> {
    match code {
        "chat_5" => run_chat_5(ctx).await,
        "first_buddy" => run_first_buddy(ctx).await,
        "Model_chat_GLM5.2" => run_model_chat(ctx).await,
        "RichMeow_Chat" => run_rich_meow(ctx).await,
        "Buddy_App" | "Buddy_App_QQ" => run_buddy_app(ctx).await,
        "automation_1" => run_automation_create(ctx).await,
        "Library_read" => run_library_read(ctx).await,
        "template_5" => run_template_use(ctx).await,
        "playbook_prompt" => run_playbook_prompt(ctx).await,
        "create_canvas" => run_create_canvas(ctx).await,
        "expert_5" => run_expert_batch(ctx, "agent", 5).await,
        "Expert_team_use_3" => run_expert_batch(ctx, "team", 3).await,
        "Hp_Appearance" => run_appearance(ctx).await,
        "skill_1" => run_skill_fresh(ctx).await,
        "Expert_lighthouse" => run_expert_lighthouse(ctx).await,
        "black_cat" => run_black_cat(ctx).await,
        "school_season" => run_mp_mini_chat(ctx, "school_season", true).await,
        "Sequential_Tasks_1" => run_mp_mini_chat(ctx, "Sequential_Tasks_1", false).await,
        "Sequential_Tasks_3" => run_mp_mini_chat(ctx, "Sequential_Tasks_3", false).await,
        "Sequential_Tasks_6" => run_mp_mini_chat(ctx, "Sequential_Tasks_6", false).await,
        "Sequential_Tasks_2" => run_mini_expert(ctx).await,
        "Sequential_Tasks_4" => run_sequential(ctx, "Sequential_Tasks_4", SequentialVariant::Automation).await,
        "Sequential_Tasks_5" => run_sequential(ctx, "Sequential_Tasks_5", SequentialVariant::ModelChat).await,
        "Sequential_Tasks_7" => run_sequential(ctx, "Sequential_Tasks_7", SequentialVariant::Playbook).await,
        _ => Err(format!("未实现的动作: {code}")),
    }
}

// ---------------------------------------------------------------------------
// 纯上报组
// ---------------------------------------------------------------------------

/// chat_5：按差额补报 chat_request_send（billing 域）。
async fn run_chat_5(ctx: &ActionCtx<'_>) -> Result<String, String> {
    let t = task_by_code(ctx.io, ctx.region, ctx.account, "chat_5")
        .await
        .map_err(|e| e)?
        .ok_or_else(|| "任务不存在".to_string())?;
    let target = if t.target <= 0 { 5 } else { t.target };
    let need = target.saturating_sub(t.current) as u64;
    if need == 0 {
        return Ok("进度已达标，无需上报".into());
    }
    let now = ctx.now_ms();
    let mut done = 0u64;
    for i in 0..need {
        let cid = format!("buddyswitch-chat5-{now}-{i}");
        if let Err(e) = report_chat_event(ctx.io, ctx.region, ctx.account, &cid, &cid, "", "").await {
            return Ok(format!("上报第 {}/{} 条失败: {e}", i + 1, need));
        }
        done += 1;
        if i + 1 < need {
            tokio::time::sleep(ACTION_GAP).await;
        }
    }
    Ok(format!("已补报 {done} 条对话事件"))
}

/// first_buddy：前置活跃上报 → 同意协议 → 领养（门槛未过为正常态）。
async fn run_first_buddy(ctx: &ActionCtx<'_>) -> Result<String, String> {
    let now = ctx.now_ms();
    let cid = format!("buddyswitch-adopt-{now}");
    report_chat_event(ctx.io, ctx.region, ctx.account, &cid, &cid, "", "").await?;
    tokio::time::sleep(ACTION_GAP).await; // 给上游事件处理留时间
    match buddy_first(ctx.io, ctx.region, ctx.account).await {
        Ok(Some(())) => Ok("已领取 Buddy（+300 分 +8 能量）".into()),
        Ok(None) => Ok("前置已上报，但领养门槛未过（上游要求当日活跃），请稍后重试".into()),
        Err(e) => Err(format!("领取 Buddy: {e}")),
    }
}

/// Model_chat_GLM5.2：accept（失败不阻塞）→ glm-5.2 真实对话 → 对齐模型上报。
async fn run_model_chat(ctx: &ActionCtx<'_>) -> Result<String, String> {
    const CODE: &str = "Model_chat_GLM5.2";
    const MODEL: &str = "glm-5.2";
    const MODEL_NAME: &str = "GLM-5.2";
    let _ = accept_tasks(ctx.io, ctx.region, ctx.account, &[CODE.to_string()]).await; // 报名不阻塞主链路
    tokio::time::sleep(ACTION_GAP).await;
    match real_chat_model(ctx.io, ctx.region, ctx.account, MODEL).await {
        Ok(_) => {}
        Err(e) => return Err(format!("对话失败: {e}")),
    }
    tokio::time::sleep(ACTION_GAP).await;
    let now = ctx.now_ms();
    let cid = format!("buddyswitch-glm52-{now}");
    if let Err(e) =
        report_chat_event(ctx.io, ctx.region, ctx.account, &cid, &cid, MODEL, MODEL_NAME).await
    {
        return Ok(format!("对话已完成，但进度上报失败：{e}"));
    }
    Ok("已完成 glm-5.2 对话并上报".into())
}

/// RichMeow_Chat：桌面指纹完整对话事件链（纯 API 可点亮）。
async fn run_rich_meow(ctx: &ActionCtx<'_>) -> Result<String, String> {
    let now = ctx.now_ms();
    let conv = format!("buddyswitch-rm-{now}");
    let req = format!("buddyswitch-rm-req-{now}");
    let events = desktop_chat_sequence(&conv, &req, &format!("req-{now}-user"), "fast-model", "fast-model");
    ctx.report_desktop(&events).await?;
    Ok("已按桌面端指纹上报完整对话事件链（agent_task_created→chat_response）".into())
}

/// Buddy_App / Buddy_App_QQ：buddyapp 五连事件（同一组事件同时覆盖两个任务）。
async fn run_buddy_app(ctx: &ActionCtx<'_>) -> Result<String, String> {
    let events = buddy_app_sequence("cb_y5Dy46tPQGGWtueMxXbe", "企鹅教师助手");
    ctx.report_desktop(&events).await?;
    Ok("已上报 buddyapp 进入五连事件（同时覆盖 Buddy_App 与 Buddy_App_QQ）".into())
}

/// automation_1：定时任务创建事件。
async fn run_automation_create(ctx: &ActionCtx<'_>) -> Result<String, String> {
    let events = [automation_create_event("buddyswitch 自动化")];
    ctx.report_desktop(&events).await?;
    Ok("已上报定时任务创建事件".into())
}

/// Library_read：web 域资料库介绍阅读事件。
async fn run_library_read(ctx: &ActionCtx<'_>) -> Result<String, String> {
    let ev = library_read_event(&ctx.uid, &ctx.nickname, &ctx.enterprise_id, ctx.now_ms());
    report_web_event(ctx.io, ctx.region, ctx.account, ev).await?;
    Ok("已上报资料库介绍阅读事件".into())
}

/// template_5：5 组模板使用事件（template_id 服务端不校验真实性）。
async fn run_template_use(ctx: &ActionCtx<'_>) -> Result<String, String> {
    let now = ctx.now_ms();
    for i in 0..5 {
        let conv = format!("buddyswitch-tpl-{now}-{i}");
        let req = format!("buddyswitch-tpl-req-{now}-{i}");
        let events = template_use_sequence(&conv, &req, TEMPLATE_GROUPS[i], TEMPLATE_NAMES[i]);
        if let Err(e) = ctx.report_desktop(&events).await {
            return Ok(format!("第 {} 组模板事件上报失败: {e}", i + 1));
        }
        if i < 4 {
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }
    Ok("已上报 template_used ×5".into())
}

/// playbook_prompt：灵感案例「做同款发送 Prompt」事件组。
async fn run_playbook_prompt(ctx: &ActionCtx<'_>) -> Result<String, String> {
    let now = ctx.now_ms();
    let conv = format!("buddyswitch-pb-{now}");
    let req = format!("buddyswitch-pb-req-{now}");
    let events = playbook_prompt_sequence(&conv, &req, "pm-gtm-launch-plan", "新产品上市 GTM 发布计划一页纸");
    ctx.report_desktop(&events).await?;
    Ok("已上报 playbook_cta_click + playbook_prompt_send".into())
}

/// create_canvas：设计创意画布创建事件组。
async fn run_create_canvas(ctx: &ActionCtx<'_>) -> Result<String, String> {
    let now = ctx.now_ms();
    let conv = format!("buddyswitch-canvas-{now}");
    let req = format!("buddyswitch-canvas-req-{now}");
    let events = design_canvas_sequence(&conv, &req);
    ctx.report_desktop(&events).await?;
    Ok("已上报 wbx_design_canvas_task_create/open".into())
}

/// Hp_Appearance：设置主题 API 留痕 + 皮肤生效事件。
async fn run_appearance(ctx: &ActionCtx<'_>) -> Result<String, String> {
    const THEME_KEY: &str = "theme-tkmw7j";
    if let Err(e) = set_appearance_theme(ctx.io, ctx.region, ctx.account, THEME_KEY).await {
        return Err(format!("设置主题: {e}"));
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let events = [appearance_skin_apply_event(THEME_KEY)];
    ctx.report_desktop(&events).await?;
    Ok("已设置主题并上报皮肤生效事件".into())
}

// ---------------------------------------------------------------------------
// 真实对话组（SSE）
// ---------------------------------------------------------------------------

/// skill_1：真实对话 + skill_info 技能加载事件（JOIN 服务端 requestId）。
async fn run_skill_fresh(ctx: &ActionCtx<'_>) -> Result<String, String> {
    let (conv, req) = real_chat(ctx.io, ctx.region, ctx.account, "").await?;
    let msg_id = format!("msg-{}", suffix8(&req));
    let mut events = desktop_chat_sequence(&conv, &req, &msg_id, "fast-model", "fast-model");
    // 技能加载语义：模型发起工具调用。
    for (code, ev) in &mut events {
        if code == "chat_message_response" {
            if let Value::Object(m) = ev {
                m.insert("finishReason".into(), json!("tool_calls"));
            }
        }
    }
    events.push(skill_info_event(
        &conv,
        &req,
        &msg_id,
        "润泽小馆·日报撰写",
        "skill_2097350077599879168",
        "1.0.0",
    ));
    ctx.report_desktop(&events).await?;
    Ok("已上报真实对话 + skill_info 技能加载事件".into())
}

/// Expert_lighthouse：轻量云专家（固定 id；市场列表命中则用其真实字段）。
async fn run_expert_lighthouse(ctx: &ActionCtx<'_>) -> Result<String, String> {
    const LH_ID: &str = "ex_2cvvUZQhDyeJ";
    let mut lh = MarketExpert {
        expert_id: LH_ID.to_string(),
        expert_type: "agent".into(),
        display_name_zh: "腾讯轻量云专家".into(),
        profession_zh: "腾讯轻量云专家".into(),
        version: "1.0.2".into(),
    };
    if let Ok(experts) = real_chat_market_guard(ctx, "agent").await {
        if let Some(found) = experts.into_iter().find(|e| e.expert_id == LH_ID) {
            lh = found;
        }
    }
    let summon = expert_summon_sequence(&lh);
    ctx.report_desktop(&summon).await?;

    let (conv, req) = real_chat(ctx.io, ctx.region, ctx.account, &lh.expert_id).await?;
    let msg_id = format!("msg-{}", suffix8(&req));
    let mut events = desktop_chat_sequence(&conv, &req, &msg_id, "fast-model", "fast-model");
    for (code, ev) in &mut events {
        if code == "agent_task_created" {
            if let Value::Object(m) = ev {
                m.insert("has_expert".into(), json!(true));
                m.insert("expert_id".into(), json!(lh.expert_id));
                m.insert("expert_name".into(), json!(lh.display()));
                m.insert("expert_industry_id".into(), json!(""));
            }
        }
    }
    events.push(expert_actual_use_local(&lh, &conv, &req));
    ctx.report_desktop(&events).await?;
    Ok("已上报轻量云专家召唤+使用链（真实对话 requestId）".into())
}

/// 专家召唤 + 真实对话 ×N（expert_5 / Expert_team_use_3 共用）。
async fn run_expert_batch(ctx: &ActionCtx<'_>, expert_type: &str, count: u32) -> Result<String, String> {
    let experts = real_chat_market_guard(ctx, expert_type).await?;
    if experts.is_empty() {
        return Err("专家市场列表为空".into());
    }
    let mut ok = 0u32;
    for i in 0..experts.len() {
        if ok >= count {
            break;
        }
        let e = &experts[i];
        let summon = expert_summon_sequence(e);
        if ctx.report_desktop(&summon).await.is_err() {
            continue;
        }
        let (conv, req) = match real_chat(ctx.io, ctx.region, ctx.account, &e.expert_id).await {
            Ok(v) => v,
            Err(_) => continue,
        };
        let msg_id = format!("msg-{}", suffix8(&req));
        let mut events = desktop_chat_sequence(&conv, &req, &msg_id, "fast-model", "fast-model");
        events.push(expert_actual_use_event(e, &conv, &req));
        if ctx.report_desktop(&events).await.is_err() {
            continue;
        }
        ok += 1;
        if i < experts.len() - 1 {
            tokio::time::sleep(EXPERT_SUMMON_GAP).await;
        }
    }
    Ok(format!("已对 {ok} 位真实专家完成召唤+使用链（类型 {expert_type}）"))
}

/// 拉取专家市场真实列表（expert_actual_use 判据要求 id 真实存在）。
async fn real_chat_market_guard(ctx: &ActionCtx<'_>, expert_type: &str) -> Result<Vec<MarketExpert>, String> {
    super::tasks::market_expert_list(ctx.io, ctx.region, ctx.account, expert_type)
        .await
        .map_err(|e| format!("拉取专家列表: {e}"))
}

/// black_cat（夜猫子）：仅 23:00–08:00（CST）计数窗口内做；窗口外是尝试型的正常提示。
async fn run_black_cat(ctx: &ActionCtx<'_>) -> Result<String, String> {
    if !crate::modules::cst::in_night_window(ctx.now_ms()) {
        return Ok("当前不在 23:00–08:00 计数窗口，行为不计分；可在夜间排程自动补足".into());
    }
    let need = match task_by_code(ctx.io, ctx.region, ctx.account, "black_cat").await {
        Ok(Some(t)) if !t.claimed && t.target > t.current => t.target - t.current,
        Ok(Some(_)) => 0,
        Ok(None) => return Ok("进度已达标，无需补足".into()),
        Err(e) => return Err(e),
    };
    if need <= 0 {
        return Ok("进度已达标，无需补足".into());
    }
    let now = ctx.now_ms();
    let mut done = 0i64;
    for i in 0..need {
        if real_chat_model(ctx.io, ctx.region, ctx.account, "glm-5.2").await.is_err() {
            return Ok(format!("完成 {done}/{need} 次后中断"));
        }
        let cid = format!("buddyswitch-night-{now}-{i}");
        if report_chat_event(ctx.io, ctx.region, ctx.account, &cid, &cid, "glm-5.2", "GLM-5.2")
            .await
            .is_err()
        {
            return Ok(format!("完成 {done}/{need} 次后中断"));
        }
        done += 1;
        if i + 1 < need {
            tokio::time::sleep(NIGHT_CHAT_GAP).await;
        }
    }
    Ok(format!("已完成 {done} 次夜间对话并上报"))
}

// ---------------------------------------------------------------------------
// 小程序（mp）口径任务组
// ---------------------------------------------------------------------------

/// mp accept 带登记回读验证（上游 200+OK 但未落账形态；2 次尝试）。
async fn accept_with_verify_mp(ctx: &ActionCtx<'_>, code: &str) -> bool {
    for attempt in 1..=2 {
        if accept_tasks_mp(ctx.io, ctx.region, ctx.account, &[code.to_string()]).await.is_err() {
            eprintln!("[growth] {code} accept 尝试{attempt} 失败");
            continue;
        }
        tokio::time::sleep(MP_ACTION_GAP).await;
        match task_by_code_mp(ctx.io, ctx.region, ctx.account, code).await {
            Ok(Some(t)) if !t.accept_status.is_empty() && t.accept_status != "not_accepted" => {
                return true;
            }
            other => {
                eprintln!(
                    "[growth] {code} accept 尝试{attempt} 未登记生效（回读={:?}）",
                    other.ok().as_ref().and_then(|o| o.as_ref()).map(|t| t.accept_status.clone())
                );
            }
        }
    }
    false
}

/// mp 限定任务通用闭环（school_season / Sequential_Tasks_1/3/6）：
/// mp 查询 → accept（带验证）→ mini 对话事件补差上报 → 回读 → 达标即领奖。
async fn run_mp_mini_chat(
    ctx: &ActionCtx<'_>,
    code: &str,
    with_activity_id: bool,
) -> Result<String, String> {
    let mut t = match task_by_code_mp(ctx.io, ctx.region, ctx.account, code).await? {
        Some(t) => t,
        None => return Ok("mp 口径未下发该任务（活动可能已结束）".into()),
    };
    if t.claimed {
        return Ok("已领取".into());
    }
    if t.accept_status == "not_accepted" || t.accept_status.is_empty() {
        if !accept_with_verify_mp(ctx, code).await {
            return Ok("accept 未登记生效（上游 200+OK 但未落账形态），待下次重试".into());
        }
    }
    let target = if t.target <= 0 { 1 } else { t.target };
    if t.current >= target || t.accept_status == "completed" {
        let c = claim_task(ctx.io, ctx.region, ctx.account, code).await?;
        return Ok(claim_msg("已领取奖励", &c));
    }
    let need = (target - t.current) as u64;
    let now = ctx.now_ms();
    for i in 0..need {
        let conv = format!("buddyswitch-mp-{now}-{i}");
        let ev = if with_activity_id {
            school_season_chat_event(&conv)
        } else {
            school_chat_times_event(&conv)
        };
        let events = [ev];
        if let Err(e) = ctx.report_mp(&events).await {
            return Ok(format!("完成 {i}/{need} 次上报后中断: {e}"));
        }
        tokio::time::sleep(MP_ACTION_GAP).await;
    }
    // 回读（异步计分，两轮各隔 3s）。
    for _ in 0..2 {
        tokio::time::sleep(Duration::from_secs(3)).await;
        if let Ok(Some(next)) = task_by_code_mp(ctx.io, ctx.region, ctx.account, code).await {
            t = next;
            if t.claimable || t.claimed || t.current >= target {
                break;
            }
        }
    }
    if t.claimed {
        return Ok("本轮已入账（claimed）".into());
    }
    if t.current < target {
        return Ok(format!("已上报 {need} 次但进度未达 {}/{}（异步计分未归账，下次重试）", t.current, target));
    }
    let c = claim_task(ctx.io, ctx.region, ctx.account, code).await?;
    Ok(claim_msg("任务点亮并领取奖励", &c))
}

/// Sequential 预留任务骨架（_4 定时任务 / _5 GLM / _7 灵感；每日零点解锁一环）。
enum SequentialVariant {
    Automation,
    ModelChat,
    Playbook,
}

async fn run_sequential(
    ctx: &ActionCtx<'_>,
    code: &str,
    variant: SequentialVariant,
) -> Result<String, String> {
    let mut t = match task_by_code_mp(ctx.io, ctx.region, ctx.account, code).await? {
        Some(t) => t,
        None => return Ok("mp 口径未下发该任务（前置任务未完成或活动未开始）".into()),
    };
    if t.claimed {
        return Ok("已领取".into());
    }
    let target = if t.target <= 0 { 1 } else { t.target };
    if t.current >= target || t.accept_status == "completed" {
        let c = claim_task(ctx.io, ctx.region, ctx.account, code).await?;
        return Ok(claim_msg("已领取奖励", &c));
    }
    if t.accept_status == "not_accepted" || t.accept_status.is_empty() {
        if !accept_with_verify_mp(ctx, code).await {
            return Ok("accept 未登记生效（任务可能处于每日锁定窗口，等解锁后自动重试）".into());
        }
    }
    if sequential_primary(ctx, &variant).await.is_err() {
        return Ok("判据上报失败".into());
    }
    for round in 0..2 {
        tokio::time::sleep(Duration::from_secs(3)).await;
        if let Ok(Some(next)) = task_by_code_mp(ctx.io, ctx.region, ctx.account, code).await {
            t = next;
            if t.claimable || t.claimed || t.current >= target {
                break;
            }
            if round == 0 {
                if sequential_fallback(ctx, &variant).await.is_err() {
                    return Ok("备选判据上报失败".into());
                }
            }
        }
    }
    if t.claimed {
        return Ok("本轮已入账（claimed）".into());
    }
    if t.current < target {
        return Ok("已上报但进度未点亮（判据形态待解锁后校正，下次重试）".into());
    }
    let c = claim_task(ctx.io, ctx.region, ctx.account, code).await?;
    Ok(claim_msg("任务点亮并领取奖励", &c))
}

/// Sequential 预留任务的「主判据」上报。
async fn sequential_primary(ctx: &ActionCtx<'_>, variant: &SequentialVariant) -> Result<(), String> {
    match variant {
        // PC 同源定时任务创建事件（automation_1 同源）。
        SequentialVariant::Automation => {
            let events = [automation_create_event("buddyswitch 自动化")];
            ctx.report_desktop(&events).await
        }
        // mp 对话事件带模型字段 glm-5.2。
        SequentialVariant::ModelChat => {
            let now = ctx.now_ms();
            let conv = format!("buddyswitch-mp-glm-{now}");
            let ev = [mini_chat_model_event(&conv, "glm-5.2", "GLM-5.2")];
            ctx.report_mp(&ev).await
        }
        // PC 灵感事件组。
        SequentialVariant::Playbook => {
            let now = ctx.now_ms();
            let conv = format!("buddyswitch-pb-{now}");
            let req = format!("buddyswitch-pb-req-{now}");
            let events =
                playbook_prompt_sequence(&conv, &req, "pm-gtm-launch-plan", "新产品上市 GTM 发布计划一页纸");
            ctx.report_desktop(&events).await
        }
    }
}

/// Sequential 预留任务的「备选判据」上报（主判据未点亮时补一轮）。
async fn sequential_fallback(ctx: &ActionCtx<'_>, variant: &SequentialVariant) -> Result<(), String> {
    match variant {
        SequentialVariant::Automation => Ok(()),
        // 备选：PC 域模型活跃上报（Model_chat_GLM5.2 同源）。
        SequentialVariant::ModelChat => {
            let now = ctx.now_ms();
            let cid = format!("buddyswitch-mp-glm-{now}");
            report_chat_event(ctx.io, ctx.region, ctx.account, &cid, &cid, "glm-5.2", "GLM-5.2").await
        }
        // 备选：mp 指纹灵感事件组。
        SequentialVariant::Playbook => {
            let events = mini_playbook_events("pm-gtm-launch-plan", "新产品上市 GTM 发布计划一页纸");
            ctx.report_mp(&events).await
        }
    }
}

/// Sequential_Tasks_2（mp 选专家对话）：市场真实专家 id → mp 指纹 expert_actual_use。
async fn run_mini_expert(ctx: &ActionCtx<'_>) -> Result<String, String> {
    const CODE: &str = "Sequential_Tasks_2";
    let mut t = match task_by_code_mp(ctx.io, ctx.region, ctx.account, CODE).await? {
        Some(t) => t,
        None => return Ok("mp 口径未下发该任务（活动可能已结束）".into()),
    };
    if t.claimed {
        return Ok("已领取".into());
    }
    let target = if t.target <= 0 { 1 } else { t.target };
    if t.current >= target || t.accept_status == "completed" {
        let c = claim_task(ctx.io, ctx.region, ctx.account, CODE).await?;
        return Ok(claim_msg("已领取奖励", &c));
    }
    // 判据载体前置（accept 之前）：市场真实专家 id（空 id 服务端不入账）。
    let experts = match real_chat_market_guard(ctx, "").await {
        Ok(list) if !list.is_empty() => list,
        Ok(_) => return Ok("专家市场不可用（列表为空），跳过以防半程态".into()),
        Err(e) => return Ok(format!("专家市场不可用（{e}），跳过以防半程态")),
    };
    let e = &experts[0];
    if t.accept_status == "not_accepted" || t.accept_status.is_empty() {
        if !accept_with_verify_mp(ctx, CODE).await {
            return Ok("accept 未登记生效（上游 200+OK 但未落账形态），待下次重试".into());
        }
    }
    let ev = [mini_expert_use_event(&e.expert_id, e.display(), &e.expert_type)];
    ctx.report_mp(&ev).await?;
    for _ in 0..2 {
        tokio::time::sleep(Duration::from_secs(3)).await;
        if let Ok(Some(next)) = task_by_code_mp(ctx.io, ctx.region, ctx.account, CODE).await {
            t = next;
            if t.claimable || t.claimed || t.current >= target {
                break;
            }
        }
    }
    if t.claimed {
        return Ok("本轮已入账（claimed）".into());
    }
    if t.current < target {
        return Ok("已上报但进度未归账（异步计分，下次重试）".into());
    }
    let c = claim_task(ctx.io, ctx.region, ctx.account, CODE).await?;
    Ok(claim_msg("任务点亮并领取奖励", &c))
}

/// 领奖消息拼接（0 奖励 = 此前已领过）。
fn claim_msg(prefix: &str, c: &Value) -> String {
    let credit = c["credit"].as_i64().unwrap_or(0);
    let energy = c["energy"].as_i64().unwrap_or(0);
    if credit > 0 || energy > 0 {
        format!("{prefix}（+{credit}c +{energy}e）")
    } else {
        format!("{prefix}（奖励此前已领取）")
    }
}

// ---------------------------------------------------------------------------
// 一键完成全部（panel runAutoAll 语义）
// ---------------------------------------------------------------------------

/// 一键完成该账号全部可自动任务（批量 accept → 逐项执行，项间节流；单项失败不影响后续）。
/// 返回 `{results: [...]}`（每项 `{taskCode, desc, status, message, ...}`，camelCase 对齐前端）。
pub async fn run_auto_all(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
) -> Result<Value, String> {
    if region == Region::Global {
        return Err("成长任务体系仅 CN（该区域无成长任务）".into());
    }
    let uid = acc_uid(account);
    if try_lock_account(&uid).is_none() {
        return Err("该账号有任务动作正在执行中，请等本轮结束后再试".into());
    }
    let result = run_auto_all_inner(io, region, account).await;
    unlock_account(&uid);
    result
}

/// 内部执行（队列路径已持锁时调用）。
pub async fn run_auto_all_inner(
    io: &dyn GrowthIo,
    region: Region,
    account: &Value,
) -> Result<Value, String> {
    let ctx = ActionCtx::new(io, region, account);
    let mut results: Vec<Value> = Vec::new();

    // 阶段 0：批量接受默认 + mp 口径未接受任务（失败不阻塞——行为事件才是进度判据）。
    let pending_codes = |tasks: &[GrowthTask]| -> Vec<String> {
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
    if let Ok(tasks) = list_tasks(io, region, account, false).await {
        let codes = pending_codes(&tasks);
        if !codes.is_empty() {
            match accept_tasks(io, region, account, &codes).await {
                Ok(()) => results.push(json!({
                    "task_code": "(批量接受)",
                    "status": "done",
                    "message": format!("已接受 {} 个任务", codes.len())
                })),
                Err(e) => results.push(json!({
                    "task_code": "(批量接受)",
                    "status": "error",
                    "message": format!("接受任务失败（不阻塞后续）: {e}")
                })),
            }
        }
    }
    if let Ok(mp_tasks) = list_tasks(io, region, account, true).await {
        let codes = pending_codes(&mp_tasks);
        if !codes.is_empty() {
            match accept_tasks_mp(io, region, account, &codes).await {
                Ok(()) => results.push(json!({
                    "task_code": "(批量接受-mp)",
                    "status": "done",
                    "message": format!("已接受 {} 个小程序任务", codes.len())
                })),
                Err(e) => results.push(json!({
                    "task_code": "(批量接受-mp)",
                    "status": "error",
                    "message": format!("接受小程序任务失败（不阻塞后续）: {e}")
                })),
            }
        }
    }

    // 逐项执行（按依赖序；skipped/error 不睡间隔，执行过才节流）。
    for code in ALL_TASK_CODES {
        let item_desc = auto_action_desc(code).unwrap_or("");
        let mut item = json!({"task_code": code, "desc": item_desc});

        let before = match task_by_code(io, region, account, code).await {
            Ok(Some(t)) => t,
            Ok(None) => {
                item["status"] = json!("skipped");
                item["message"] = json!("该账号无此任务");
                results.push(item);
                continue;
            }
            Err(e) => {
                item["status"] = json!("error");
                item["message"] = json!(format!("查询失败: {e}"));
                results.push(item);
                continue;
            }
        };
        if before.claimed || (before.target > 0 && before.current >= before.target) {
            item["status"] = json!("skipped");
            item["message"] = json!(format!("已完成（{}）", task_progress_text(Some(&before))));
            results.push(item);
            continue;
        }

        let exec = dispatch_run(&ctx, code).await;
        match exec {
            Ok(msg) => {
                let is_mp = is_mp_task_code(code);
                let after: Option<GrowthTask> = if is_mp {
                    task_by_code_mp(io, region, account, code).await.unwrap_or(None)
                } else {
                    task_by_code_waiting(io, region, account, code).await.unwrap_or(None)
                };
                item["status"] = json!("done");
                item["message"] = json!(msg);
                item["progress_after"] = json!(task_progress_text(after.as_ref()));
                if let Some(t) = &after {
                    if t.claimable {
                        item["claimable"] = json!(true);
                        match claim_task(io, region, account, code).await {
                            Ok(c) => {
                                let credit = c["credit"].as_i64().unwrap_or(0);
                                let energy = c["energy"].as_i64().unwrap_or(0);
                                item["claimed"] = json!(true);
                                item["credit"] = json!(credit);
                                item["energy"] = json!(energy);
                                if credit > 0 || energy > 0 {
                                    item["message"] =
                                        json!(format!("{msg}；已自动领奖 +{credit} 分 +{energy} 能"));
                                } else {
                                    item["message"] = json!(format!("{msg}；奖励此前已领取"));
                                }
                            }
                            Err(e) => {
                                item["claim_error"] = json!(e);
                                item["message"] = json!(format!("{msg}；达标但领奖失败（可在列表手动重试）"));
                            }
                        }
                    }
                }
                results.push(item);
                tokio::time::sleep(ACTION_GAP).await; // 项间节流
            }
            Err(e) => {
                item["status"] = json!("error");
                item["message"] = json!(e);
                results.push(item);
            }
        }
    }

    // 出口归一为 camelCase（前端 GrowthAutoAllItem.taskCode；Rust 侧键名不贯穿全程）。
    for item in &mut results {
        if let Some(obj) = item.as_object_mut() {
            if let Some(code) = obj.remove("task_code") {
                obj.insert("taskCode".into(), code);
            }
        }
    }
    Ok(json!({"ok": true, "results": results}))
}

// ---------------------------------------------------------------------------
// 单测（Io 注入；无真实网络）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::growth::tasks::GrowthResponse;
    use std::future::Future;
    use std::pin::Pin;

    /// 脚本化响应桩：按调用顺序返回预设响应。
    struct StubIo {
        responses: Vec<GrowthResponse>,
        call_count: std::sync::atomic::AtomicUsize,
    }

    impl StubIo {
        fn new(responses: Vec<GrowthResponse>) -> Self {
            Self {
                responses,
                call_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }
        fn calls(&self) -> usize {
            self.call_count.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    impl GrowthIo for StubIo {
        fn json<'a>(
            &'a self,
            _region: Region,
            _url: &'a str,
            _method: &'a str,
            _body: Option<Value>,
            _account: &'a Value,
            _extra: &'a HashMap<String, String>,
        ) -> Pin<Box<dyn Future<Output = GrowthResponse> + Send + 'a>> {
            Box::pin(async move {
                let i = self.call_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                self.responses
                    .get(i)
                    .cloned()
                    .unwrap_or((200, json!({"code": 0, "data": {}})))
            })
        }
        fn sse_chat<'a>(
            &'a self,
            _region: Region,
            _account: &'a Value,
            _expert_id: &'a str,
            _body: &'a Value,
        ) -> Pin<Box<dyn Future<Output = Result<(String, String), String>> + Send + 'a>> {
            Box::pin(async { Ok(("conv-stub".into(), "a".repeat(32))) })
        }
    }

    fn cn_account_uid(uid: &str) -> Value {
        json!({"uid": uid, "nickname": "测试", "access_token": "at"})
    }

    fn claimed_tasks_resp() -> GrowthResponse {
        (
            200,
            json!({"code": 0, "data": {"tasks": [
                {"task_code": "chat_5", "current": 5, "target": 5, "accept_status": "claimed"}
            ]}}),
        )
    }

    /// 动作顺序 = 依赖序（chat_5 最先，black_cat 靠后）。
    #[test]
    fn action_index_ordering() {
        assert_eq!(auto_action_index("chat_5"), 0);
        assert_eq!(auto_action_index("first_buddy"), 1);
        assert!(auto_action_index("black_cat") > auto_action_index("Expert_lighthouse"));
        assert!(auto_action_index("Sequential_Tasks_7") > auto_action_index("school_season"));
        assert_eq!(auto_action_index("unknown_code"), UNKNOWN_ACTION_INDEX);
    }

    /// 可自动化判定（扫描/队列判据）。
    #[test]
    fn has_auto_action_covers_all_codes() {
        assert!(ALL_TASK_CODES.iter().all(|c| has_auto_action(c)));
        assert!(!has_auto_action("nope"));
    }

    /// 账号锁互斥（同 uid 第二次拿不到）。
    #[test]
    fn account_lock_is_mutually_exclusive() {
        assert!(try_lock_account("u-x").is_some());
        assert!(try_lock_account("u-x").is_none());
        unlock_account("u-x");
        assert!(try_lock_account("u-x").is_some());
        unlock_account("u-x");
        // 不同 uid 互不影响。
        assert!(try_lock_account("u-y").is_some());
        assert!(try_lock_account("u-x").is_some());
        unlock_account("u-y");
        unlock_account("u-x");
    }

    /// run_auto_action：已领取任务 → skipped 短路（不打任何动作请求）。
    #[tokio::test]
    async fn auto_action_short_circuits_claimed() {
        let io = StubIo::new(vec![claimed_tasks_resp()]);
        let out =
            run_auto_action(&io, Region::Cn, &cn_account_uid("u-claimed"), "chat_5")
                .await
                .unwrap();
        assert_eq!(out["skipped"], true, "{out}");
        assert_eq!(out["message"], "该任务已领取过奖励");
        assert_eq!(io.calls(), 1, "claimed 短路不该再打其它请求");
    }

    /// run_auto_action：无对应动作 → 可读错误。
    #[tokio::test]
    async fn auto_action_unknown_code_errors() {
        let io = StubIo::new(vec![]);
        let err = run_auto_action(&io, Region::Cn, &cn_account_uid("u-unknown"), "nope")
            .await
            .unwrap_err();
        assert!(err.contains("客户端内交互"), "{err}");
    }

    /// Global 区域 → 不支持（不发起任何上游调用）。
    #[tokio::test]
    async fn global_region_unsupported() {
        let io = StubIo::new(vec![]);
        let err = run_auto_action(&io, Region::Global, &cn_account_uid("u-global"), "chat_5")
            .await
            .unwrap_err();
        assert!(err.contains("仅 CN"), "{err}");
        assert_eq!(io.calls(), 0);
    }

    /// run_auto_all：全部任务已领取 → 逐项 skipped，无执行请求（只打任务列表）。
    #[tokio::test]
    async fn auto_all_skips_completed_tasks() {
        let io = StubIo::new(vec![
            claimed_tasks_resp(),
            (200, json!({"code": 0, "data": {"tasks": []}})), // mp 列表
            claimed_tasks_resp(),
            claimed_tasks_resp(),
        ]);
        let out = run_auto_all(&io, Region::Cn, &cn_account_uid("u-autoall"))
            .await
            .unwrap();
        let results = out["results"].as_array().unwrap();
        let statuses: Vec<String> = results
            .iter()
            .map(|r| r["status"].as_str().unwrap_or("").to_string())
            .collect();
        assert!(statuses.iter().all(|s| s == "skipped" || s == "done"), "{results:?}");
    }
}
