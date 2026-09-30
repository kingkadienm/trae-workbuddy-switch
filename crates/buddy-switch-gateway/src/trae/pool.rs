//! Trae 网关账号池：从**磁盘上的账号库/冷却/积分缓存**派生可路由账号。
//!
//! ## 为什么池不自己存状态
//!
//! 参考实现的池把 `credits` / `cooldown_until` / `disabled` 全放在内存里，只在启动
//! 时同步一次。在本仓库里这会立刻出问题：Trae 的签到页与网关页读的是**同一批账号**，
//! 若网关只在启动时同步，用户在签到页手动「解除冷却」后网关仍会认为该账号冷却中，
//! 两个页面互相打脸。
//!
//! 因此本池的取值原则是：**每次选号都重新从磁盘派生**，唯一的例外是「选号结果」
//! 这种纯计算产物。写回也走既有通道——
//! [`buddy_switch_core::modules::trae::credits::save_cooldown_for`]，于是冷却状态的
//! 唯一真相是**该产品线自己的**冷却文件（`account_cooldowns.json` /
//! `account_cooldowns.trae_cn.json`），签到页与网关页看到的是同一份。
//!
//! **写读同源**：池构造期固化所属变体（[`TraePool::for_variant`]），写回冷却必须用
//! 该变体选文件——若写成恒定默认变体，CN 账号的冷却会落进 Work 文件、CN 池却读 CN 文件，
//! 死号永不被冷却且被反复选中（「读侧改了、写侧没改」的半成品缺陷）。
//!
//! 代价是每个请求要读 3 个小 JSON（账号库 / 冷却 / 剩余积分）。对本机单用户工具
//! 这是可接受的，且与 WorkBuddy 网关「每请求 `load_accounts_for`」的既有做法一致。

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use buddy_switch_core::modules::trae::region::TraeRegion;
use buddy_switch_core::modules::trae::variant::TraeVariant;
use buddy_switch_core::modules::trae::{account, credits, device};

/// 上游错误的治理类别。
///
/// 取值字符串与 [`credits::classify_error`] 的词汇表**逐字一致**，因此可以直接
/// 落进 `account_cooldowns.json`，被签到页原样展示。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraeErrKind {
    /// 不归因于账号（例如「模型配置为空」），不冷却。
    None,
    /// 套餐额度用尽。
    PlanLimit,
    /// 触发限流。
    SoftRate,
    /// 会话失效（凭据问题，永久禁用）。
    SessionDead,
    /// 接口不存在。
    NotFound,
    /// 上游 5xx。
    Server,
    /// 上游 4xx。
    Client,
    /// 业务错误码。
    Business,
}

impl TraeErrKind {
    /// 落盘用的类型名。
    pub fn as_str(self) -> &'static str {
        match self {
            TraeErrKind::None => "None",
            TraeErrKind::PlanLimit => "PlanLimit",
            TraeErrKind::SoftRate => "SoftRate",
            TraeErrKind::SessionDead => "SessionDead",
            TraeErrKind::NotFound => "NotFound",
            TraeErrKind::Server => "Server",
            TraeErrKind::Client => "Client",
            TraeErrKind::Business => "BusinessError",
        }
    }

    /// 是否永久禁用该账号（只有会话失效如此）。
    pub fn is_permanent(self) -> bool {
        matches!(self, TraeErrKind::SessionDead)
    }
}

/// 按 HTTP 状态码分类（上游在**建立连接阶段**就失败时使用）。
pub fn classify_http(status: u16) -> TraeErrKind {
    match status {
        401 => TraeErrKind::SessionDead,
        429 => TraeErrKind::SoftRate,
        404 => TraeErrKind::NotFound,
        code if code >= 500 => TraeErrKind::Server,
        code if code >= 400 => TraeErrKind::Client,
        _ => TraeErrKind::None,
    }
}

/// 按 SOLO 业务错误码 + 文案分类（上游在 **SSE 流内**报错时使用）。
///
/// 顺序有讲究：先看业务码，再看文案兜底，最后才按数字区间归类。把 `1005`（套餐额度）
/// 误判成 `Client` 会让一个额度耗尽的账号在 10 分钟后被反复重试。
pub fn classify_solo(code: i64, message: &str) -> TraeErrKind {
    let lower = message.to_lowercase();
    if code == 1005 || lower.contains("plan") {
        return TraeErrKind::PlanLimit;
    }
    // 模型配置为空是**模型**问题而非账号问题，冷却账号没有意义。
    if code == 4001 || lower.contains("model config is empty") {
        return TraeErrKind::None;
    }
    if code == 4008
        || lower.contains("quota")
        || lower.contains("exceeded")
        || lower.contains("rate")
    {
        return TraeErrKind::SoftRate;
    }
    match code {
        401 => TraeErrKind::SessionDead,
        429 => TraeErrKind::SoftRate,
        404 => TraeErrKind::NotFound,
        // **只有真正的 HTTP 状态码区间**才映射为 Client / Server。
        // 必须显式写区间上界：`code >= 500` 会把业务码（1005 / 4001 / 1234…）
        // 一并吞成 Server，而业务码恰恰是最需要被单独识别的一类。
        value if (400..500).contains(&value) => TraeErrKind::Client,
        value if (500..600).contains(&value) => TraeErrKind::Server,
        0 => TraeErrKind::None,
        _ => TraeErrKind::Business,
    }
}

/// 池中一个账号的可路由视图。
#[derive(Debug, Clone)]
pub struct TraePoolEntry {
    pub uid: String,
    pub name: String,
    /// 已剥离 `Cloud-IDE-JWT ` 前缀的裸令牌。
    pub jwt: String,
    pub device_id: String,
    pub machine_id: String,
    pub credits: Option<f64>,
    pub credits_expire_at: Option<i64>,
    /// 会话失效 → 永久禁用（与签到侧 `SessionDead` 语义一致）。
    pub disabled: bool,
    pub cooldown_until: i64,
    pub cooldown_reason: String,
    /// 该条目归属的产品线变体（`sync_for` 时写入，与池的变体同源）。
    ///
    /// 可用性判定需要**区域**维度（国际版免费档零积分是正常态，见
    /// [`Self::rejection`]），因此条目必须记住自己来自哪条线——`rejection`
    /// 是条目方法，拿不到池的变体。
    pub variant: TraeVariant,
}

impl TraePoolEntry {
    /// 该账号在 `now` 时刻是否可用。
    pub fn healthy(&self, now: i64) -> bool {
        !self.disabled && (self.cooldown_until == 0 || now >= self.cooldown_until)
    }

    /// 积分是否已过期（`expire_at` 缺失或为 0 视为「无过期时间」，不算过期）。
    pub fn credits_expired(&self, now: i64) -> bool {
        self.credits_expire_at
            .map(|expire| expire > 0 && expire < now)
            .unwrap_or(false)
    }

    /// 是否零积分（`None` 表示未知，不因此排除）。
    pub fn no_credits(&self) -> bool {
        self.credits.map(|value| value <= 0.0).unwrap_or(false)
    }

    /// 该账号所属区域（由池在 `sync_for` 时写入；`Default` 构造的池是国内区域）。
    fn region(&self) -> TraeRegion {
        self.variant.region()
    }

    /// 不可路由的原因；`None` 表示可用。
    ///
    /// **国际版免费档例外**（2026-09-30）：Global 没有签到/积分体系，免费计划的
    /// `credits_limit` 本就为 0（额度形态是 $1 基础用量 + 5000 次自动补全，见
    /// `variant.rs` 端点表注释）——积分恒 0 是**正常态**而非「没额度」，因此国际版
    /// 账号不因「零积分」被排除，可用性完全交给冷却/会话失效（上游 401/1005/429
    /// 仍会经 [`Self::apply_error`] 冷却兜底）。国内版维持原判定。
    pub fn rejection(&self, now: i64) -> Option<&'static str> {
        if self.disabled {
            Some("会话失效（需重新登录）")
        } else if self.cooldown_until > now {
            Some("冷却中")
        } else if self.credits_expired(now) {
            Some("积分已过期")
        } else if self.no_credits() && self.region() != TraeRegion::Global {
            Some("零积分")
        } else {
            None
        }
    }

    /// 明细视图里的状态字串：国际版免费档零积分显示「免费额度」而非 `no_credits`。
    fn credits_status(&self, now: i64) -> &str {
        if self.credits_expired(now) {
            "expired"
        } else if self.no_credits() {
            if self.region() == TraeRegion::Global {
                "free_plan"
            } else {
                "no_credits"
            }
        } else {
            "available"
        }
    }
}

/// 选中的账号（携带出站所需的全部凭据）。
#[derive(Debug, Clone)]
pub struct PickedTraeAccount {
    pub uid: String,
    pub name: String,
    pub jwt: String,
    pub device_id: String,
    pub machine_id: String,
}

/// 池状态摘要（对外响应的一部分）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TraePoolSummary {
    pub total: usize,
    pub available: usize,
    pub cooling: usize,
    pub disabled: usize,
    pub expired: usize,
    pub zero_credits: usize,
    pub total_credits: f64,
}

/// 账号池。
///
/// 每个池**绑定唯一的产品线变体**（[`Self::variant`]）：账号库 / 冷却 / 剩余积分
/// 都按该变体分家。变体在构造期固化（[`Self::for_variant`]），并在 [`Self::sync_for`]
/// 中以入参校正一次——因此「池的变体」只有一个来源，**不可能**出现「池是 CN、
/// 写盘却是 Work」这种读写分离。
#[derive(Debug, Default)]
pub struct TraePool {
    entries: Vec<TraePoolEntry>,
    /// 本池所属的产品线变体。冷却写回（[`Self::apply_error`]）据此选文件。
    variant: TraeVariant,
}

impl TraePool {
    /// 新建空池（默认变体；首次 [`Self::sync_for`] 前为空）。
    pub fn new() -> Self {
        Self::for_variant(TraeVariant::default())
    }

    /// 新建属于**指定变体**的空池。
    ///
    /// 变体在构造期固化：`apply_error` 依据它选冷却文件。配合 `sync_for` 的校正，
    /// 保证写入侧（冷却落哪个文件）与读取侧（`entries_for` / `load_*_for`）永远同源。
    pub fn for_variant(variant: TraeVariant) -> Self {
        Self {
            entries: Vec::new(),
            variant,
        }
    }

    /// 从**指定变体**的磁盘数据重建池。返回条目数。
    ///
    /// 顺序沿用账号库顺序（用户可见顺序），不排序——`pick` 的择优逻辑与顺序无关。
    ///
    /// ## 为什么必须带 `variant`
    ///
    /// 两条产品线的账号库 / 冷却 / 剩余积分**各自分家**（`*_for(variant)`）。
    /// 改造前这里只读默认变体（`account::entries()` = `entries_for(TraeWork)`），
    /// 于是**只装 Trae CN 账号的用户网关池恒为空、`pick` 永远返回 `None`、调用必然失败**——
    /// 这就是本次「池按变体分家」要修掉的既有功能洞。
    ///
    /// **设备标识不需要分家**：`device::derive(uid)` 是 uid 的纯函数、与变体无关，
    /// 因此这里直接用 `device::derive`，无需 `ensure_for_variant`。
    pub fn sync_for(&mut self, variant: TraeVariant) -> usize {
        // 以入参校正本池变体：即使该池此前由别的路径以默认值创建，这里也保证
        // 「读哪条线」与「写哪条线」一致（`apply_error` 依据 `self.variant` 落盘）。
        self.variant = variant;
        let remaining = credits::load_remaining_for(variant);
        let cooldowns = credits::load_cooldowns_for(variant);

        // 账号库按**区域**分家（见 `core::modules::trae::region`）：国内两个产品线标识
        // 读到的是**同一本**库。这里显式传 `variant.region()`，让"按区域取号"这件事
        // 在调用点就看得见，而不是靠 `*_for(variant)` 内部的隐式折算。
        self.entries = account::entries_for_region(variant.region())
            .into_iter()
            .map(|(uid, raw)| {
                let cooldown = cooldowns.cooldowns.get(&uid);
                let device = device::derive(&uid);
                TraePoolEntry {
                    jwt: clean_jwt(&raw.jwt),
                    device_id: device.device_id,
                    // `x-machine-id` 用与 session-id 不同的 salt 派生，避免两个字段撞车。
                    machine_id: device::rand_hex_salted(64, "mach", Some(&uid)),
                    name: if raw.name.trim().is_empty() {
                        uid.clone()
                    } else {
                        raw.name.clone()
                    },
                    credits: remaining.credits.get(&uid).copied(),
                    credits_expire_at: remaining.expire_times.get(&uid).copied(),
                    disabled: cooldown
                        .map(|entry| entry.error_type == "SessionDead")
                        .unwrap_or(false),
                    cooldown_until: cooldown.map(|entry| entry.until).unwrap_or(0),
                    cooldown_reason: cooldown.map(|entry| entry.reason.clone()).unwrap_or_default(),
                    variant,
                    uid,
                }
            })
            .filter(|entry| !entry.jwt.is_empty())
            .collect();

        self.entries.len()
    }

    /// 当前条目（只读）。
    pub fn entries(&self) -> &[TraePoolEntry] {
        &self.entries
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 选号。
    ///
    /// 择优规则与参考实现一致：**积分到期时间最近的优先**（先用掉快过期的额度），
    /// 到期时间相同则积分多的优先。零积分与已过期账号直接跳过，避免必然失败的请求。
    pub fn pick(&self, now: i64, tried: &HashSet<String>) -> Option<PickedTraeAccount> {
        let mut best: Option<&TraePoolEntry> = None;
        for entry in &self.entries {
            if tried.contains(&entry.uid) || entry.rejection(now).is_some() {
                continue;
            }
            best = Some(match best {
                None => entry,
                Some(current) => match (entry.credits_expire_at, current.credits_expire_at) {
                    // 有到期时间的优先于没有的。
                    (Some(_), None) => entry,
                    (None, Some(_)) => current,
                    (Some(left), Some(right)) => {
                        if left < right {
                            entry
                        } else if left > right {
                            current
                        } else if entry.credits.unwrap_or(0.0) > current.credits.unwrap_or(0.0) {
                            entry
                        } else {
                            current
                        }
                    }
                    (None, None) => {
                        if entry.credits.unwrap_or(0.0) > current.credits.unwrap_or(0.0) {
                            entry
                        } else {
                            current
                        }
                    }
                },
            });
        }
        best.map(|entry| PickedTraeAccount {
            uid: entry.uid.clone(),
            name: entry.name.clone(),
            jwt: entry.jwt.clone(),
            device_id: entry.device_id.clone(),
            machine_id: entry.machine_id.clone(),
        })
    }

    /// 记录一次上游错误：写回**本池所属变体**的冷却文件，并就地更新内存视图。
    ///
    /// `TraeErrKind::None` 不落盘（既不加冷却也不清冷却）——它是「与账号无关」的
    /// 失败，写进去只会污染状态。
    pub fn apply_error(&mut self, uid: &str, kind: TraeErrKind, reason: &str) -> bool {
        if matches!(kind, TraeErrKind::None) {
            return false;
        }
        let (error_type, cooldown_seconds) = cooldown_of(kind);
        // 关键：按**本池变体**写回冷却（`self.variant`）。若改回无变体的 `save_cooldown`，
        // 会恒定写默认产品线，导致 CN 账号的冷却落进 Work 文件、CN 池读 CN 文件读不到，
        // 死号永不被冷却且被反复选中——正是「读侧改了、写侧没改」的半成品缺陷。
        credits::save_cooldown_for(self.variant, uid, error_type, cooldown_seconds, reason);

        let now = chrono::Local::now().timestamp();
        for entry in self.entries.iter_mut().filter(|entry| entry.uid == uid) {
            if kind.is_permanent() {
                entry.disabled = true;
                entry.cooldown_until = 9_999_999_999;
            } else if cooldown_seconds > 0 {
                entry.cooldown_until = now + cooldown_seconds;
                entry.cooldown_reason = reason.to_string();
            }
        }
        kind.is_permanent()
    }

    /// 记录一次成功：只清冷却，不动账号可用性。
    ///
    /// **不**在这里 `clear_cooldown`：一次成功不代表套餐额度已恢复（`PlanLimit` 冷却
    /// 是有意为之），真正的解冻交给 `credits::refresh_all_remaining` 的自动解冻逻辑。
    pub fn note_success(&mut self, uid: &str) {
        // 内存视图与磁盘一致：成功不清冷却，但也不留下过期的冷却时间。
        let now = chrono::Local::now().timestamp();
        for entry in self.entries.iter_mut().filter(|entry| entry.uid == uid) {
            if entry.cooldown_until != 0 && entry.cooldown_until <= now {
                entry.cooldown_until = 0;
                entry.cooldown_reason.clear();
            }
        }
    }

    /// 池状态摘要。
    pub fn summary(&self, now: i64) -> TraePoolSummary {
        let mut summary = TraePoolSummary {
            total: self.entries.len(),
            ..TraePoolSummary::default()
        };
        for entry in &self.entries {
            if entry.disabled {
                summary.disabled += 1;
            } else if entry.cooldown_until > now {
                summary.cooling += 1;
            } else if entry.credits_expired(now) {
                summary.expired += 1;
            } else if entry.no_credits() && entry.variant.region() != TraeRegion::Global {
                summary.zero_credits += 1;
            } else {
                summary.available += 1;
            }
            summary.total_credits += entry.credits.unwrap_or(0.0);
        }
        summary.total_credits = (summary.total_credits * 100.0).round() / 100.0;
        summary
    }

    /// 账号明细（camelCase，与 Trae 模块其余线上形态一致）。
    pub fn status_list(&self, now: i64) -> Vec<Value> {
        self.entries
            .iter()
            .map(|entry| {
                let status = if entry.disabled {
                    "disabled"
                } else if entry.cooldown_until > now {
                    "cooling"
                } else {
                    entry.credits_status(now)
                };
                json!({
                    "uid": entry.uid,
                    "name": entry.name,
                    "status": status,
                    "credits": entry.credits,
                    "creditsExpireAt": entry.credits_expire_at,
                    "cooling": entry.cooldown_until > now,
                    "cooldownUntil": if entry.cooldown_until > 0 { Some(entry.cooldown_until) } else { None },
                    "cooldownReason": if entry.cooldown_reason.is_empty() { None } else { Some(entry.cooldown_reason.clone()) },
                    "disabled": entry.disabled,
                    "deviceIdMasked": mask_device(&entry.device_id),
                })
            })
            .collect()
    }

    /// 诊断：每个账号为什么不能路由。用于「no healthy account」的排查。
    pub fn diagnose(&self, now: i64) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| {
                let reason = entry.rejection(now).unwrap_or("可用");
                let credits = entry
                    .credits
                    .map(|value| format!("{value:.0}"))
                    .unwrap_or_else(|| "未知".to_string());
                let tail = entry.uid.get(entry.uid.len().saturating_sub(8)..).unwrap_or("");
                format!("{}({tail}:{reason},积分={credits})", entry.name)
            })
            .collect()
    }
}

/// 剥掉 `Cloud-IDE-JWT ` 前缀并去掉首尾空白。
///
/// 账号库里两种形态都可能存在（OAuth 登录存裸 token，手工粘贴常带前缀），
/// 上游只接受裸 token，因此这一步必须在出站前统一。
pub fn clean_jwt(raw: &str) -> String {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix("Cloud-IDE-JWT ")
        .unwrap_or(trimmed)
        .trim()
        .to_string()
}

/// 设备标识脱敏：只留首尾各 4 位。
fn mask_device(device_id: &str) -> String {
    if device_id.len() <= 8 {
        return device_id.to_string();
    }
    format!("{}…{}", &device_id[..4], &device_id[device_id.len() - 4..])
}

/// 错误类别 → `(落盘类型名, 冷却秒数)`。
///
/// 秒数沿用 Trae 账号模块的经验值：`-1` 表示永久。**不要**把 `SessionDead` 调小，
/// 401 是凭据失效，短冷却只会让它被反复重试并持续触发风控。
fn cooldown_of(kind: TraeErrKind) -> (&'static str, i64) {
    match kind {
        TraeErrKind::None => ("None", 0),
        TraeErrKind::PlanLimit => ("PlanLimit", 43_200),
        TraeErrKind::SoftRate => ("SoftRate", 60),
        TraeErrKind::SessionDead => ("SessionDead", -1),
        TraeErrKind::NotFound => ("NotFound", 60),
        TraeErrKind::Server => ("Server", 600),
        TraeErrKind::Client => ("Client", 600),
        TraeErrKind::Business => ("BusinessError", 300),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(uid: &str, credits: Option<f64>, expire_at: Option<i64>) -> TraePoolEntry {
        TraePoolEntry {
            uid: uid.to_string(),
            name: uid.to_string(),
            jwt: format!("jwt-{uid}"),
            device_id: "000000000000001".to_string(),
            machine_id: "0".repeat(64),
            credits,
            credits_expire_at: expire_at,
            disabled: false,
            cooldown_until: 0,
            cooldown_reason: String::new(),
            variant: TraeVariant::default(),
        }
    }

    fn entry_for(uid: &str, variant: TraeVariant, credits: Option<f64>) -> TraePoolEntry {
        let mut item = entry(uid, credits, None);
        item.variant = variant;
        item
    }

    fn pool_with(entries: Vec<TraePoolEntry>) -> TraePool {
        let mut pool = TraePool::for_variant(TraeVariant::default());
        pool.entries = entries;
        pool
    }

    #[test]
    fn clean_jwt_strips_prefix_and_whitespace() {
        assert_eq!(clean_jwt("Cloud-IDE-JWT abc"), "abc");
        assert_eq!(clean_jwt("  Cloud-IDE-JWT  abc  "), "abc");
        assert_eq!(clean_jwt("abc"), "abc");
        assert_eq!(clean_jwt("   "), "");
    }

    #[test]
    fn classify_http_covers_the_error_families() {
        assert_eq!(classify_http(401), TraeErrKind::SessionDead);
        assert_eq!(classify_http(429), TraeErrKind::SoftRate);
        assert_eq!(classify_http(404), TraeErrKind::NotFound);
        assert_eq!(classify_http(500), TraeErrKind::Server);
        assert_eq!(classify_http(503), TraeErrKind::Server);
        assert_eq!(classify_http(400), TraeErrKind::Client);
        assert_eq!(classify_http(200), TraeErrKind::None);
    }

    #[test]
    fn classify_solo_prefers_business_codes_over_ranges() {
        // 1005 必须归为套餐限额，而不是被当成 Client。
        assert_eq!(classify_solo(1005, ""), TraeErrKind::PlanLimit);
        // 4001 是模型配置问题，不该冷却账号。
        assert_eq!(classify_solo(4001, ""), TraeErrKind::None);
        assert_eq!(classify_solo(0, "model config is empty"), TraeErrKind::None);
        // 4008 是限流，短冷却。
        assert_eq!(classify_solo(4008, ""), TraeErrKind::SoftRate);
        assert_eq!(classify_solo(0, "Quota exceeded"), TraeErrKind::SoftRate);
        assert_eq!(classify_solo(0, "rate limit"), TraeErrKind::SoftRate);
        assert_eq!(classify_solo(401, ""), TraeErrKind::SessionDead);
        assert_eq!(classify_solo(500, ""), TraeErrKind::Server);
        assert_eq!(classify_solo(400, ""), TraeErrKind::Client);
        assert_eq!(classify_solo(0, ""), TraeErrKind::None);
        assert_eq!(classify_solo(1234, ""), TraeErrKind::Business);
        // 区间边界：只有 4xx/5xx 才是 Client / Server，600 起是业务码。
        assert_eq!(classify_solo(499, ""), TraeErrKind::Client);
        assert_eq!(classify_solo(500, ""), TraeErrKind::Server);
        assert_eq!(classify_solo(599, ""), TraeErrKind::Server);
        assert_eq!(classify_solo(600, ""), TraeErrKind::Business);
        // 业务码即使落在 4xx/5xx 之外的高位，也不能被吞成 Server。
        assert_eq!(classify_solo(1005, "plan limit exceeded"), TraeErrKind::PlanLimit);
    }

    #[test]
    fn rejection_prefers_disabled_then_cooldown_then_credits() {
        let now = 1_000_000;
        let mut item = entry("u", Some(10.0), Some(now + 100));
        assert!(item.rejection(now).is_none());

        item.credits = Some(0.0);
        assert_eq!(item.rejection(now), Some("零积分"));

        item.credits = Some(10.0);
        item.credits_expire_at = Some(now - 1);
        assert_eq!(item.rejection(now), Some("积分已过期"));

        item.credits_expire_at = Some(now + 100);
        item.cooldown_until = now + 60;
        assert_eq!(item.rejection(now), Some("冷却中"));

        item.disabled = true;
        assert_eq!(item.rejection(now), Some("会话失效（需重新登录）"));
    }

    #[test]
    fn global_zero_credits_is_not_a_rejection_but_cn_stays() {
        let now = 1_000_000;
        // 国际版免费档：credits_limit=0（$1 基础用量 + 5000 次自动补全），零积分是正常态，
        // 不得被「零积分」排除——否则国际版 Free 账号永远进不了池。
        let global = entry_for("g", TraeVariant::Global, Some(0.0));
        assert!(global.rejection(now).is_none(), "Global 零积分不应不可路由");
        assert_eq!(global.credits_status(now), "free_plan");

        // 同形状国内账号：零积分依旧不可路由。
        let cn = entry_for("c", TraeVariant::default(), Some(0.0));
        assert_eq!(cn.rejection(now), Some("零积分"));

        // Global 账号冷却/会话失效判定与国内一致（区域只豁免「零积分」，不豁免治理）。
        let mut cooling = entry_for("gc", TraeVariant::Global, Some(0.0));
        cooling.cooldown_until = now + 60;
        assert_eq!(cooling.rejection(now), Some("冷却中"));
        cooling.cooldown_until = 0;
        cooling.disabled = true;
        assert_eq!(cooling.rejection(now), Some("会话失效（需重新登录）"));
    }

    #[test]
    fn global_pool_picks_zero_credit_free_plan_account() {
        let now = 1_000_000;
        let only = entry_for("g", TraeVariant::Global, Some(0.0));
        let mut pool = TraePool::for_variant(TraeVariant::Global);
        pool.entries = vec![only];

        let picked = pool.pick(now, &HashSet::new()).expect("Global 免费档应可选");
        assert_eq!(picked.uid, "g");

        let summary = pool.summary(now);
        assert_eq!(summary.available, 1, "免费档零积分计入可用，而非 zero_credits");
        assert_eq!(summary.zero_credits, 0);

        // 明细视图：免费档显示 free_plan，诊断文案不得再报「零积分」。
        let status = pool.status_list(now);
        assert_eq!(status[0]["status"], "free_plan");
        assert_eq!(pool.diagnose(now), vec!["g(g:可用,积分=0)".to_string()]);
    }

    #[test]
    fn missing_credits_and_missing_expiry_are_not_rejections() {
        // 未知积分 / 无到期时间都不应让账号不可用——否则新账号永远选不上。
        let now = 1_000_000;
        let item = entry("u", None, None);
        assert!(item.rejection(now).is_none());
        assert!(!item.no_credits());
        assert!(!item.credits_expired(now));
    }

    #[test]
    fn pick_skips_tried_and_unhealthy_and_prefers_soonest_expiry() {
        let now = 1_000_000;
        let mut soon = entry("soon", Some(10.0), Some(now + 100));
        soon.name = "soon".to_string();
        let later = entry("later", Some(999.0), Some(now + 100_000));
        let mut cooling = entry("cooling", Some(999.0), Some(now + 10));
        cooling.cooldown_until = now + 500;
        let zero = entry("zero", Some(0.0), Some(now + 10));

        let pool = pool_with(vec![soon, later, cooling, zero]);
        let picked = pool.pick(now, &HashSet::new()).expect("应选出账号");
        assert_eq!(picked.uid, "soon", "到期最近的优先");

        // 已试过的账号不再选。
        let mut tried = HashSet::new();
        tried.insert("soon".to_string());
        assert_eq!(pool.pick(now, &tried).unwrap().uid, "later");

        // 全试过 → 没有可选。
        tried.insert("later".to_string());
        assert!(pool.pick(now, &tried).is_none());
    }

    #[test]
    fn pick_prefers_known_expiry_over_unknown() {
        let now = 1_000_000;
        let unknown = entry("unknown", Some(5000.0), None);
        let known = entry("known", Some(1.0), Some(now + 999));
        let pool = pool_with(vec![unknown, known]);
        assert_eq!(pool.pick(now, &HashSet::new()).unwrap().uid, "known");
    }

    #[test]
    fn summary_counts_every_bucket_once() {
        let now = 1_000_000;
        let available = entry("a", Some(10.0), Some(now + 100));
        let mut cooling = entry("b", Some(5.0), Some(now + 100));
        cooling.cooldown_until = now + 60;
        let mut disabled = entry("c", Some(5.0), Some(now + 100));
        disabled.disabled = true;
        let expired = entry("d", Some(5.0), Some(now - 1));
        let zero = entry("e", Some(0.0), Some(now + 100));

        let pool = pool_with(vec![available, cooling, disabled, expired, zero]);
        let summary = pool.summary(now);
        assert_eq!(summary.total, 5);
        assert_eq!(summary.available, 1);
        assert_eq!(summary.cooling, 1);
        assert_eq!(summary.disabled, 1);
        assert_eq!(summary.expired, 1);
        assert_eq!(summary.zero_credits, 1);
        assert_eq!(summary.total_credits, 25.0);
    }

    #[test]
    fn status_list_uses_camel_case_and_masks_device() {
        let now = 1_000_000;
        let mut item = entry("u1", Some(12.5), Some(now + 100));
        item.device_id = "123456789012345".to_string();
        let pool = pool_with(vec![item]);
        let list = pool.status_list(now);
        let object = list[0].as_object().unwrap();
        assert_eq!(object["status"], "available");
        assert_eq!(object["credits"], 12.5);
        assert_eq!(object["deviceIdMasked"], "1234…2345");
        // 不得出现 snake_case 键。
        assert!(!object.contains_key("credits_expire_at"));
        assert!(!object.contains_key("cooldown_until"));
    }

    #[test]
    fn diagnose_reports_reason_for_every_entry() {
        let now = 1_000_000;
        let mut cooling = entry("u1", Some(9.0), Some(now + 100));
        cooling.cooldown_until = now + 60;
        let pool = pool_with(vec![cooling]);
        let lines = pool.diagnose(now);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("冷却中"), "{}", lines[0]);
    }

    #[test]
    fn cooldown_of_maps_every_kind_to_a_documented_pair() {
        assert_eq!(cooldown_of(TraeErrKind::None), ("None", 0));
        assert_eq!(cooldown_of(TraeErrKind::SessionDead), ("SessionDead", -1));
        assert_eq!(cooldown_of(TraeErrKind::PlanLimit).1, 43_200);
        assert_eq!(cooldown_of(TraeErrKind::SoftRate).1, 60);
        // 永久禁用只属于会话失效。
        for kind in [
            TraeErrKind::PlanLimit,
            TraeErrKind::SoftRate,
            TraeErrKind::NotFound,
            TraeErrKind::Server,
            TraeErrKind::Client,
            TraeErrKind::Business,
            TraeErrKind::None,
        ] {
            assert!(!kind.is_permanent(), "{kind:?} 不该是永久");
        }
        assert!(TraeErrKind::SessionDead.is_permanent());
    }
}
