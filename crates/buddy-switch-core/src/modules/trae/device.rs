//! 确定性伪设备标识派生。
//!
//! Trae 侧存在两套「设备身份」：
//!
//! 1. **客户端文件层**（`storage.json` 的 `telemetry.machineId` / `aha.device.device_id`、
//!    `machineid` 文件、注册表 `MachineGuid` 等 6 层）——由客户端的 Electron/VS Code
//!    逻辑写入，属于第 B 组「设备隔离」的范畴，见 [`crate::modules::trae::platform`]。
//! 2. **请求头层**（`x-device-id` / `x-market-user-id` / `vscode-sessionid`）——由本模块
//!    按 `user_id` **确定性派生**，使同一账号在任何机器上、任何进程里都呈现同一套
//!    请求头身份。
//!
//! ## 为什么必须逐字节复刻参考实现的算法
//!
//! `device_map.json` 由签到逻辑与 MITM 代理**共同读写**。若两侧的派生算法有任何差异，
//! 同一账号会在签到接口与代理捕获的请求里呈现两套 `x-device-id`，上游会把它们当作
//! 两个设备，签到与积分归属随之错乱。因此这里的字节流派生必须与参考实现的
//! `_seeded_stream(seed, salt, nbytes)` 完全一致：
//!
//! ```text
//! data = f"{salt}:{seed}"            # UTF-8
//! block_i = SHA256(data + i.to_bytes(4, "big"))    # i 从 0 递增
//! stream = concat(block_0, block_1, …)[:nbytes]
//! ```
//!
//! 取字节流的**前缀**性质保证了「请求 n 字节」与「请求 n+1 字节再截断」结果相同，
//! 因此三个字段（15 / 64 / 16 字节）可以各自独立派生而互不影响。

use sha2::{Digest, Sha256};

use crate::modules::trae::store;
use crate::modules::trae::variant::TraeVariant;

/// 设备标识生成算法版本。与参考实现一致：`device_map.json` 中 `gen` 缺失（按 1 处理）
/// 或小于该值的记录会在下次取用时就地重建，从而让旧的病态派生结果被自愈替换。
pub const DEVICE_GEN: u32 = 2;

/// 一个账号的伪设备标识。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeviceEntry {
    /// `x-device-id`：15 位十进制数字串。
    pub device_id: String,
    /// `x-market-user-id`：UUID v4 形态。
    #[serde(default)]
    pub market_user_id: Option<String>,
    /// `vscode-sessionid`：64 位十六进制串。
    #[serde(default)]
    pub session_id: Option<String>,
    /// 记录创建时间（ISO8601，本地时区）。
    #[serde(default)]
    pub created: Option<String>,
    /// 派生算法版本；缺失按 1 处理（即需要重建）。
    #[serde(default)]
    pub gen: u32,
}

/// `user_id` → 设备标识的映射表（与参考实现 `device_map.json` 同构）。
pub type DeviceMap = std::collections::HashMap<String, DeviceEntry>;

/// 确定性派生一个均匀字节流：`SHA256("<salt>:<seed>" + i.to_be_bytes())` 依次拼接。
///
/// `seed` 为空返回 `None`——调用方必须显式处理「无种子」的情况，
/// 而不是悄悄退化成不可复现的随机值。
fn seeded_stream(seed: &str, salt: &str, nbytes: usize) -> Option<Vec<u8>> {
    if seed.is_empty() {
        return None;
    }
    let prefix = format!("{salt}:{seed}");
    let mut out: Vec<u8> = Vec::with_capacity(nbytes + 32);
    let mut counter: u32 = 0;
    while out.len() < nbytes {
        let mut hasher = Sha256::new();
        hasher.update(prefix.as_bytes());
        hasher.update(counter.to_be_bytes());
        out.extend_from_slice(&hasher.finalize());
        counter = counter.wrapping_add(1);
    }
    out.truncate(nbytes);
    Some(out)
}

/// 非确定性兜底：无种子时用 UUID v4 的字节。
///
/// 仅在 `user_id` 缺失的异常路径上使用；正常流程永远走确定性派生，
/// 因此「同账号同身份」这一核心保证不受影响。
fn fallback_bytes(nbytes: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(nbytes);
    while out.len() < nbytes {
        out.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    out.truncate(nbytes);
    out
}

/// 生成 `n` 位十进制数字串（`x-device-id`）。
pub fn rand_digits(n: usize, seed: Option<&str>) -> String {
    let bytes = seed
        .and_then(|s| seeded_stream(s, "devid", n))
        .unwrap_or_else(|| fallback_bytes(n));
    bytes.iter().map(|b| char::from(b'0' + (b % 10))).collect()
}

/// 生成 `n` 位十六进制串（`vscode-sessionid`）。
pub fn rand_hex(n: usize, seed: Option<&str>) -> String {
    rand_hex_salted(n, "sess", seed)
}

/// 按任意 `salt` 派生 `n` 位十六进制串。
///
/// 抽出来是为了让同一套字节流算法能服务多个字段：`vscode-sessionid` 用 `sess`，
/// API 网关的 `x-machine-id` 用 `mach`（与参考实现一致）。**必须**复用同一份
/// [`seeded_stream`]，否则同账号在两个功能里会派生出不同的设备身份。
pub fn rand_hex_salted(n: usize, salt: &str, seed: Option<&str>) -> String {
    let needed = n.div_ceil(2);
    let bytes = seed
        .and_then(|s| seeded_stream(s, salt, needed))
        .unwrap_or_else(|| fallback_bytes(needed));
    let mut hex = String::with_capacity(bytes.len() * 2);
    for byte in &bytes {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex.truncate(n);
    hex
}

/// 生成标准 UUID v4 字符串（`x-market-user-id`），按 `seed` 确定性派生。
pub fn gen_market_uuid(seed: Option<&str>) -> String {
    let mut bytes: [u8; 16] = match seed.and_then(|s| seeded_stream(s, "market", 16)) {
        Some(stream) => {
            let mut arr = [0u8; 16];
            arr.copy_from_slice(&stream);
            arr
        }
        None => *uuid::Uuid::new_v4().as_bytes(),
    };
    // 置版本位（4）与变体位（RFC 4122），使输出形态与 `uuid4()` 一致。
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;
    uuid::Uuid::from_bytes(bytes).to_string()
}

/// 按 `user_id` 派生完整设备标识（不落盘）。
pub fn derive(user_id: &str) -> DeviceEntry {
    let seed = (!user_id.is_empty()).then_some(user_id);
    DeviceEntry {
        device_id: rand_digits(15, seed),
        market_user_id: Some(gen_market_uuid(seed)),
        session_id: Some(rand_hex(64, seed)),
        created: Some(store::now_iso()),
        gen: DEVICE_GEN,
    }
}

/// 读取设备映射表（默认变体，兼容壳）。
pub fn load_map() -> DeviceMap {
    load_map_for(TraeVariant::default())
}

/// 读取设备映射表（按变体分家）。
///
/// **必须分家**：伪设备标识与账号绑定，两条产品线的 `user_id` 是两套空间。
/// 共用一份表不会报错，只会让某个账号拿到另一条产品线派生出的设备身份，
/// 表现为上游侧的设备校验偶发失败 —— 属于极难定位的静默缺陷。
pub fn load_map_for(variant: TraeVariant) -> DeviceMap {
    store::read_json(&crate::modules::trae::paths::device_map_file_for(variant))
}

/// 写入设备映射表（默认变体，兼容壳）。
pub fn save_map(map: &DeviceMap) -> Result<(), String> {
    save_map_for(TraeVariant::default(), map)
}

/// 写入设备映射表（按变体分家）。
pub fn save_map_for(variant: TraeVariant, map: &DeviceMap) -> Result<(), String> {
    store::write_json(
        &crate::modules::trae::paths::device_map_file_for(variant),
        map,
    )
}

/// 取（必要时创建）某账号的设备标识，返回该条目（默认变体，兼容壳）。
pub fn ensure_for(user_id: &str) -> Result<DeviceEntry, String> {
    ensure_for_variant(TraeVariant::default(), user_id)
}

/// 取（必要时创建）某账号的设备标识，返回该条目（按变体分家）。
///
/// 三种情况会就地重建：记录缺失、`gen` 低于 [`DEVICE_GEN`]、或记录里字段为空。
/// 重建结果按 `user_id` 确定性派生，因此「重建」对同一账号是幂等的——
/// 不会因为反复调用而产生身份漂移。
pub fn ensure_for_variant(variant: TraeVariant, user_id: &str) -> Result<DeviceEntry, String> {
    let mut map = load_map_for(variant);
    let stale = map
        .get(user_id)
        .map(|entry| {
            entry.gen < DEVICE_GEN
                || entry.device_id.is_empty()
                || entry.session_id.as_deref().unwrap_or("").is_empty()
                || entry.market_user_id.as_deref().unwrap_or("").is_empty()
        })
        .unwrap_or(true);
    if stale {
        let entry = derive(user_id);
        map.insert(user_id.to_string(), entry.clone());
        save_map_for(variant, &map)?;
        return Ok(entry);
    }
    Ok(map.get(user_id).cloned().unwrap_or_else(|| derive(user_id)))
}

/// 删除某账号的设备标识（「重置设备」的请求头层；默认变体，兼容壳）。
pub fn reset_for(user_id: &str) -> Result<bool, String> {
    reset_for_variant(TraeVariant::default(), user_id)
}

/// 删除某账号的设备标识（按变体分家）。
///
/// 只删除映射记录：下次 [`ensure_for_variant`] 会按 `user_id` 重新派生。
/// 注意：由于派生是确定性的，删除后重建得到的是**同一套 ID**——
/// 想真正换一套身份，需要连 `user_id` 一起变（新账号）或改用随机种子派生。
pub fn reset_for_variant(variant: TraeVariant, user_id: &str) -> Result<bool, String> {
    let mut map = load_map_for(variant);
    let removed = map.remove(user_id).is_some();
    if removed {
        save_map_for(variant, &map)?;
    }
    Ok(removed)
}

// ---------------------------------------------------------------------------
// 身份 A：授权 URL 用的 OAuth 登录设备身份
// ---------------------------------------------------------------------------

/// 授权 URL 用的设备身份（**变体级持久稳定**）。
///
/// ## 为什么它与另外两套身份都不同源
///
/// Trae 侧共有三套设备身份，用途与稳定性各不相同（架构 §2.3 D-id）：
///
/// | 身份 | 用途 | 取值来源 | 稳定性 |
/// |:---|:---|:---|:---|
/// | **A1. 授权 URL 的 `device_id`**（含 `x_device_id`） | 授权页 URL | **同源**：`icube.rs` 的 `DeviceIdentity.device_id`（`icube-dc` 键名内嵌值），本模块**不提供** | 机器级恒定 |
/// | **A2. 授权 URL 的 `machine_id`**（含 `x_machine_id`） | 授权页 URL | **本机自造**：本类型的 `machine_id` | 变体级稳定 |
/// | B. `DeviceInfo` 身份 | `ExchangeToken` 请求体的 `DeviceInfo.*` | `icube.rs` 的设备凭证（客户端写入，我们不生成） | 机器级恒定 |
/// | C. 签到身份 | 签到/积分请求头 `x-device-id` 等 | `device_map(.trae_cn).json` | 账号级稳定 |
///
/// ## ★ 本模块**只负责 `machine_id`**（红线）
///
/// 授权 URL 的 `device_id` **必须与 icube 设备凭证同源**（= 签名私钥所属的那个
/// `icube-dc` deviceId），否则服务端 **20403/20405**。参考 `commands/oauth.rs:259-267`
/// 逐字：「登录 URL 的 device_id 必须与之同源，否则服务端 20403/20405；恒覆盖旧值
/// （**旧值是随机/device_map 对齐的，与私钥不匹配**）」。
///
/// 因此本类型**故意不含 `device_id` 字段**——「自造 device_id」这个能力在**类型层面
/// 就不存在**。这是结构性护栏，比任何测试断言都强。取 `device_id` 请走
/// [`crate::modules::trae::icube::device_identity_for`]。
///
/// ## 与既有实现的一处行为修正
///
/// 改造前每次登录都用随机值现生成。架构裁决改为**变体级持久稳定**
/// （依据：参考实现 `load_or_create_oauth_device` 本就是持久化语义，`random_hex(32)`
/// 后写入 kv、下次读回）。代价是同变体内多个账号在授权 URL 里共享同一 `machine_id`
/// —— 真正的账号隔离由身份 C 承担，本轮不动。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OAuthLoginMachine {
    /// 无对应上游字段：首次 `random_hex(32)` 后固定。
    pub machine_id: String,
}

/// 取（必要时生成并落盘）某变体的 OAuth 登录 `machine_id`（按变体分家）。
///
/// **纯本机值，不读 `storage.json`**：授权 URL 在用户点授权之前就要打开，
/// 那时不该让「客户端装没装」决定登录能否发起（客户端装没装的后果由
/// `device_id` 那条同源红线去表达，见类型文档）。
///
/// - 文件已有非空 `machine_id` ⇒ **原样返回**（这是「稳定」的实现方式）；
/// - 文件缺失 / 字段为空 ⇒ `random_hex(32)` 生成并落盘。
///
/// 落盘失败不阻断登录（本次仍返回可用值，下次重新生成）——
/// 与 `ensure_for_variant` 的取舍一致：宁可损失一次稳定性，也不要让登录发起不了。
pub fn oauth_login_machine_for(variant: TraeVariant) -> OAuthLoginMachine {
    let path = crate::modules::trae::paths::oauth_device_file_for(variant);
    let existing: OAuthLoginMachine = store::read_json(&path);
    if !existing.machine_id.is_empty() {
        return existing;
    }
    let machine = OAuthLoginMachine {
        machine_id: crate::modules::trae::icube::random_hex(32),
    };
    let _ = store::write_json(&path, &machine);
    machine
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeded_stream_is_deterministic_and_prefix_stable() {
        let full = seeded_stream("1234567890123456", "devid", 16).unwrap();
        let short = seeded_stream("1234567890123456", "devid", 15).unwrap();
        assert_eq!(short, full[..15].to_vec(), "取前缀性质被破坏，派生会漂移");
        assert_eq!(
            seeded_stream("1234567890123456", "devid", 16).unwrap(),
            full
        );
    }

    #[test]
    fn different_salts_and_seeds_produce_different_streams() {
        let a = seeded_stream("user-a", "devid", 32).unwrap();
        let b = seeded_stream("user-a", "sess", 32).unwrap();
        let c = seeded_stream("user-b", "devid", 32).unwrap();
        assert_ne!(a, b, "同 seed 不同 salt 必须不同");
        assert_ne!(a, c, "不同 seed 必须不同");
    }

    #[test]
    fn empty_seed_yields_no_stream() {
        assert!(seeded_stream("", "devid", 16).is_none());
    }

    #[test]
    fn rand_digits_has_exact_length_and_is_stable() {
        let first = rand_digits(15, Some("1234567890123456"));
        let second = rand_digits(15, Some("1234567890123456"));
        assert_eq!(first, second);
        assert_eq!(first.len(), 15);
        assert!(first.chars().all(|c| c.is_ascii_digit()), "{first}");
    }

    #[test]
    fn rand_hex_has_exact_length_and_is_stable() {
        let first = rand_hex(64, Some("1234567890123456"));
        let second = rand_hex(64, Some("1234567890123456"));
        assert_eq!(first, second);
        assert_eq!(first.len(), 64);
        assert!(
            first
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "{first}"
        );
        // 奇数长度也必须精确截断
        assert_eq!(rand_hex(7, Some("u")).len(), 7);
    }

    #[test]
    fn rand_hex_delegates_to_salted_variant_and_salts_stay_distinct() {
        // `rand_hex` 必须是 `rand_hex_salted(n, "sess", …)` 的别名，
        // 否则 `vscode-sessionid` 会与既有落盘数据不一致。
        assert_eq!(
            rand_hex(64, Some("u")),
            rand_hex_salted(64, "sess", Some("u"))
        );
        // 不同 salt 必须给出不同结果，否则 `x-machine-id` 会与 session-id 撞车。
        assert_ne!(
            rand_hex_salted(64, "mach", Some("u")),
            rand_hex_salted(64, "sess", Some("u"))
        );
        assert_eq!(rand_hex_salted(64, "mach", Some("u")).len(), 64);
    }

    #[test]
    fn market_uuid_is_valid_v4_and_stable() {
        let first = gen_market_uuid(Some("1234567890123456"));
        assert_eq!(first, gen_market_uuid(Some("1234567890123456")));
        let parsed = uuid::Uuid::parse_str(&first).expect("必须是合法 UUID");
        assert_eq!(parsed.get_version_num(), 4, "{first}");
        assert_eq!(first.len(), 36);
        // 变体位必须是 RFC 4122（8/9/a/b）
        let variant = parsed.as_bytes()[8] >> 4;
        assert!((0x8..=0xb).contains(&variant), "变体位={variant:#x}");
    }

    #[test]
    fn derive_is_stable_across_calls_and_independent_of_other_users() {
        let a1 = derive("user-a");
        let a2 = derive("user-a");
        let b = derive("user-b");
        assert_eq!(a1.device_id, a2.device_id);
        assert_eq!(a1.session_id, a2.session_id);
        assert_eq!(a1.market_user_id, a2.market_user_id);
        assert_ne!(a1.device_id, b.device_id);
        assert_eq!(a1.gen, DEVICE_GEN);
        assert_eq!(a1.device_id.len(), 15);
        assert_eq!(a1.session_id.as_deref().unwrap().len(), 64);
    }

    #[test]
    fn derive_with_empty_user_id_never_panics_and_keeps_shape() {
        let entry = derive("");
        assert_eq!(entry.device_id.len(), 15);
        assert_eq!(entry.session_id.as_deref().unwrap().len(), 64);
        assert!(uuid::Uuid::parse_str(entry.market_user_id.as_deref().unwrap()).is_ok());
    }

    /// 与参考实现 Python 版逐字节对齐的黄金用例。
    ///
    /// 期望值由 `src-python/auto_checkin.py` 的 `_seeded_stream` 生成：
    /// `sha256(b"devid:1234567890123456" + (0).to_bytes(4,"big"))` 之后按 `% 10` 取位。
    #[test]
    fn seeded_derivation_matches_reference_bytes() {
        // 手工复算第一个 SHA-256 块，确认分段方式是 `prefix || counter_be(4)`。
        let mut hasher = Sha256::new();
        hasher.update(b"devid:1234567890123456");
        hasher.update(0u32.to_be_bytes());
        let digest = hasher.finalize();
        let expected_digits: String = digest
            .iter()
            .take(15)
            .map(|b| char::from(b'0' + (b % 10)))
            .collect();
        assert_eq!(rand_digits(15, Some("1234567890123456")), expected_digits);

        let mut hasher = Sha256::new();
        hasher.update(b"sess:1234567890123456");
        hasher.update(0u32.to_be_bytes());
        let digest = hasher.finalize();
        let mut expected_hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        expected_hex.truncate(64);
        // 第一步只产出 32 字节 = 64 个 hex 字符，恰好覆盖 64 位需求。
        assert_eq!(rand_hex(64, Some("1234567890123456")), expected_hex);
    }

    // -----------------------------------------------------------------------
    // 身份 A2：授权 URL 用的 OAuth 登录 machine_id
    // -----------------------------------------------------------------------

    /// 造一个隔离的临时 home（`HomeOverrideGuard` 内含 `env_lock()`，drop 时还原）。
    fn with_temp_home<T>(f: impl FnOnce(&std::path::Path) -> T) -> T {
        let dir = std::env::temp_dir().join(format!(
            "buddy-switch-oauth-device-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&dir).expect("临时 home 应能创建");
        let guard = crate::modules::config::HomeOverrideGuard::set(&dir);
        let out = f(&dir);
        drop(guard);
        let _ = std::fs::remove_dir_all(&dir);
        out
    }

    /// ★ 同一变体两次调用必须相等；两个变体必须**各存各的**（互不读取）。
    #[test]
    fn oauth_login_machine_is_stable_and_variant_scoped() {
        with_temp_home(|_| {
            let work_first = oauth_login_machine_for(TraeVariant::TraeWork);
            let work_second = oauth_login_machine_for(TraeVariant::TraeWork);
            assert_eq!(work_first, work_second, "同变体两次调用必须稳定");

            // 另一条产品线：文件不同 ⇒ 必须生成**另一个**值（证明没有读到 Work 的文件）。
            let cn = oauth_login_machine_for(TraeVariant::Global);
            assert_ne!(cn.machine_id, work_first.machine_id);

            // 两个文件都要真的落盘，且路径不同。
            let work_path =
                crate::modules::trae::paths::oauth_device_file_for(TraeVariant::TraeWork);
            let cn_path = crate::modules::trae::paths::oauth_device_file_for(TraeVariant::Global);
            assert_ne!(work_path, cn_path);
            assert!(
                work_path.is_file(),
                "Trae Work 的 machine_id 未落盘: {work_path:?}"
            );
            assert!(
                cn_path.is_file(),
                "Trae CN 的 machine_id 未落盘: {cn_path:?}"
            );

            // 落盘内容可被读回（形状与结构体一致）。
            let reloaded: OAuthLoginMachine = store::read_json(&work_path);
            assert_eq!(reloaded, work_first);
        });
    }

    /// ★ 首次生成必须是 `random_hex(32)` 语义（32 位 hex，不是派生、不是数字串）。
    #[test]
    fn oauth_login_machine_is_random_hex_32_on_first_call() {
        with_temp_home(|_| {
            let machine = oauth_login_machine_for(TraeVariant::TraeWork);
            assert_eq!(machine.machine_id.len(), 32, "必须是 32 位 hex");
            assert!(
                machine.machine_id.chars().all(|c| c.is_ascii_hexdigit()),
                "必须是 hex，实际为 {}",
                machine.machine_id
            );
            // 与 `icube::random_hex(32)` 同一条实现（避免两处随机源分叉）。
            let sample = crate::modules::trae::icube::random_hex(32);
            assert_eq!(sample.len(), machine.machine_id.len());
        });
    }

    /// ★ 结构性护栏：落盘文件**只有 `machine_id` 一个键**。
    ///
    /// 这条比任何行为断言都强——它直接钉死「本模块不提供自造 `device_id` 的能力」。
    /// 若有人（含未来的我）往这个结构体里加回 `device_id` / `seed`，本用例立刻红。
    #[test]
    fn oauth_login_machine_struct_has_no_device_id_field() {
        with_temp_home(|_| {
            let _ = oauth_login_machine_for(TraeVariant::TraeWork);
            let path = crate::modules::trae::paths::oauth_device_file_for(TraeVariant::TraeWork);
            let raw = std::fs::read_to_string(&path).expect("machine_id 应已落盘");
            let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
            let keys: Vec<&String> = value.as_object().unwrap().keys().collect();
            assert_eq!(
                keys,
                vec![&"machine_id".to_string()],
                "落盘键集合漂了：授权 URL 的 device_id 必须与 icube 凭证同源，\
                 本模块不得自造（自造值必然 20403/20405）"
            );
            assert!(value.get("device_id").is_none(), "不得出现 device_id");
            assert!(value.get("seed").is_none(), "不得出现 seed");
        });
    }

    /// 字段为空时**重新生成并落盘**（而不是返回一个空值让授权 URL 少参数）。
    #[test]
    fn oauth_login_machine_heals_empty_value() {
        with_temp_home(|_| {
            let path = crate::modules::trae::paths::oauth_device_file_for(TraeVariant::TraeWork);
            store::write_json(
                &path,
                &OAuthLoginMachine {
                    machine_id: String::new(),
                },
            )
            .unwrap();

            let healed = oauth_login_machine_for(TraeVariant::TraeWork);
            assert_eq!(healed.machine_id.len(), 32);
            let reloaded: OAuthLoginMachine = store::read_json(&path);
            assert_eq!(reloaded, healed, "自愈结果必须落盘");
        });
    }

    /// ★ 旧 schema 的文件必须被**安全**读入 —— 真实用户升级后人人都是这个形态。
    ///
    /// 返工前本文件存的是 `{seed, device_id, machine_id}` 三键（当时的类型叫
    /// `OAuthLoginDevice`，`device_id` 还是自造的 15 位数字）。新结构体只认
    /// `machine_id`，所以必须钉住两件事：
    ///
    /// 1. **读得进**：未知键（`seed` / `device_id`）不能让反序列化失败
    ///    —— 失败会退化成「重新随机」，登录身份随之漂移；
    /// 2. **不产出半残值**：连 `machine_id` 都没有的更早期文件必须**自愈**成
    ///    合法的 32 位 hex，而不是留下空串（空串会让授权 URL 少参数）。
    #[test]
    fn oauth_login_machine_reads_legacy_schema_safely() {
        with_temp_home(|_| {
            let path = crate::modules::trae::paths::oauth_device_file_for(TraeVariant::TraeWork);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();

            // ① 逐字复刻返工前的落盘形态。
            std::fs::write(
                &path,
                r#"{
  "seed": "ef73197b3b8e42ca8c387a677fb2e02f",
  "device_id": "087133290180569",
  "machine_id": "66d9edd2b8a323aa58e624eacce943d3"
}"#,
            )
            .unwrap();
            let machine = oauth_login_machine_for(TraeVariant::TraeWork);
            assert_eq!(
                machine.machine_id, "66d9edd2b8a323aa58e624eacce943d3",
                "旧文件里的 machine_id 必须原样保留（重新生成会让登录身份漂移）"
            );

            // ② 更早期的形态：只有 `seed` / `device_id`，没有 `machine_id`。
            std::fs::write(&path, r#"{"seed":"abc","device_id":"123456789012345"}"#).unwrap();
            let healed = oauth_login_machine_for(TraeVariant::TraeWork);
            assert_eq!(
                healed.machine_id.len(),
                32,
                "缺 machine_id 时必须自愈，不能留空"
            );
            assert!(
                healed.machine_id.chars().all(|c| c.is_ascii_hexdigit()),
                "自愈值必须是 hex，实际为 {}",
                healed.machine_id
            );
            let reloaded: OAuthLoginMachine = store::read_json(&path);
            assert_eq!(reloaded, healed, "自愈结果必须落盘");
        });
    }
}
