//! Qoder COSY 推理签名 —— 出站请求的会话密钥包裹、载荷编码与签名头。
//!
//! 本模块与**官方客户端行为逐字节对齐**（算法来自对官方客户端协议的兼容性实现），
//! 三个部分：
//!
//! 1. **自定义 base64 变体**（出站请求体的编码格式）：
//!    `std = standard_base64(plain)`；按 `a = len/3` 三段轮转
//!    `std[n-a:] + std[a:n-a] + std[:a]`；再逐字符映射到自定义字母表，`'='` → `'$'`。
//! 2. **COSY 会话**：`temp_key`（16 字节会话密钥）→ RSA PKCS#1 v1.5 包裹成 `cosy_key`；
//!    身份体九个键 → 排序紧凑 JSON → AES-128-CBC（**key == iv == temp_key**）→ `info`。
//! 3. **Bearer 签名**：`payload = base64(排序紧凑 {cosyVersion, ideVersion, info, requestId, version})`；
//!    `sig = hex(md5(payload + "\n" + cosyKey + "\n" + date + "\n" + body + "\n" + path))`；
//!    `Authorization = "Bearer COSY." + payload + "." + sig`。
//!
//! ## 协议对齐的硬约束（**不要单方面「加固」**）
//!
//! - `temp_key` 是 **16 个十六进制字符**（UUID v4 的 hex 前 16 位）直接当 UTF-8 字节用作
//!   AES-128 的 key/iv —— 名义 AES-128、有效熵约 64 bit。**不能**改成 `OsRng` 的 16 随机字节：
//!   服务端用同一串字节解 `info`，字节序列一变请求整体失败（无降级路径）。
//! - 服务端 RSA 公钥是官方客户端硬编码的 **1024 位**密钥，PKCS#1 v1.5 —— 公钥必须与官方
//!   私钥配对，换更长的密钥在数学上不可能，属协议层决策。
//!
//! 以上两点是**兼容性刚需**；改它们的前提是官方客户端先换协议 + 真机全量回归。
//!
//! ## 测试向量的来源
//!
//! 确定性部分（base64 变体 / 排序 JSON / AES / md5 签名 / 设备指纹）用**与官方客户端
//! 对齐的参考行为**生成的固定向量钉死（`golden` 断言的期望值逐字节写入测试）。
//! RSA 输出含随机填充 ⇒ 用**自生成密钥对往返**验证（加密→私钥解密 == 原文）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use cbc::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
use md5::{Digest, Md5};
use rsa::pkcs8::DecodePublicKey;
use rsa::{Pkcs1v15Encrypt, RsaPublicKey};
use serde_json::{Map, Value};
use uuid::Uuid;

use super::device;

// ---------------------------------------------------------------------------
// 1. 自定义 base64 变体
// ---------------------------------------------------------------------------

/// 官方自定义 base64 字母表（64 字符，逐字对齐）。
const CUSTOM_ALPHABET: &str = "_doRTgHZBKcGVjlvpC,@aFSx#DPuNJme&i*MzLOEn)sUrthbf%Y^w.(kIQyXqWA!";
/// 标准 base64 字母表（映射的源）。
const STD_ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
/// 填充符：自定义变体用 `$` 代替 `=`。
const PAD_CHAR: u8 = b'$';

// 编译期护栏：字母表抄错一位 = 全体请求静默变成乱码（服务端 400 但看不出为什么）。
const _: () = {
    assert!(CUSTOM_ALPHABET.len() == 64, "自定义字母表必须恰 64 字符");
    assert!(STD_ALPHABET.len() == 64, "标准字母表必须恰 64 字符");
};

/// 明文 → 官方自定义 base64 变体（出站请求体格式）。
pub fn qoder_encode(plain: &[u8]) -> String {
    let std = base64::engine::general_purpose::STANDARD.encode(plain);
    let bytes = std.as_bytes();
    let n = bytes.len();
    let a = n / 3;
    // 三段轮转：尾 / 中 / 首（与官方客户端同构）。
    let rearranged = [&bytes[n - a..], &bytes[a..n - a], &bytes[..a]].concat();
    let custom = CUSTOM_ALPHABET.as_bytes();
    rearranged
        .iter()
        .map(|&b| {
            if b == b'=' {
                PAD_CHAR as char
            } else {
                match STD_ALPHABET.as_bytes().iter().position(|&x| x == b) {
                    Some(index) => custom[index] as char,
                    // 理论上不可达（标准 base64 只产字母表内字符）；保留原字符让下游响亮失败。
                    None => b as char,
                }
            }
        })
        .collect()
}

/// [`qoder_encode`] 的逆运算（测试与诊断用）。
pub fn qoder_decode(encoded: &str) -> Result<Vec<u8>, String> {
    let std: String = encoded
        .bytes()
        .map(|b| {
            if b == PAD_CHAR {
                '='
            } else {
                match CUSTOM_ALPHABET.as_bytes().iter().position(|&x| x == b) {
                    Some(index) => STD_ALPHABET.as_bytes()[index] as char,
                    None => b as char,
                }
            }
        })
        .collect();
    let bytes = std.as_bytes();
    let n = bytes.len();
    let a = n / 3;
    // 逆向：S = R3 + R2 + R1（R1/R2/R3 即正向的「首 / 中 / 尾」）。
    let (r1, rest) = bytes.split_at(a);
    let (r2, r3) = rest.split_at(n - 2 * a);
    let original = [r3, r2, r1].concat();
    base64::engine::general_purpose::STANDARD
        .decode(&original)
        .map_err(|error| format!("Qoder base64 解码失败: {error}"))
}

// ---------------------------------------------------------------------------
// 2. 排序紧凑 JSON（服务端签名字节与此强绑定）
// ---------------------------------------------------------------------------

/// 键排序 + 无空白的紧凑 JSON；`null` 归一成空字符串 `""`（与官方客户端同构）。
///
/// ⚠️ 键按**字节序**排序（ASCII 键下与码点序一致）——不要换成任何本地化排序。
pub fn json_sorted_compact(values: &Map<String, Value>) -> Vec<u8> {
    let mut keys: Vec<&String> = values.keys().collect();
    keys.sort();
    let mut out = String::from("{");
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        let key_json = serde_json::to_string(*key).unwrap_or_else(|_| "\"\"".to_string());
        let value_json = match values.get(*key) {
            // `null` 与「缺值」都按官方客户端的语义落成空字符串。
            Some(Value::Null) | None => "\"\"".to_string(),
            Some(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()),
        };
        out.push_str(&key_json);
        out.push(':');
        out.push_str(&value_json);
    }
    out.push('}');
    out.into_bytes()
}

// ---------------------------------------------------------------------------
// 3. AES-128-CBC（key == iv == temp_key）
// ---------------------------------------------------------------------------

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

/// AES-128-CBC + PKCS7 加密（Qoder 的 `info` 体用 key == iv == temp_key）。
pub fn aes_cbc_encrypt(plain: &[u8], key: &[u8; 16], iv: &[u8; 16]) -> Vec<u8> {
    Aes128CbcEnc::new(key.into(), iv.into()).encrypt_padded_vec_mut::<Pkcs7>(plain)
}

// ---------------------------------------------------------------------------
// 4. RSA PKCS#1 v1.5 公钥包裹
// ---------------------------------------------------------------------------

/// 服务端 RSA 公钥（**1024 位**，官方客户端硬编码的那把）—— 见模块头「协议对齐的硬约束」。
pub const SERVER_PUB_PEM: &str = "-----BEGIN PUBLIC KEY-----\n\
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDA8iMH5c02LilrsERw9t6Pv5Nc\n\
4k6Pz1EaDicBMpdpxKduSZu5OANqUq8er4GM95omAGIOPOh+Nx0spthYA2BqGz+l\n\
6HRkPJ7S236FZz73In/KVuLnwI8JJ2CbuJap8kvheCCZpmAWpb/cPx/3Vr/J6I17\n\
XcW+ML9FoCI6AOvOzwIDAQAB\n\
-----END PUBLIC KEY-----";

/// 解析一次、进程内复用（公钥是常量，重新解析纯属浪费）。
pub fn server_public_key() -> Result<&'static RsaPublicKey, String> {
    static KEY: OnceLock<Result<RsaPublicKey, String>> = OnceLock::new();
    KEY.get_or_init(|| {
        RsaPublicKey::from_public_key_pem(SERVER_PUB_PEM)
            .map_err(|error| format!("解析 Qoder 服务端公钥失败: {error}"))
    })
    .as_ref()
    .map_err(|error| error.clone())
}

/// RSA PKCS#1 v1.5（type 2，随机填充）加密。
pub fn rsa_pkcs1v15_encrypt(public_key: &RsaPublicKey, plain: &[u8]) -> Result<Vec<u8>, String> {
    let mut rng = rand_core::OsRng;
    public_key
        .encrypt(&mut rng, Pkcs1v15Encrypt, plain)
        .map_err(|error| format!("RSA 包裹会话密钥失败: {error}"))
}

// ---------------------------------------------------------------------------
// 5. COSY 会话与签名
// ---------------------------------------------------------------------------

/// COSY 协议版本（官方 0.4.3+ 客户端内置值；`cosy-version` 头与 payload 共用）。
pub const COSY_VERSION: &str = "1.1.64";
/// 默认用户类型（账号未携带 user_type 时的兜底）。
pub const DEFAULT_USER_TYPE: &str = "personal_professional_trial";

/// 与官方客户端同构的固定客户端 IP 头值（协议常量，不是本机真实 IP）。
const COSY_CLIENT_IP: &str = "169.254.198.161";

/// 去掉 URL path 的 `/algo` 前缀（签名只覆盖剥掉前缀后的 path）。
///
/// ⚠️ 与官方客户端**逐字对齐**：只要**以 `/algo` 开头**就剥 5 个字符
/// （`/algox` 也会被剥成 `x`）——这是兼容性行为，别「顺手修正」成按段匹配。
pub fn strip_algo_prefix(path: &str) -> &str {
    match path.strip_prefix("/algo") {
        Some(rest) => rest,
        None => path,
    }
}

/// 五行 md5 签名：`payload \n cosyKey \n date \n body \n path`。
pub fn signature(payload_b64: &str, cosy_key: &str, date: &str, body: &str, path: &str) -> String {
    let raw = [payload_b64, cosy_key, date, body, path].join("\n");
    let mut hasher = Md5::new();
    hasher.update(raw.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn now_secs() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs().to_string())
        .unwrap_or_else(|_| "0".to_string())
}

/// 单账号的 COSY 签名会话。
///
/// 机器标识按 UID 稳定派生（[`device::machine_identity`]）；`temp_key / cosy_key / info`
/// 在会话创建时生成，**access token 轮换后整体重建**（见 [`CosySessionCache`]）。
pub struct CosySession {
    /// 账号 UID。
    pub uid: String,
    machine_id: String,
    machine_token: String,
    machine_type: String,
    cosy_key: String,
    info: String,
}

/// 创建会话所需的账号字段（借用入参，避免调用方为不同来源各拼一遍结构体）。
pub struct CosyAccountInput<'a> {
    pub uid: &'a str,
    pub nickname: &'a str,
    pub access_token: &'a str,
    pub refresh_token: &'a str,
    pub user_type: Option<&'a str>,
    pub org_id: Option<&'a str>,
    pub org_name: Option<&'a str>,
}

impl CosySession {
    /// 创建会话：生成会话密钥 → RSA 包裹 → AES 加密身份体。
    pub fn new(input: &CosyAccountInput<'_>) -> Result<Self, String> {
        if input.access_token.is_empty() {
            return Err("cosy: 空的 access token，无法创建签名会话".to_string());
        }
        let identity = device::machine_identity(input.uid);
        // temp_key = UUIDv4 hex 前 16 位（协议对齐，见模块头；**不要**换 OsRng 随机字节）。
        let hex = Uuid::new_v4().simple().to_string();
        let mut temp_key = [0u8; 16];
        temp_key.copy_from_slice(&hex.as_bytes()[..16]);

        let cosy_key = base64::engine::general_purpose::STANDARD
            .encode(rsa_pkcs1v15_encrypt(server_public_key()?, &temp_key)?);

        let identity_json = {
            let mut map = Map::new();
            map.insert(
                "name".to_string(),
                Value::String(input.nickname.to_string()),
            );
            map.insert("aid".to_string(), Value::String(input.uid.to_string()));
            map.insert("uid".to_string(), Value::String(input.uid.to_string()));
            map.insert("yx_uid".to_string(), Value::String(String::new()));
            map.insert(
                "organization_id".to_string(),
                Value::String(input.org_id.unwrap_or("").to_string()),
            );
            map.insert(
                "organization_name".to_string(),
                Value::String(input.org_name.unwrap_or("").to_string()),
            );
            map.insert(
                "user_type".to_string(),
                Value::String(
                    input
                        .user_type
                        .filter(|value| !value.is_empty())
                        .unwrap_or(DEFAULT_USER_TYPE)
                        .to_string(),
                ),
            );
            map.insert(
                "security_oauth_token".to_string(),
                Value::String(input.access_token.to_string()),
            );
            map.insert(
                "refresh_token".to_string(),
                Value::String(input.refresh_token.to_string()),
            );
            map
        };
        let plain = json_sorted_compact(&identity_json);
        let info = base64::engine::general_purpose::STANDARD
            .encode(aes_cbc_encrypt(&plain, &temp_key, &temp_key));

        Ok(Self {
            uid: input.uid.to_string(),
            machine_id: identity.machine_id,
            machine_token: identity.machine_token,
            machine_type: identity.machine_type,
            cosy_key,
            info,
        })
    }

    /// 构造一次调用的 `(payload_b64, date, bearer)`。
    ///
    /// `raw_path` 传**完整 URL 的 path**（含 `/algo` 前缀，如
    /// `/algo/api/v2/service/pro/sse/agent_chat_generation`）—— 本函数内部剥前缀。
    pub fn bearer(&self, body: &str, raw_path: &str) -> (String, String, String) {
        let payload_b64 = {
            let mut payload = Map::new();
            payload.insert(
                "cosyVersion".to_string(),
                Value::String(COSY_VERSION.to_string()),
            );
            payload.insert("ideVersion".to_string(), Value::String(String::new()));
            payload.insert("info".to_string(), Value::String(self.info.clone()));
            payload.insert(
                "requestId".to_string(),
                Value::String(Uuid::new_v4().to_string()),
            );
            payload.insert("version".to_string(), Value::String("v1".to_string()));
            base64::engine::general_purpose::STANDARD.encode(json_sorted_compact(&payload))
        };
        let date = now_secs();
        let path = strip_algo_prefix(raw_path);
        let sig = signature(&payload_b64, &self.cosy_key, &date, body, path);
        let bearer = format!("Bearer COSY.{payload_b64}.{sig}");
        (payload_b64, date, bearer)
    }

    /// 推理 / 清单接口的完整 COSY 头集（`model_key` 有值时才带 `x-model-key`）。
    ///
    /// `sse=false` 用于模型清单这类 **JSON 一次性响应**（`accept: application/json`
    /// 且**不带** `cache-control: no-cache`）——与官方客户端的调用姿势逐字对齐。
    pub fn headers(
        &self,
        body: &str,
        raw_path: &str,
        model_key: Option<&str>,
        sse: bool,
    ) -> Vec<(String, String)> {
        let (_, date, bearer) = self.bearer(body, raw_path);
        let mut headers: Vec<(String, String)> = vec![
            ("cosy-data-policy".into(), "AGREE".into()),
            ("content-type".into(), "application/json".into()),
            ("cosy-machinetype".into(), self.machine_type.clone()),
            ("cosy-clienttype".into(), "5".into()),
            ("cosy-date".into(), date),
            ("cosy-user".into(), self.uid.clone()),
            ("cosy-key".into(), self.cosy_key.clone()),
            (
                "accept".into(),
                if sse {
                    "text/event-stream".to_string()
                } else {
                    "application/json".to_string()
                },
            ),
            ("cosy-clientip".into(), COSY_CLIENT_IP.into()),
            ("authorization".into(), bearer),
            ("accept-encoding".into(), "identity".into()),
            ("cosy-version".into(), COSY_VERSION.into()),
            ("cosy-machineid".into(), self.machine_id.clone()),
            ("cosy-machinetoken".into(), self.machine_token.clone()),
            ("login-version".into(), "v2".into()),
            ("user-agent".into(), "Go-http-client/2.0".into()),
        ];
        if sse {
            headers.push(("cache-control".into(), "no-cache".into()));
        }
        if let Some(key) = model_key {
            headers.push(("x-model-key".into(), key.to_string()));
            headers.push(("x-model-source".into(), "system".into()));
        }
        headers
    }
}

/// 账号 UID → [`CosySession`] 的进程级缓存；**access token 变化即失效**。
///
/// 失效判据是 token 而非时间：token 轮换后旧会话的 `info` 里还带着旧 token，
/// 上游会 401；而 token 不变时重建会话纯属浪费（RSA 包裹 + AES 都是白算）。
pub struct CosySessionCache {
    entries: Mutex<HashMap<String, (String, Arc<CosySession>)>>,
}

impl CosySessionCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// 取（或创建）会话。
    pub fn get(&self, input: &CosyAccountInput<'_>) -> Result<Arc<CosySession>, String> {
        if input.access_token.is_empty() {
            return Err("cosy: 空的 access token，无法创建签名会话".to_string());
        }
        let key = if input.uid.is_empty() {
            input.access_token.chars().take(16).collect::<String>()
        } else {
            input.uid.to_string()
        };
        {
            let guard = self
                .entries
                .lock()
                .map_err(|_| "cosy: 会话缓存锁被毒化".to_string())?;
            if let Some((token, session)) = guard.get(&key) {
                if token == input.access_token {
                    return Ok(session.clone());
                }
            }
        }
        let session = Arc::new(CosySession::new(input)?);
        let mut guard = self
            .entries
            .lock()
            .map_err(|_| "cosy: 会话缓存锁被毒化".to_string())?;
        guard.insert(key, (input.access_token.to_string(), session.clone()));
        Ok(session)
    }

    /// 主动失效（如账号被删 / 凭据更新后）。
    pub fn invalidate(&self, uid: &str) {
        if let Ok(mut guard) = self.entries.lock() {
            guard.remove(uid);
        }
    }
}

impl Default for CosySessionCache {
    fn default() -> Self {
        Self::new()
    }
}

/// 进程内共享的会话缓存（网关 / 命令面共用）。
pub fn shared_cache() -> &'static CosySessionCache {
    static CACHE: OnceLock<CosySessionCache> = OnceLock::new();
    CACHE.get_or_init(CosySessionCache::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::traits::PublicKeyParts;
    use rsa::RsaPrivateKey;

    fn b64_std() -> base64::engine::general_purpose::GeneralPurpose {
        base64::engine::general_purpose::STANDARD
    }

    // ── 1. 自定义 base64：与官方客户端对齐的固定向量 ──

    /// 期望值由与官方客户端对齐的参考行为逐字节给出（见模块头「测试向量的来源」）。
    /// 变异：把三段轮转的拼接顺序换回 `std` / 把 `$` 换回 `=` ⇒ 本条红。
    #[test]
    fn qoder_encode_matches_official_vectors() {
        let cases: &[(&str, &str)] = &[
            ("", ""),
            ("a", "$p$#"),
            ("ab", "$SB#"),
            ("abc", "MSK#"),
            ("abcd", "$$KMD_#S"),
            (r#"{"k":"v"}"#, "J*KWBMn*mYKU"),
            ("你好世界", "SQQSVPSWQG*QG(&Q"),
        ];
        for (plain, expected) in cases {
            assert_eq!(
                qoder_encode(plain.as_bytes()),
                *expected,
                "qoder_encode({plain:?}) 与官方向量不一致"
            );
        }
    }

    #[test]
    fn qoder_decode_is_the_exact_inverse() {
        for plain in [
            "",
            "a",
            "ab",
            "abc",
            "abcd",
            r#"{"k":"v"}"#,
            "你好世界",
            "x".repeat(64).as_str(),
        ] {
            let decoded = qoder_decode(&qoder_encode(plain.as_bytes())).expect("往返必须成功");
            assert_eq!(decoded, plain.as_bytes());
        }
        // 固定向量逆向。
        assert_eq!(qoder_decode("$p$#").unwrap(), b"a");
        assert_eq!(qoder_decode("").unwrap(), b"");
    }

    // ── 2. 排序紧凑 JSON ──

    #[test]
    fn json_sorted_compact_matches_official_vectors() {
        let mut map = Map::new();
        map.insert("b".into(), Value::from(1));
        map.insert("a".into(), Value::from("x"));
        map.insert("z".into(), Value::Null);
        map.insert("n".into(), Value::from("你好"));
        assert_eq!(
            String::from_utf8(json_sorted_compact(&map)).unwrap(),
            r#"{"a":"x","b":1,"n":"你好","z":""}"#
        );
        assert_eq!(
            String::from_utf8(json_sorted_compact(&Map::new())).unwrap(),
            "{}"
        );
    }

    // ── 3. AES-128-CBC（key == iv） ──

    #[test]
    fn aes_cbc_encrypt_matches_official_vector() {
        let key = b"0123456789abcdef";
        let ct = aes_cbc_encrypt(br#"{"name":"jackey","aid":"q1"}"#, key, key);
        let hex: String = ct.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "d2b4b6af77e5945ccb0e1c41f575720f5c4ddc4231efd8f41c1c294670db4621"
        );
    }

    // ── 4. 五行 md5 签名 ──

    #[test]
    fn signature_matches_official_vector() {
        assert_eq!(
            signature(
                "cGF5bG9hZA==",
                "Q09TWUtFWQ==",
                "1789000000",
                "ENCODED_BODY",
                "/api/v2/model/list"
            ),
            "93a5414037f64183e005a9577182e2ed"
        );
    }

    #[test]
    fn strip_algo_prefix_matches_official_behavior() {
        assert_eq!(
            strip_algo_prefix("/algo/api/v2/model/list"),
            "/api/v2/model/list"
        );
        assert_eq!(
            strip_algo_prefix("/api/v2/model/list"),
            "/api/v2/model/list"
        );
        // 官方客户端是「以 /algo 开头就剥 5 字符」——`/algox` 同样被剥（兼容性行为，非笔误）。
        assert_eq!(strip_algo_prefix("/algox"), "x");
        assert_eq!(strip_algo_prefix(""), "");
    }

    // ── 5. 设备指纹（与官方客户端对齐的固定向量；同时钉住 device.rs） ──

    #[test]
    fn device_identity_matches_official_vectors() {
        let identity = device::machine_identity("q1");
        assert_eq!(identity.machine_id, "abec85ce8c7ed7ebd0041ee4ca63e701");
        assert_eq!(identity.machine_type, "964afd94abdbefa690");
        assert_eq!(
            identity.machine_token,
            "2PJ8T1JfAGWIBuPPnEYyOGKqFw6DXwEMeAng__6fYlJ"
        );
    }

    // ── 6. RSA：自生成密钥对往返（随机填充 ⇒ 不能比密文，能比明文） ──

    #[test]
    fn rsa_wrap_round_trips_with_generated_keypair() {
        let mut rng = rand_core::OsRng;
        let private = RsaPrivateKey::new(&mut rng, 1024).expect("测试密钥对生成");
        let public = RsaPublicKey::from(&private);
        let wrapped = rsa_pkcs1v15_encrypt(&public, b"0123456789abcdef").expect("加密");
        // 1024 位 ⇒ 固定 128 字节密文块。
        assert_eq!(wrapped.len(), 128);
        let back = private
            .decrypt(Pkcs1v15Encrypt, &wrapped)
            .expect("私钥解密");
        assert_eq!(back, b"0123456789abcdef");
    }

    #[test]
    fn server_public_key_parses_and_is_1024_bit() {
        let key = server_public_key().expect("服务端公钥必须可解析");
        assert_eq!(key.n().bits(), 1024, "官方公钥是 1024 位（协议事实）");
        assert_eq!(key.e().to_string(), "65537");
        // 包裹 16 字节会话密钥（≤ 117 字节上限）必须成功。
        assert_eq!(
            rsa_pkcs1v15_encrypt(key, b"0123456789abcdef")
                .expect("包裹")
                .len(),
            128
        );
    }

    // ── 7. 会话与头集 ──

    fn input<'a>(access: &'a str, refresh: &'a str) -> CosyAccountInput<'a> {
        CosyAccountInput {
            uid: "q1",
            nickname: "jackey",
            access_token: access,
            refresh_token: refresh,
            user_type: None,
            org_id: None,
            org_name: None,
        }
    }

    #[test]
    fn session_builds_well_formed_bearer_and_headers() {
        let session = CosySession::new(&input("dt-test", "drt-test")).expect("建会话");
        // cosy_key = base64(128 字节 RSA 块) ⇒ 解码回 128 字节。
        let (payload_b64, date, bearer) = session.bearer("ENCODED", "/algo/api/v2/model/list");
        // date 是纯数字的 unix 秒。
        assert!(date.chars().all(|c| c.is_ascii_digit()), "{date}");
        // bearer = "Bearer COSY." + payload + "." + 32 位 hex 签名。
        let parts: Vec<&str> = bearer.splitn(2, ' ').collect();
        assert_eq!(parts[0], "Bearer");
        let token = parts[1];
        assert!(token.starts_with("COSY."), "{token}");
        let segs: Vec<&str> = token["COSY.".len()..].split('.').collect();
        assert_eq!(segs.len(), 2, "bearer 应是 COSY.<payload>.<sig>");
        assert_eq!(segs[0], payload_b64);
        assert_eq!(segs[1].len(), 32);
        assert!(segs[1].chars().all(|c| c.is_ascii_hexdigit()));
        // payload 解码后是带 cosyVersion / version / requestId 的排序 JSON。
        let payload_json: Value =
            serde_json::from_slice(&b64_std().decode(segs[0]).expect("payload 是 base64"))
                .expect("payload 是 JSON");
        assert_eq!(payload_json["cosyVersion"], COSY_VERSION);
        assert_eq!(payload_json["version"], "v1");
        assert_eq!(payload_json["ideVersion"], "");
        assert!(Uuid::parse_str(payload_json["requestId"].as_str().unwrap()).is_ok());

        // 头集齐备；model_key 出现时才带 x-model-key。
        let headers = session.headers("ENCODED", "/algo/api/v2/model/list", Some("ultimate"), true);
        let lookup = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.clone())
        };
        for key in [
            "cosy-data-policy",
            "content-type",
            "cosy-machinetype",
            "cosy-clienttype",
            "cosy-date",
            "cosy-user",
            "cosy-key",
            "accept",
            "cosy-clientip",
            "authorization",
            "accept-encoding",
            "cosy-version",
            "cosy-machineid",
            "cosy-machinetoken",
            "login-version",
            "user-agent",
            "cache-control",
            "x-model-key",
            "x-model-source",
        ] {
            assert!(lookup(key).is_some(), "缺头 {key}");
        }
        assert_eq!(lookup("cosy-version").unwrap(), COSY_VERSION);
        assert_eq!(lookup("cosy-user").unwrap(), "q1");
        assert_eq!(lookup("cosy-machinetype").unwrap(), "964afd94abdbefa690");
        assert_eq!(lookup("x-model-key").unwrap(), "ultimate");
        assert_eq!(lookup("x-model-source").unwrap(), "system");
    }

    #[test]
    fn session_rejects_empty_access_token() {
        let err = CosySession::new(&input("", "drt"))
            .err()
            .expect("空 token 必须失败");
        assert!(err.contains("access token"), "{err}");
        let cache = CosySessionCache::new();
        assert!(cache.get(&input("", "drt")).is_err());
    }

    // ── 8. 会话缓存：token 变化即失效 ──

    /// 变异：把「token 相等才复用」改成「uid 相等就复用」⇒ 本条红
    /// （旧发布会拿着轮换前的 token 去调用 ⇒ 上游 401）。
    #[test]
    fn cache_reuses_until_token_changes() {
        let cache = CosySessionCache::new();
        let a = cache.get(&input("token-1", "r")).expect("第一次");
        let b = cache.get(&input("token-1", "r")).expect("同 token 复用");
        assert!(Arc::ptr_eq(&a, &b), "同 token 必须复用同一会话");

        let c = cache.get(&input("token-2", "r")).expect("换 token 重建");
        assert!(!Arc::ptr_eq(&a, &c), "token 变化必须重建会话");

        cache.invalidate("q1");
        let d = cache.get(&input("token-2", "r")).expect("失效后重建");
        assert!(!Arc::ptr_eq(&c, &d), "invalidate 后必须重建");
    }
}
