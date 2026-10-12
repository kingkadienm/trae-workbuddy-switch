//! TOTP 两步验证模块（Google Authenticator 兼容，RFC 6238）。
//!
//! 部署形态：
//!   - TOTP 未启用时：所有 API 直接放行（向后兼容）。
//!   - TOTP 启用后：除 `/api/auth/totp/*` 外，所有 API 需携带有效 session token。
//!
//! 持久化：`~/.buddy-switch/totp_config.json`（secret 为 base32 明文，
//! 与账号库同级存放——本地部署 + Nginx 反代场景下，文件权限即第一道防线）。

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use base32::{Alphabet, decode as base32_decode, encode as base32_encode};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use totp_rs::{Algorithm, Builder, Secret, Totp};

// ---------------------------------------------------------------------------
// 数据结构
// ---------------------------------------------------------------------------

/// TOTP 配置（持久化到磁盘）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TotpConfig {
    pub secret: String,
    pub enabled: bool,
    /// Session 有效期（分钟），前端登录后凭此值记忆。
    pub session_duration_minutes: u32,
}

impl Default for TotpConfig {
    fn default() -> Self {
        Self {
            secret: String::new(),
            enabled: false,
            session_duration_minutes: 30,
        }
    }
}

/// 内存中的 session 条目。
#[derive(Debug, Clone)]
struct Session {
    expires_at: i64,
}

use std::sync::OnceLock;

// ---------------------------------------------------------------------------
// 全局状态
// ---------------------------------------------------------------------------

static CONFIG: OnceLock<ConfigCell> = OnceLock::new();
static SESSIONS: OnceLock<SessionsCell> = OnceLock::new();

fn config_cell() -> &'static ConfigCell {
    CONFIG.get_or_init(ConfigCell::new)
}

fn sessions_cell() -> &'static SessionsCell {
    SESSIONS.get_or_init(SessionsCell::new)
}

struct ConfigCell(UnsafeCell<Option<TotpConfig>>);

impl ConfigCell {
    fn new() -> Self {
        Self(UnsafeCell::new(None))
    }

    /// 只写一次（init 阶段），安全。
    pub fn init(&self, cfg: TotpConfig) {
        unsafe { *self.0.get() = Some(cfg); }
    }

    /// 读快照（Option  clone），不持有引用。
    pub fn read(&self) -> Option<TotpConfig> {
        unsafe { (*self.0.get()).clone() }
    }

    /// 写（set + save），调用方持有 &self。
    pub fn write<F>(&self, f: F)
    where
        F: FnOnce(&mut Option<TotpConfig>),
    {
        unsafe { f(&mut *self.0.get()); }
        self.persist();
    }

    fn persist(&self) {
        let cfg = self.read();
        let path = config_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Some(ref c) = cfg {
            let _ = std::fs::write(&path, serde_json::to_string_pretty(c).unwrap_or_default());
        }
    }
}

// SAFETY：所有方法要么只写一次（init），要么只读 clone，不跨线程共享可变引用。
// Tauri 的 command 都在同一个 JS 事件循环对应的 Rust 线程上串行执行。
unsafe impl Send for ConfigCell {}
unsafe impl Sync for ConfigCell {}

struct SessionsCell(UnsafeCell<HashMap<String, Session>>);

impl SessionsCell {
    fn new() -> Self {
        Self(UnsafeCell::new(HashMap::default()))
    }

    pub fn read(&self) -> HashMap<String, Session> {
        unsafe { (*self.0.get()).clone() }
    }

    pub fn write<F>(&self, f: F)
    where
        F: FnOnce(&mut HashMap<String, Session>),
    {
        unsafe { f(&mut *self.0.get()); }
    }
}

unsafe impl Send for SessionsCell {}
unsafe impl Sync for SessionsCell {}

// ---------------------------------------------------------------------------
// 持久化
// ---------------------------------------------------------------------------

fn config_path() -> std::path::PathBuf {
    let home = std::env::var("BUDDY_SWITCH_HOME")
        .unwrap_or_else(|_| std::env::var("HOME").unwrap_or_else(|_| ".".to_string()));
    std::path::PathBuf::from(home).join("totp_config.json")
}

fn load_config() -> TotpConfig {
    let path = config_path();
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Ok(cfg) = serde_json::from_str::<TotpConfig>(&data) {
            return cfg;
        }
    }
    TotpConfig::default()
}

// ---------------------------------------------------------------------------
// 公开 API
// ---------------------------------------------------------------------------

/// 启动时从磁盘加载配置，必须调用一次。
pub fn init() {
    let cfg = load_config();
    config_cell().init(cfg);
}

/// TOTP 是否已启用。
pub fn is_enabled() -> bool {
    config_cell().read().as_ref().map(|c| c.enabled).unwrap_or(false)
}

/// 生成新密钥（首次设置）。
/// 返回 base32 编码的密钥字符串，同时写入磁盘（enabled = false）。
pub fn setup() -> Result<String, String> {
    let cfg = config_cell().read();
    if cfg.as_ref().map(|c| c.enabled).unwrap_or(false) {
        return Err("TOTP 已启用，如需重新设置请先禁用".into());
    }

    let totp = Totp::default();
    let raw_bytes = totp.secret().as_bytes().to_vec();
    let encoded = base32_encode(Alphabet::Rfc4648 { padding: true }, &raw_bytes);

    config_cell().write(|c| {
        let c = c.get_or_insert_default();
        c.secret = encoded.clone();
        c.enabled = false;
    });
    Ok(encoded)
}

/// 验证首次设置的动态口令，验证通过后启用 TOTP。
pub fn verify_setup(code: &str) -> Result<(), String> {
    let secret = config_cell().read().ok_or("TOTP 未配置，请先执行 setup")?.secret;
    let totp = new_totp(&secret)?;
    verify_code(&totp, code)?;

    config_cell().write(|c| {
        if let Some(ref mut c) = c.as_mut() {
            c.enabled = true;
        }
    });
    Ok(())
}

/// 登录验证：校验动态口令，成功则返回 session token。
pub fn validate(code: &str) -> Result<String, String> {
    if !is_enabled() {
        return Err("TOTP 未启用".into());
    }
    let secret = config_cell().read().ok_or("TOTP 未配置")?.secret;
    let totp = new_totp(&secret)?;
    verify_code(&totp, code)?;

    let token: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let ttl = config_cell().read().as_ref().map(|c| c.session_duration_minutes).unwrap_or(30).max(1) as i64 * 60;
    sessions_cell().write(|s| {
        s.insert(token.clone(), Session { expires_at: now + ttl });
    });
    Ok(token)
}

/// 校验 session token 是否有效。
pub fn check_session(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let sessions = sessions_cell().read();
    if let Some(s) = sessions.get(token) {
        if s.expires_at > now {
            return true;
        }
    }
    false
}

/// 禁用 TOTP（需提供有效验证码确认）。
pub fn disable(code: &str) -> Result<(), String> {
    if !is_enabled() {
        return Err("TOTP 未启用".into());
    }
    let secret = config_cell().read().ok_or("TOTP 未配置")?.secret;
    let totp = new_totp(&secret)?;
    verify_code(&totp, code)?;

    config_cell().write(|c| {
        if let Some(ref mut c) = c.as_mut() {
            c.enabled = false;
        }
    });
    Ok(())
}

/// 获取 otpauth URI（供前端生成二维码）。
pub fn get_otpauth_uri() -> Result<String, String> {
    let secret = config_cell().read().ok_or("TOTP 未配置")?.secret;
    let totp = new_totp(&secret)?;
    totp.to_url().map_err(|e| format!("生成 URI 失败: {e}"))
}

/// 当前 session 有效期（分钟）。
pub fn session_duration() -> u32 {
    config_cell().read().as_ref().map(|c| c.session_duration_minutes).unwrap_or(30)
}

// ---------------------------------------------------------------------------
// 内部工具
// ---------------------------------------------------------------------------

fn new_totp(encoded_secret: &str) -> Result<Totp, String> {
    let raw = base32_decode(Alphabet::Rfc4648 { padding: true }, encoded_secret.trim())
        .ok_or("base32 解码失败")?;
    let secret = Secret::new(raw.into_boxed_slice());
    Builder::new()
        .with_algorithm(Algorithm::SHA1)
        .with_digits(6)
        .with_skew(1)
        .with_step_duration(30)
        .with_secret(secret)
        .with_issuer(Some("Buddy Switch"))
        .with_account_name("admin")
        .build()
        .map_err(|e| format!("创建 TOTP 实例失败: {e}"))
}

fn verify_code(totp: &Totp, code: &str) -> Result<(), String> {
    let matched_step = totp
        .check_current(code)
        .ok_or("验证码错误，请检查")?;
    let _ = matched_step;
    Ok(())
}
