//! 全账号任务中心：扫描待办（scan_all）→ 并发执行队列（run_queue）→ 状态轮询（queue_status）。
//!
//! 语义对照 panel `taskcenter.go`：
//! - 扫描（只读）：并发拉取每 CN 账号的成长任务（默认 + 小程序口径合并去重），
//!   汇总「未完成且可自动化」的待办清单
//! - 队列：待办按账号分组、组内按动作依赖序；账号内**串行**（per-account 锁，
//!   与单任务/一键完成互斥），账号间**并发**（信号量夹取 [1,4]）
//! - `run_growth_queue_once`：调度器 growth 时点钩子（conc=1，与「执行全部待办」同管线）
//!
//! 进程内单实例（`OnceLock<Mutex<_>>`）。桌面端与 WebUI-server 是两个进程，
//! 队列状态不共享——使用约定为「从一处发起」；重复点击由 409 语义的
//! `started=false + message` 挡住。
//!
//! D4 门控（同 panel）：成长任务体系仅 CN，Global 账号整账号跳过，不发起上游调用。

use std::collections::HashSet;
use std::sync::OnceLock;

use serde_json::{json, Value};

use crate::modules::account::load_accounts_for;
use crate::modules::config::now_ms;
use crate::modules::region::Region;
use tokio::sync::{Mutex, Semaphore};

use super::autotasks::{run_auto_action_inner, try_lock_account, unlock_account, auto_action_index, has_auto_action};
use super::tasks::{list_tasks, GrowthIo, GrowthTask, RealGrowthIo, ACTION_GAP};

/// 队列执行单元（对照 panel `queueItem`）。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueItem {
    pub uid: String,
    pub nickname: String,
    /// 固定 `growth`（保留多 kind 扩展位）。
    pub kind: &'static str,
    pub code: String,
    /// pending | running | done | skipped | error
    pub status: String,
    pub message: String,
}

/// 队列运行状态。`seq` 每次启动 +1——前端只渲染「自己启动的那一轮」，
/// 执行结束后的残留 items 不会覆盖后续扫描结果视图。
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskQueueState {
    pub running: bool,
    /// 启动时间（ms）；0 = 从未启动过。
    pub started_at: i64,
    pub items: Vec<QueueItem>,
    /// 本轮并发数。
    pub conc: u32,
    pub seq: u32,
}

static QUEUE: OnceLock<Mutex<TaskQueueState>> = OnceLock::new();

fn queue() -> &'static Mutex<TaskQueueState> {
    QUEUE.get_or_init(|| Mutex::new(TaskQueueState::default()))
}

/// 队列状态快照（轮询用）。
pub async fn queue_status() -> Value {
    let q = queue().lock().await;
    json!({
        "running": q.running,
        "total": q.items.len(),
        "conc": q.conc,
        "started": q.started_at != 0,
        "startedAt": q.started_at,
        "seq": q.seq,
        "items": q.items,
    })
}

// ---------------------------------------------------------------------------
// 扫描（只读）
// ---------------------------------------------------------------------------

/// 单账号扫描结果。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanAccount {
    pub uid: String,
    pub nickname: String,
    /// 未完成且可自动化的待办任务。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub growth: Vec<GrowthTask>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub growth_error: String,
}

/// 拉取单账号待办（默认 + mp 合并去重，按动作依赖序；mp 列表失败静默）。
async fn account_pending(io: &dyn GrowthIo, region: Region, account: &Value) -> Vec<GrowthTask> {
    let mut out = Vec::new();
    if let Ok(tasks) = list_tasks(io, region, account, false).await {
        for t in tasks {
            if is_growth_pending(&t) {
                out.push(t);
            }
        }
    }
    if let Ok(mp_tasks) = list_tasks(io, region, account, true).await {
        let seen: HashSet<String> =
            out.iter().map(|t| t.task_code.clone()).collect();
        for t in mp_tasks {
            if !seen.contains(&t.task_code) && is_growth_pending(&t) {
                out.push(t);
            }
        }
    }
    out.sort_by_key(|t| auto_action_index(&t.task_code));
    out
}

fn is_growth_pending(t: &GrowthTask) -> bool {
    // 与 tasks::is_growth_pending 同判据 + 「有对应自动动作」。
    super::tasks::is_growth_pending(t, has_auto_action(&t.task_code))
        && !(t.target > 0 && t.current >= t.target)
}

/// 扫描全部 CN 账号的成长任务待办（只读，并发拉取）。
/// Global 账号整体跳过（D4 门控）。
pub async fn scan_all(region: Region) -> Value {
    if region == Region::Global {
        return json!({"ok": true, "accounts": [], "pendingCount": 0, "message": "成长任务体系仅 CN"});
    }
    let accounts = load_accounts_for(Region::Cn);
    let mut items: Vec<ScanAccount> = Vec::new();
    let mut pending = 0usize;
    // 账号数个位数，直接并发（tokio join）。
    let futures: Vec<_> = accounts
        .into_iter()
        .map(|account| {
            tokio::spawn(async move {
                let io = RealGrowthIo;
                let mut it = ScanAccount {
                    uid: acc_field(&account, "uid"),
                    nickname: acc_field(&account, "nickname"),
                    growth: Vec::new(),
                    growth_error: String::new(),
                };
                it.growth = account_pending(&io, region, &account).await;
                it
            })
        })
        .collect();
    for handle in futures {
        if let Ok(it) = handle.await {
            pending += it.growth.len();
            items.push(it);
        }
    }
    json!({
        "ok": true,
        "accounts": items,
        "pendingCount": pending
    })
}

fn acc_field(account: &Value, key: &str) -> String {
    account.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

// ---------------------------------------------------------------------------
// 执行队列
// ---------------------------------------------------------------------------

/// 启动执行队列：先扫描，把全部待办排队执行。
/// 返回 `{ok, started, total?, seq?, message?}`：
/// - 队列已在跑 → `started=false` + 提示（前端 409 语义）
/// - 无待办 → `started=false` + 「全部账号没有待办任务」
/// - 正常 → `started=true, total, seq`
pub async fn start_growth_queue(region: Region, concurrency: u32) -> Value {
    let conc = clamp_conc(concurrency);
    let q = queue();
    {
        let g = q.lock().await;
        if g.running {
            return json!({
                "ok": true,
                "started": false,
                "seq": -1,
                "message": "队列正在执行中（可在任务中心查看进度）"
            });
        }
        drop(g);
    }

    // 扫描待办（与 scan_all 同拉取口径，账号粒度）。
    let accounts = load_accounts_for(Region::Cn);
    let mut groups: Vec<(Value, Vec<GrowthTask>)> = Vec::new();
    let futures: Vec<_> = accounts
        .into_iter()
        .map(|account| {
            tokio::spawn(async move {
                let io = RealGrowthIo;
                let pending = account_pending(&io, region, &account).await;
                (account, pending)
            })
        })
        .collect();
    for handle in futures {
        if let Ok((account, pending_tasks)) = handle.await {
            if !pending_tasks.is_empty() {
                groups.push((account, pending_tasks));
            }
        }
    }
    if groups.is_empty() {
        return json!({
            "ok": true,
            "started": false,
            "message": "全部账号没有待办任务"
        });
    }

    // 组装队列（账号分组保持顺序，组内按依赖序）。
    let items: Vec<QueueItem> = groups
        .iter()
        .flat_map(|(account, tasks)| {
            let uid = acc_field(account, "uid");
            let nickname = acc_field(account, "nickname");
            tasks.iter().map(move |t| QueueItem {
                uid: uid.clone(),
                nickname: nickname.clone(),
                kind: "growth",
                code: t.task_code.clone(),
                status: "pending".into(),
                message: String::new(),
            })
        })
        .collect();

    let seq;
    {
        let mut g = q.lock().await;
        g.running = true;
        g.started_at = now_ms();
        g.items = items.clone();
        g.conc = conc;
        g.seq += 1;
        seq = g.seq;
        drop(g);
    }
    eprintln!("[growth] 队列启动：{} 项（并发 {conc}）", items.len());

    let total = items.len();
    tokio::spawn(run_queue_items(groups, items, conc));

    json!({"ok": true, "started": true, "total": total, "seq": seq})
}

fn clamp_conc(v: u32) -> u32 {
    v.clamp(1, 4)
}

/// 队列执行主体：账号间并发（信号量），账号内串行（per-account 锁 + 逐项顺序）。
/// 每项结果写回队列状态；结束后置 `running=false`。
async fn run_queue_items(
    groups: Vec<(Value, Vec<GrowthTask>)>,
    items: Vec<QueueItem>,
    conc: u32,
) {
    let io = RealGrowthIo;
    let region = Region::Cn;
    let sem = std::sync::Arc::new(Semaphore::new(conc as usize));
    let total = items.len();
    let mut handles = Vec::new();
    for (account, _tasks) in groups {
        let sem = sem.clone();
        handles.push(tokio::spawn(async move {
            let _permit = sem.acquire().await;
            let uid = acc_field(&account, "uid");
            // per-account 互斥：与单任务/一键完成共用同一把锁。
            if try_lock_account(&uid).is_none() {
                mark_items(&uid, |it| {
                    it.status = "skipped".into();
                    it.message = "该账号有其它任务动作在执行，跳过".into();
                })
                .await;
                return;
            }
            // 前置：批量接受该账号未接受任务（失败不阻塞——行为事件才是进度判据）。
            accept_pending(&io, region, &account).await;
            for i in 0..total {
                let code = {
                    let g = queue().lock().await;
                    match g.items.get(i) {
                        Some(it) if it.uid == uid => it.code.clone(),
                        _ => continue,
                    }
                };
                mark_one(i, "running", "").await;
                let result = run_auto_action_inner(&io, region, &account, &code).await;
                match result {
                    Ok(resp) => {
                        let msg = resp
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let status = if resp.get("ok").and_then(Value::as_bool).unwrap_or(false)
                            && !resp.get("skipped").and_then(Value::as_bool).unwrap_or(false)
                        {
                            "done"
                        } else if resp.get("skipped").and_then(Value::as_bool).unwrap_or(false)
                        {
                            "skipped"
                        } else {
                            "error"
                        };
                        mark_one(i, status, &msg).await;
                    }
                    Err(e) => {
                        mark_one(i, "error", &e).await;
                    }
                }
                tokio::time::sleep(ACTION_GAP).await; // 项间节流
            }
            unlock_account(&uid);
        }));
    }
    let q = queue();
    for h in handles {
        let _ = h.await;
    }
    {
        let mut g = q.lock().await;
        g.running = false;
    }
    eprintln!("[growth] 队列执行结束（共 {total} 项）");
}

// ---------------------------------------------------------------------------
// 队列写回辅助
// ---------------------------------------------------------------------------

async fn mark_one(index: usize, status: &str, msg: &str) {
    let mut g = queue().lock().await;
    if let Some(it) = g.items.get_mut(index) {
        it.status = status.to_string();
        it.message = msg.to_string();
    }
}

async fn mark_items(uid: &str, f: impl Fn(&mut QueueItem)) {
    let mut g = queue().lock().await;
    for it in g.items.iter_mut() {
        if it.uid == uid {
            f(it);
        }
    }
}

/// 前置：批量接受该账号未接受任务（默认口径；mp 由动作内部 accept_with_verify_mp 兜）。
/// 返回接受的个数（失败返回 0，不阻塞）。
async fn accept_pending(io: &dyn GrowthIo, region: Region, account: &Value) -> usize {
    let Ok(tasks) = list_tasks(io, region, account, false).await else {
        return 0;
    };
    let codes: Vec<String> = tasks
        .iter()
        .filter(|t| {
            !t.claimed
                && !t.locked
                && t.accept_status != "accepted"
                && t.accept_status != "completed"
        })
        .map(|t| t.task_code.clone())
        .collect();
    if codes.is_empty() {
        return 0;
    }
    if let Err(e) = super::tasks::accept_tasks(io, region, account, &codes).await {
        eprintln!("[growth] 队列 accept uid={:?}: {e}（不阻塞）", acc_field(account, "uid"));
        return 0;
    }
    eprintln!("[growth] 队列 accept uid={:?}: 已接受 {} 个任务", acc_field(account, "uid"), codes.len());
    tokio::time::sleep(ACTION_GAP).await; // 给上游状态流转留时间
    codes.len()
}

// ---------------------------------------------------------------------------
// 调度器钩子
// ---------------------------------------------------------------------------

/// 调度器 growth 时点回调（panel `RunGrowthQueueOnce` 同款）：
/// 与「执行全部待办」按钮完全同管线（串行并发 1）。Sequential 族每日零点
/// 解锁一环，此钩子让链条每天自动走一环。已在跑/无待办安全跳过。
pub async fn run_growth_queue_once(region: Region) -> Value {
    let out = start_growth_queue(region, 1).await;
    if out.get("started").and_then(Value::as_bool).unwrap_or(false) {
        eprintln!(
            "[growth] 定时成长任务队列已启动（{} 项）",
            out.get("total").and_then(Value::as_u64).unwrap_or(0)
        );
    }
    out
}

// ---------------------------------------------------------------------------
// 单测（纯逻辑部分；队列执行需要 Io 注入缝——扫描/夹取走真实 Io 但测试只
// 覆盖 clamp / 门控 / 状态初值等无网络路径）
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conc_is_clamped_to_1_4() {
        assert_eq!(clamp_conc(0), 1);
        assert_eq!(clamp_conc(3), 3);
        assert_eq!(clamp_conc(99), 4);
    }

    #[test]
    fn pending_filter_matches_panel_semantics() {
        let mk = |task_code: &str, current: i64, target: i64, claimed: bool, locked: bool| {
            GrowthTask {
                task_code: task_code.into(),
                current,
                target,
                claimed,
                locked,
                ..Default::default()
            }
        };
        // 可自动 + 未达标 → 待办。
        assert!(is_growth_pending(&mk("chat_5", 0, 5, false, false)));
        // claimed / locked → 不是待办。
        assert!(!is_growth_pending(&mk("chat_5", 0, 5, true, false)));
        assert!(!is_growth_pending(&mk("chat_5", 0, 5, false, true)));
        // 达标未领 → 不是待办（panel 口径：达标项由队列执行时直接领奖路径外，
        // 不进扫描待办）。
        assert!(!is_growth_pending(&mk("chat_5", 5, 5, false, false)));
        // 无自动动作 → 不是待办。
        assert!(!is_growth_pending(&mk("unknown_task", 0, 1, false, false)));
    }

    #[tokio::test]
    async fn global_region_scan_is_noop() {
        let out = scan_all(Region::Global).await;
        assert_eq!(out["pendingCount"], 0, "{out}");
        assert!(out["message"].as_str().unwrap_or("").contains("仅 CN"));
    }

    #[tokio::test]
    async fn queue_status_is_default_state() {
        let out = queue_status().await;
        assert_eq!(out["running"], false);
        assert_eq!(out["total"], 0);
        assert_eq!(out["seq"], 0);
    }

    #[test]
    fn all_task_codes_are_known_to_the_action_table() {
        use super::super::autotasks::ALL_TASK_CODES;
        assert!(ALL_TASK_CODES.iter().all(|c| has_auto_action(c)));
    }
}
