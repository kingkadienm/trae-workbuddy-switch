//! TOTP 两步验证模块（Google Authenticator 兼容，RFC 6238）。
//!
//! 部署形态：
//!   - TOTP 未启用时：所有 API 直接放行（向后兼容）。
//!   - TOTP 启用后：除 `/api/auth/totp/*` 外，所有 API 需携带有效 session token。
//!
//! 持久化：`~/.buddy-switch/totp_config.json`（secret 为 base32 明文，
//! 与账号库同级存放——本地部署 + Nginx 反代场景下，文件权限即第一道防线）。

use std::collections::HashMap;
use std::sync::LazyLock;
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

// ---------------------------------------------------------------------------
// 全局状态
// ---------------------------------------------------------------------------

static CONFIG: std::sync::LazyLock<std::sync::Mutex<Option<TotpConfig>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(None));
static SESSIONS: std::sync::LazyLock<std::sync::Mutex<HashMap<String, Session>>> =
    std::sync::LazyLock::new(|| std::sync::Mutex::new(HashMap::new()));

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

fn save_config(cfg: &TotpConfig) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, serde_json::to_string_pretty(cfg).unwrap_or_default());
}

// ---------------------------------------------------------------------------
// 公开 API
// ---------------------------------------------------------------------------

/// 启动时从磁盘加载配置，必须调用一次。
pub fn init() {
    let cfg = load_config();
    *CONFIG.lock().unwrap() = Some(cfg);
}

/// TOTP 是否已启用。
pub fn is_enabled() -> bool {
    CONFIG
        .lock().unwrap()
        .as_ref()
        .map(|c| c.enabled)
        .unwrap_or(false)
}

/// 生成新密钥（首次设置）。
/// 返回 base32 编码的密钥字符串，同时写入磁盘（enabled = false）。
pub fn setup() -> Result<String, String> {
    let mut cfg = CONFIG.lock().unwrap();
    if cfg.as_ref().map(|c| c.enabled).unwrap_or(false) {
        return Err("TOTP 已启用，如需重新设置请先禁用".into());
    }

    let c = cfg.get_or_insert_default();
    // 用 totp-rs 内置随机生成器生成 20 字节密钥
    let totp = Totp::default();
    let raw_bytes = totp.secret().as_bytes().to_vec();
    let encoded = base32_encode(Alphabet::Rfc4648 { padding: true }, &raw_bytes);
    c.secret = encoded;
    c.enabled = false;
    save_config(c);
    Ok(c.secret.clone())
}

/// 验证首次设置的动态口令，验证通过后启用 TOTP。
pub fn verify_setup(code: &str) -> Result<(), String> {
    let cfg = CONFIG.lock().unwrap();
    let cfg = cfg.as_ref().ok_or("TOTP 未配置，请先执行 setup")?;
    let totp = new_totp(&cfg.secret)?;
    verify_code(&totp, code)?;

    let mut cfg = CONFIG.lock().unwrap();
    if let Some(ref mut c) = cfg.as_mut() {
        c.enabled = true;
        save_config(c);
    }
    Ok(())
}

/// 登录验证：校验动态口令，成功则返回 session token。
pub fn validate(code: &str) -> Result<String, String> {
    if !is_enabled() {
        return Err("TOTP 未启用".into());
    }
    let cfg = CONFIG.lock().unwrap();
    let cfg = cfg.as_ref().ok_or("TOTP 未配置")?;
    let totp = new_totp(&cfg.secret)?;
    verify_code(&totp, code)?;

    // 生成 session token
    let token: String = rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(32)
        .map(char::from)
        .collect();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let ttl = cfg.session_duration_minutes.max(1) as i64 * 60;
    SESSIONS
        .lock().unwrap()
        .insert(token.clone(), Session { expires_at: now + ttl });
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
    let mut sessions = SESSIONS.lock().unwrap();
    if let Some(s) = sessions.get(token) {
        if s.expires_at > now {
            return true;
        }
        sessions.remove(token);
    }
    false
}

/// 禁用 TOTP（需提供有效验证码确认）。
pub fn disable(code: &str) -> Result<(), String> {
    if !is_enabled() {
        return Err("TOTP 未启用".into());
    }
    let cfg = CONFIG.lock().unwrap();
    let cfg = cfg.as_ref().ok_or("TOTP 未配置")?;
    let totp = new_totp(&cfg.secret)?;
    verify_code(&totp, code)?;

    let mut cfg = CONFIG.lock().unwrap();
    if let Some(ref mut c) = cfg.as_mut() {
        c.enabled = false;
        save_config(c);
    }
    Ok(())
}

/// 获取 otpauth URI（供前端生成二维码）。
pub fn get_otpauth_uri() -> Result<String, String> {
    let cfg = CONFIG.lock().unwrap();
    let cfg = cfg.as_ref().ok_or("TOTP 未配置")?;
    let totp = new_totp(&cfg.secret)?;
    totp.to_url().map_err(|e| format!("生成 URI 失败: {e}"))
}

/// 当前 session 有效期（分钟）。
pub fn session_duration() -> u32 {
    CONFIG
        .lock().unwrap()
        .as_ref()
        .map(|c| c.session_duration_minutes)
        .unwrap_or(30)
}

/// 清理过期 session（可定期调用）。
pub fn cleanup_expired() {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    SESSIONS
        .lock().unwrap()
        .retain(|_, s| s.expires_at > now);
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
    // matched_step 是匹配到的 time-step（非零即有效）
    let _ = matched_step;
    Ok(())
}
