//! 活跃地图任务：对话事件连发上报 + 连登自检 + 连登奖励链（礼包/补偿/补签/兑换/抽奖）。
//!
//! 对照参考实现 `internal/scheduler/scheduler.go` 的 `runActivity` / `claimGrowthRewards`
//! 与 `internal/upstream/growth_reward.go` + `growth_bonus.go`。
//!
//! 关键约定：
//! - 活跃上报对 **CN 与 Global 都执行**（`/v2/report` 两版均可用），不做 region 跳过；
//!   连登奖励链只服务 **CN**（Global 的 `/activity/growth/streak` 实测 500，链上第一步即失败）。
//! - 「正常态」必须**静默**：redeem 409 duplicate/已领取、403 连续登录天数不足、
//!   lottery 400 无次数/未开启——不是失败，不刷 ERROR。
//! - 礼包 / 补偿 / 兑换 / 抽奖按 **CST 自然日幂等**（同日不重复领，跨日重置）。

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::modules::account::{account_display_name, load_accounts_for};
use crate::modules::config::{authed_json_request_for, load_checkin_config, now_ms};
use crate::modules::cst::{cst_date_str, cst_yesterday_str};
use crate::modules::refresh::ensure_fresh_token_for;
use crate::modules::region::{region_spec, Region};
use crate::modules::schedule::load_schedule_config;

/// 账号之间的间隔（限速，避免批量请求触发风控）。
pub const ACTIVITY_ACCOUNT_DELAY: Duration = Duration::from_millis(800);
/// 同一账号内连发上报之间的间隔。
pub const ACTIVITY_REPORT_GAP: Duration = Duration::from_millis(1500);

const REPORT_PATH: &str = "/v2/report";
const STREAK_PATH: &str = "/activity/growth/streak";
const HEATMAP_PATH: &str = "/activity/growth/heatmap";
const MAKEUP_PATH: &str = "/activity/growth/makeup-cards/use";
const REDEEM_PATH: &str = "/activity/growth/redeem";
const LOTTERY_CHANCES_PATH: &str = "/activity/growth/lottery/chances";
const LOTTERY_DRAW_PATH: &str = "/activity/growth/lottery/draw";
const CLAIM_GIFT_PATH: &str = "/billing/meter/claim-gift";
const CLAIM_COMPENSATION_PATH: &str = "/billing/meter/claim-compensation";

// ---------------------------------------------------------------------------
// 按 CST 自然日的幂等闸
// ---------------------------------------------------------------------------

/// 纯判定：上一次处理日期不是今天 → 允许执行（同日跳过，跨日恢复）。
pub fn reward_gate_allows(last_date: Option<&str>, today: &str) -> bool {
    last_date != Some(today)
}

/// 领取类写操作的「当日已处理」闸（uid → CST 自然日）。
///
/// **持久化**：`mark()` 会把「日期 == 今天」的条目写回 `~/.buddy-switch/reward_gate.json`，
/// 因此**重启不会重跑当日的领取**。早期实现只在进程内标记，重启即清零，会对上游重复
/// 发起 claim-gift / claim-compensation / redeem / draw —— 上游虽有幂等兜底（409
/// duplicate 等正常态），但那是**兜底**，不该被我们当成常态依赖：它既产生多余的
/// 往返与日志噪声，也让「今日已处理」这个语义在重启后失真。
///
/// 落盘是**尽力而为**：写失败只打印一次告警，绝不阻断任务（幂等的最终防线仍在上游）。
/// 文件自限界：只保留「今天」的条目，因此不会随运行天数无限增长。
#[derive(Default)]
pub struct DailyGate {
    inner: Mutex<GateState>,
}

#[derive(Default)]
struct GateState {
    entries: HashMap<String, String>,
    /// `None` = 纯内存（单测用），不落盘。
    file: Option<std::path::PathBuf>,
}

impl DailyGate {
    /// 纯内存闸（不落盘）——供单测与不需要持久化的场景使用。
    pub fn in_memory() -> Self {
        Self::default()
    }

    /// 从指定文件载入；文件缺失 / 损坏一律当作空表（不 panic）。
    pub fn load_from(file: std::path::PathBuf) -> Self {
        let entries = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|value| value.get("entries").cloned())
            .and_then(|entries| entries.as_object().cloned())
            .map(|map| {
                map.into_iter()
                    .filter_map(|(uid, date)| date.as_str().map(|date| (uid, date.to_string())))
                    .collect::<HashMap<String, String>>()
            })
            .unwrap_or_default();
        Self {
            inner: Mutex::new(GateState {
                entries,
                file: Some(file),
            }),
        }
    }

    /// 载入生产路径（`~/.buddy-switch/reward_gate.json`，受 `BUDDY_SWITCH_HOME` 影响）。
    pub fn load() -> Self {
        Self::load_from(crate::modules::config::reward_gate_file())
    }

    /// 该账号今天是否还没处理过（true = 允许执行）。
    pub fn should_run(&self, uid: &str, today: &str) -> bool {
        let state = self.inner.lock().unwrap();
        reward_gate_allows(state.entries.get(uid).map(String::as_str), today)
    }

    /// 记录该账号今天已处理，并（尽力）落盘。
    pub fn mark(&self, uid: &str, today: &str) {
        let mut state = self.inner.lock().unwrap();
        state.entries.insert(uid.to_string(), today.to_string());
        // 自限界：判定只做「是否等于今天」的相等比较，因此更早的日期整体失效，
        // 保留它们只会让文件随运行天数增长。
        state.entries.retain(|_, date| date == today);
        persist_gate(&state);
    }

    /// 当前快照（诊断 / 测试用）。
    pub fn snapshot(&self) -> HashMap<String, String> {
        self.inner.lock().unwrap().entries.clone()
    }
}

/// 落盘当日闸；写失败只告警一次，不影响任务继续。
fn persist_gate(state: &GateState) {
    let Some(file) = state.file.as_ref() else {
        return;
    };
    let payload = serde_json::json!({ "version": 1, "entries": state.entries });
    let Ok(text) = serde_json::to_string_pretty(&payload) else {
        return;
    };
    if let Err(error) = crate::modules::config::atomic_write(file, &text) {
        static WARNED: OnceLock<()> = OnceLock::new();
        if WARNED.set(()).is_ok() {
            eprintln!("[activity] 当日领取闸落盘失败（不影响本次执行）：{error}");
        }
    }
}

static REWARD_GATE: OnceLock<DailyGate> = OnceLock::new();

fn reward_gate() -> &'static DailyGate {
    REWARD_GATE.get_or_init(DailyGate::load)
}

// ---------------------------------------------------------------------------
// 请求封装
// ---------------------------------------------------------------------------

/// growth / chat 域请求头附加项（与出行中心同域，带 Origin/Referer/web 标识）。
fn growth_extra(region: Region) -> HashMap<String, String> {
    let base = region_spec(region).billing_base;
    let mut headers = HashMap::new();
    headers.insert("x-client-platform".to_string(), "web".to_string());
    headers.insert("origin".to_string(), base.to_string());
    headers.insert(
        "referer".to_string(),
        format!("{base}/profile/growth-center"),
    );
    headers
}

async fn billing_post(region: Region, path: &str, body: Value, account: &Value) -> (u16, Value) {
    let url = format!("{}{path}", region_spec(region).billing_base);
    authed_json_request_for(region, &url, "POST", Some(body), account, &HashMap::new()).await
}

async fn chat_get(region: Region, path: &str, account: &Value) -> (u16, Value) {
    let url = format!("{}{path}", region_spec(region).chat_base);
    authed_json_request_for(region, &url, "GET", None, account, &growth_extra(region)).await
}

async fn chat_post(region: Region, path: &str, body: Value, account: &Value) -> (u16, Value) {
    let url = format!("{}{path}", region_spec(region).chat_base);
    authed_json_request_for(
        region,
        &url,
        "POST",
        Some(body),
        account,
        &growth_extra(region),
    )
    .await
}

/// 构造一条 `chat_request_send`（craft）上报事件。
///
/// `userId` 必填——参考实现实测缺 `userId` 时上游返回 200 但**静默丢弃**（连登不点亮）。
pub fn build_report_event(cid: &str, request_id: &str, uid: &str, at_ms: i64) -> Value {
    json!({
        "eventCode": "chat_request_send",
        "timestamp": at_ms,
        "reportDelay": 0,
        "mode": "craft",
        "conversationId": cid,
        "requestId": request_id,
        "inputLength": 12,
        "requestModelId": "deepseek-v4-flash",
        "requestModelName": "DeepSeek V4 Flash",
        "presentAt": at_ms,
        "rootRequestId": cid,
        "parentConversationId": cid,
        "agentName": "default",
        "agentType": "conversation",
        "userId": uid,
    })
}

// ---------------------------------------------------------------------------
// 响应解析（读 streak / heatmap）
// ---------------------------------------------------------------------------

/// 连登天数（`data.streak.days`）。
fn streak_days(resp: &Value) -> Option<i64> {
    resp.pointer("/data/streak/days").and_then(Value::as_i64)
}

/// 连登奖励兑换状态（`data.redemption_status`）。
fn redemption_status(resp: &Value) -> Value {
    resp.pointer("/data/redemption_status")
        .cloned()
        .unwrap_or(Value::Null)
}

/// 补签卡余额（`data.makeup_cards.balance`）。
fn makeup_balance(resp: &Value) -> i64 {
    resp.pointer("/data/makeup_cards/balance")
        .and_then(Value::as_i64)
        .unwrap_or(0)
}

/// 在热力格列表里取某日（`YYYY-MM-DD`）的 score；无该日格返回 `None`（无判据）。
pub fn heatmap_day_score(cells: &[Value], date: &str) -> Option<i64> {
    for cell in cells {
        if let Some(cell_date) = cell.get("date").and_then(Value::as_str) {
            if cell_date.len() >= 10 && &cell_date[..10] == date {
                return cell.get("score").and_then(Value::as_i64);
            }
        }
    }
    None
}

/// 该档位本月是否已领取（`tier_*_status == "claimed"`）。
fn tier_claimed(redemption: &Value, tier: &str) -> bool {
    let key = match tier {
        "7d" => "tier_7d_status",
        "14d" => "tier_14d_status",
        "28d" => "tier_28d_status",
        _ => return false,
    };
    redemption.get(key).and_then(Value::as_str) == Some("claimed")
}

/// 从高到低挑「连登天数已达标且**未领取**」的最高档位；无可领返回 `None`（正常态）。
///
/// 参考实现按 `tiers` 数组的 days 升序倒序挑；这里改为按 `days` 数值取最大，避免对
/// 上游数组顺序的隐含假设。
pub fn growth_eligible_tier(days: i64, redemption: &Value) -> Option<String> {
    let tiers = redemption.get("tiers")?.as_array()?;
    let mut best: Option<(i64, String)> = None;
    for tier in tiers {
        let Some(name) = tier.get("tier").and_then(Value::as_str) else {
            continue;
        };
        let tier_days = tier.get("days").and_then(Value::as_i64).unwrap_or(0);
        if days < tier_days || tier_claimed(redemption, name) {
            continue;
        }
        if best.as_ref().map(|(d, _)| tier_days > *d).unwrap_or(true) {
            best = Some((tier_days, name.to_string()));
        }
    }
    best.map(|(_, tier)| tier)
}

// ---------------------------------------------------------------------------
// 「正常态」识别（静默，不刷 ERROR）
// ---------------------------------------------------------------------------

/// 取响应文案（`message` / `msg`）并转小写。
fn message_text(resp: &Value) -> String {
    resp.get("message")
        .or_else(|| resp.get("msg"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase()
}

fn message_contains(resp: &Value, needle: &str) -> bool {
    message_text(resp).contains(&needle.to_lowercase())
}

/// redeem 409 且 body 含 `duplicate` / `已领取` = 本月已领，正常态。
pub fn is_redeem_already_claimed(status: u16, resp: &Value) -> bool {
    status == 409 && (message_contains(resp, "duplicate") || message_contains(resp, "已领取"))
}

/// redeem 403 且 body 含 `连续登录天数不足` = 未达标，正常态。
pub fn is_redeem_not_enough_days(status: u16, resp: &Value) -> bool {
    status == 403 && message_contains(resp, "连续登录天数不足")
}

/// lottery 400 且 body 含 `insufficient lottery chance balance` = 无次数，正常态。
pub fn is_lottery_no_chance(status: u16, resp: &Value) -> bool {
    status == 400 && message_contains(resp, "insufficient lottery chance balance")
}

/// lottery 400 且 body 含 `lottery disabled` = 抽奖未开启，正常态。
pub fn is_lottery_disabled(status: u16, resp: &Value) -> bool {
    status == 400 && message_contains(resp, "lottery disabled")
}

// ---------------------------------------------------------------------------
// 连登奖励链
// ---------------------------------------------------------------------------

/// 昨日漏签且有补签卡时补签（保住连登连续天数）。成功返回 true。
async fn makeup_yesterday(region: Region, account: &Value, yesterday: &str) -> bool {
    let (heatmap_status, heatmap) = chat_get(region, HEATMAP_PATH, account).await;
    if heatmap_status != 200 {
        return false;
    }
    let cells = heatmap
        .pointer("/data/cells")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    // 只对「昨日有漏签（score==0）」补签；无该日格或无漏签则不动。
    if heatmap_day_score(&cells, yesterday) != Some(0) {
        return false;
    }
    let (streak_status, streak) = chat_get(region, STREAK_PATH, account).await;
    if streak_status != 200 || makeup_balance(&streak) <= 0 {
        return false;
    }
    let (use_status, _) = chat_post(
        region,
        MAKEUP_PATH,
        json!({"target_date": yesterday}),
        account,
    )
    .await;
    use_status == 200
}

/// 领取连登奖励 + 抽奖（CN 专有）。失败不阻断上报主流程（finally 语义）。
async fn claim_growth_rewards(region: Region, account: &Value) {
    if region != Region::Cn {
        return;
    }
    let uid = account
        .get("uid")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if uid.is_empty() {
        return;
    }
    let today = cst_date_str(now_ms());
    if !reward_gate().should_run(&uid, &today) {
        return;
    }

    // 礼包 / 补偿：有则领的幂等写，业务错误静默（大多数号早已领过）。
    let _ = billing_post(region, CLAIM_GIFT_PATH, json!({}), account).await;
    let _ = billing_post(region, CLAIM_COMPENSATION_PATH, json!({}), account).await;

    let (streak_status, streak) = chat_get(region, STREAK_PATH, account).await;
    if streak_status != 200 {
        // 读状态失败：不标记当日，下一轮再试（只读，无写风险）。
        return;
    }
    let mut days = streak_days(&streak).unwrap_or(0);
    let mut redemption = redemption_status(&streak);

    // 补签保连登：补签成功则重读 state，让本日 redeem 吃到恢复后的天数。
    let yesterday = cst_yesterday_str(now_ms());
    if makeup_yesterday(region, account, &yesterday).await {
        if let (200, reread) = chat_get(region, STREAK_PATH, account).await {
            days = streak_days(&reread).unwrap_or(days);
            redemption = redemption_status(&reread);
        }
    }

    if let Some(tier) = growth_eligible_tier(days, &redemption) {
        let token = format!("redeem-{tier}-{}", random_hex32());
        let (status, resp) = chat_post(
            region,
            REDEEM_PATH,
            json!({"tier": tier, "client_token": token}),
            account,
        )
        .await;
        debug_assert!(
            status == 200
                || is_redeem_already_claimed(status, &resp)
                || is_redeem_not_enough_days(status, &resp),
            "redeem 非正常态: status={status} resp={resp}"
        );
    }

    // 无论 redeem 是否成功都标记当日已处理：领取类各状态当日不再重试（次日自然重置）。
    reward_gate().mark(&uid, &today);

    claim_growth_lottery(region, account).await;
}

/// 消耗连登奖励赠与的抽奖次数（只抽 balance>0；无次数/未开启静默）。
async fn claim_growth_lottery(region: Region, account: &Value) {
    let (status, resp) = chat_get(region, LOTTERY_CHANCES_PATH, account).await;
    if status != 200 {
        return;
    }
    let balance = resp
        .pointer("/data/balance")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if balance <= 0 {
        return;
    }
    let token = format!("draw-{}", random_hex32());
    let (draw_status, draw_resp) = chat_post(
        region,
        LOTTERY_DRAW_PATH,
        json!({"client_token": token}),
        account,
    )
    .await;
    debug_assert!(
        draw_status == 200
            || is_lottery_no_chance(draw_status, &draw_resp)
            || is_lottery_disabled(draw_status, &draw_resp),
        "lottery draw 非正常态: status={draw_status} resp={draw_resp}"
    );
}

/// 32 位小写 hex 随机串（uuid v4 去横线），用作 client_token 的一次性幂等键。
fn random_hex32() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

// ---------------------------------------------------------------------------
// 主循环
// ---------------------------------------------------------------------------

/// 对全部账号执行一轮活跃上报（CN）。
pub async fn run_activity_cycle() -> Value {
    run_activity_cycle_for(Region::Cn).await
}

/// 按 region 执行一轮活跃上报：逐账号连发 `activity_report_count` 条 → 回读连登 → 领奖。
///
/// CN 与 Global **都执行**；单账号失败只记账不影响其它账号。
pub async fn run_activity_cycle_for(region: Region) -> Value {
    let cfg = load_schedule_config();
    let count = cfg.activity_report_count.max(1);
    let checkin_cfg = load_checkin_config();
    let accounts = load_accounts_for(region);
    if accounts.is_empty() {
        return json!({"status": "no_accounts", "region": region.as_str()});
    }

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
            tokio::time::sleep(ACTIVITY_ACCOUNT_DELAY).await;
        }
        first = false;

        let acc = ensure_fresh_token_for(region, account.clone(), &checkin_cfg).await;
        let uid = acc
            .get("uid")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        // N 条共用同一 conversationId（同会话多轮），requestId 各条独立。
        let cid = format!("wb2api-{}", now_ms());
        let mut reported = 0u32;
        for i in 1..=count {
            let rid = format!("{cid}-r{i}");
            let event = build_report_event(&cid, &rid, &uid, now_ms());
            let (status, _resp) = billing_post(region, REPORT_PATH, json!([event]), &acc).await;
            if status != 200 {
                break;
            }
            reported += 1;
            if i < count {
                tokio::time::sleep(ACTIVITY_REPORT_GAP).await;
            }
        }

        let mut streak_days_value: Option<i64> = None;
        if reported >= count {
            // 回读连登自检：report 200 ≠ 计分，需回读验证闭环（发现静默丢弃）。
            let (status, resp) = chat_get(region, STREAK_PATH, &acc).await;
            if status == 200 {
                streak_days_value = streak_days(&resp);
            }
            claim_growth_rewards(region, &acc).await;
        }

        items.push(json!({
            "email": account_display_name(&acc),
            "reported": reported,
            "streakDays": streak_days_value,
        }));
    }

    json!({"status": "ok", "region": region.as_str(), "accounts": items})
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 临时闸文件路径（不落真实 home；用 uuid 避免并发用例互相踩）。
    fn temp_gate_file() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("wb-gate-{}.json", uuid::Uuid::new_v4()))
    }

    #[test]
    fn gate_persists_across_reload_so_restart_does_not_reclaim() {
        let file = temp_gate_file();
        let gate = DailyGate::load_from(file.clone());
        assert!(gate.should_run("u1", "2026-09-16"), "首次应允许执行");
        gate.mark("u1", "2026-09-16");

        // 模拟进程重启：从同一文件重新载入
        let reloaded = DailyGate::load_from(file.clone());
        assert!(
            !reloaded.should_run("u1", "2026-09-16"),
            "重启后同日必须仍被拦住（否则会对上游重复领取）"
        );
        assert!(reloaded.should_run("u1", "2026-09-17"), "跨日必须恢复执行");
        assert!(
            reloaded.should_run("u2", "2026-09-16"),
            "未处理过的账号不受影响"
        );
        assert_eq!(
            reloaded.snapshot().get("u1").map(String::as_str),
            Some("2026-09-16")
        );

        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn gate_prunes_stale_dates_so_the_file_stays_bounded() {
        let file = temp_gate_file();
        let gate = DailyGate::load_from(file.clone());
        gate.mark("yesterday-uid", "2026-09-15");
        gate.mark("today-uid", "2026-09-16");

        let snapshot = gate.snapshot();
        assert_eq!(snapshot.len(), 1, "更早的日期应被剪掉: {snapshot:?}");
        assert!(snapshot.contains_key("today-uid"));

        let reloaded = DailyGate::load_from(file.clone());
        assert_eq!(reloaded.snapshot().len(), 1, "落盘的也只剩今天");
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn gate_loads_missing_or_malformed_file_as_empty() {
        let missing = temp_gate_file();
        assert!(
            DailyGate::load_from(missing).snapshot().is_empty(),
            "缺失文件不得 panic"
        );

        let malformed = temp_gate_file();
        std::fs::write(&malformed, b"{ not json").expect("写入临时文件");
        assert!(
            DailyGate::load_from(malformed.clone())
                .snapshot()
                .is_empty(),
            "损坏文件应视为空表"
        );
        let _ = std::fs::remove_file(&malformed);

        let wrong_shape = temp_gate_file();
        std::fs::write(&wrong_shape, br#"{"entries": "oops"}"#).expect("写入临时文件");
        assert!(
            DailyGate::load_from(wrong_shape.clone())
                .snapshot()
                .is_empty(),
            "entries 非对象应视为空表"
        );
        let _ = std::fs::remove_file(&wrong_shape);
    }

    #[test]
    fn in_memory_gate_never_writes_to_disk() {
        let gate = DailyGate::in_memory();
        gate.mark("u1", "2026-09-16");
        assert!(!gate.should_run("u1", "2026-09-16"), "内存闸语义不变");
        assert_eq!(gate.snapshot().len(), 1);
    }

    #[test]
    fn daily_gate_skips_same_day_and_recovers_next_day() {
        let gate = DailyGate::default();
        assert!(gate.should_run("u1", "2026-09-16"), "首次应允许");
        gate.mark("u1", "2026-09-16");
        assert!(!gate.should_run("u1", "2026-09-16"), "同日第二次必须被跳过");
        assert!(gate.should_run("u1", "2026-09-17"), "跨日必须恢复");
        // 其它账号互不影响。
        assert!(gate.should_run("u2", "2026-09-16"));
        assert!(reward_gate_allows(None, "2026-09-16"));
        assert!(!reward_gate_allows(Some("2026-09-16"), "2026-09-16"));
    }

    #[test]
    fn growth_tier_picks_highest_reached_unclaimed() {
        let redemption = json!({
            "tiers": [
                {"tier": "7d", "days": 7},
                {"tier": "14d", "days": 14},
                {"tier": "28d", "days": 28},
            ],
            "tier_7d_status": "available",
            "tier_14d_status": "available",
            "tier_28d_status": "available",
        });
        assert_eq!(growth_eligible_tier(7, &redemption).as_deref(), Some("7d"));
        assert_eq!(
            growth_eligible_tier(20, &redemption).as_deref(),
            Some("14d")
        );
        assert_eq!(
            growth_eligible_tier(30, &redemption).as_deref(),
            Some("28d")
        );
        // 未达最低档 → 无可领。
        assert_eq!(growth_eligible_tier(3, &redemption), None);

        // 14d 已领 → 退到 7d。
        let claimed_14 = json!({
            "tiers": [{"tier": "7d", "days": 7}, {"tier": "14d", "days": 14}],
            "tier_7d_status": "available",
            "tier_14d_status": "claimed",
        });
        assert_eq!(growth_eligible_tier(20, &claimed_14).as_deref(), Some("7d"));
    }

    #[test]
    fn growth_tier_none_when_all_claimed_or_no_status() {
        let all_claimed = json!({
            "tiers": [{"tier": "7d", "days": 7}],
            "tier_7d_status": "claimed",
        });
        assert_eq!(growth_eligible_tier(30, &all_claimed), None);
        assert_eq!(growth_eligible_tier(30, &Value::Null), None);
    }

    #[test]
    fn heatmap_day_score_matches_by_date_prefix() {
        let cells = vec![
            json!({"date": "2026-09-15", "score": 0}),
            json!({"date": "2026-09-16T00:00:00", "score": 5}),
        ];
        assert_eq!(heatmap_day_score(&cells, "2026-09-15"), Some(0));
        assert_eq!(heatmap_day_score(&cells, "2026-09-16"), Some(5));
        assert_eq!(heatmap_day_score(&cells, "2026-09-17"), None);
    }

    #[test]
    fn normal_states_are_recognized_and_silent() {
        // redeem 409 duplicate / 已领取
        assert!(is_redeem_already_claimed(
            409,
            &json!({"message": "duplicate redeem"})
        ));
        assert!(is_redeem_already_claimed(
            409,
            &json!({"msg": "该奖励已领取"})
        ));
        assert!(!is_redeem_already_claimed(
            409,
            &json!({"message": "other"})
        ));
        // redeem 403 天数不足
        assert!(is_redeem_not_enough_days(
            403,
            &json!({"message": "连续登录天数不足"})
        ));
        assert!(!is_redeem_not_enough_days(
            403,
            &json!({"message": "forbidden"})
        ));
        // lottery 400 无次数 / 未开启
        assert!(is_lottery_no_chance(
            400,
            &json!({"message": "insufficient lottery chance balance"})
        ));
        assert!(is_lottery_disabled(
            400,
            &json!({"message": "lottery disabled"})
        ));
        assert!(!is_lottery_no_chance(400, &json!({"message": "其它"})));
    }

    #[test]
    fn report_event_carries_required_identity_fields() {
        let ev = build_report_event("wb2api-1", "wb2api-1-r1", "uid-9", 1_700_000_000_000);
        assert_eq!(ev["eventCode"], "chat_request_send");
        assert_eq!(ev["mode"], "craft");
        assert_eq!(ev["userId"], "uid-9");
        assert_eq!(ev["conversationId"], "wb2api-1");
        assert_eq!(ev["requestId"], "wb2api-1-r1");
        assert_eq!(ev["requestModelId"], "deepseek-v4-flash");
        assert_eq!(ev["requestModelName"], "DeepSeek V4 Flash");
        assert_eq!(ev["rootRequestId"], "wb2api-1");
        assert_eq!(ev["parentConversationId"], "wb2api-1");
        assert_eq!(ev["inputLength"], 12);
    }
}
