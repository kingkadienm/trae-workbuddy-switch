//! 真机探针：验证 Trae `storage.json` 的 `tc` 信封**在本机真实数据上**可解密。
//!
//! ## 为什么必须有一条真机探针
//!
//! 单测用的是自造信封 —— 它只能证明「我的解密实现与我自己的加密实现互为逆运算」，
//! **不能**证明「上游客户端的信封我能解开」。而本轮缺陷的教训恰恰是
//! 「229 个单测全绿、协议契约全错」。`tc` 解密是整条设备凭证链路的地基，
//! 因此必须拿真机 `storage.json` 实证一次。
//!
//! ## 只读
//!
//! 不写任何文件、不改客户端状态、**不打印任何私钥 / 令牌正文**。
//!
//! 运行：
//! `cargo test -p buddy-switch-core --test trae_icube_probe -- --ignored --nocapture`

use buddy_switch_core::modules::trae::icube;
use buddy_switch_core::modules::trae::variant::TraeVariant;

/// 标准 EC P-256 SPKI 的 DER 头。
const SPKI_PREFIX: [u8; 15] = [
    0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08,
];

/// 解析 userData 根目录。
///
/// ⚠️ Git Bash 下 `APPDATA` **可能是空字符串**（不是未设置），
/// 此时 `PathBuf::from("")` 会拼出相对路径、候选全部「不存在」。
/// 必须显式回退 `USERPROFILE\AppData\Roaming`。
fn appdata_root() -> Option<std::path::PathBuf> {
    if let Ok(value) = std::env::var("APPDATA") {
        if !value.trim().is_empty() {
            return Some(std::path::PathBuf::from(value));
        }
    }
    std::env::var("USERPROFILE")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|home| {
            std::path::PathBuf::from(home)
                .join("AppData")
                .join("Roaming")
        })
}

/// 本机是否存在至少一条产品线的 storage.json。
fn real_storage_exists() -> bool {
    let Some(root) = appdata_root() else {
        return false;
    };
    for variant in TraeVariant::all() {
        for name in buddy_switch_core::modules::trae::platform::data_dir_names_for(variant) {
            if root
                .join(name)
                .join("User")
                .join("globalStorage")
                .join("storage.json")
                .is_file()
            {
                return true;
            }
        }
    }
    false
}

#[test]
#[ignore = "探针：读取本机真实环境，默认不跑；用 --ignored 显式触发"]
fn probe_real_machine_icube_decryption() {
    assert!(
        real_storage_exists(),
        "本机未找到任何 Trae 客户端 storage.json（APPDATA={:?}，USERPROFILE={:?}）",
        std::env::var("APPDATA"),
        std::env::var("USERPROFILE")
    );

    let mut checked = 0usize;
    for variant in TraeVariant::all() {
        let label = variant.display_name();
        println!("\n=== {label}（{}）===", variant.as_str());

        // 1) 设备身份：AuthCode 路径用的结构（无私钥）。
        match icube::device_identity_for(variant) {
            Ok(identity) => {
                println!("  deviceId   = {}", identity.device_id);
                println!("  machineId  = {}", identity.machine_id);
                println!("  appVersion = {:?}", identity.app_version);
                println!("  sourceApp  = {}", identity.source_app);
                println!(
                    "  publicKeyPEM 长度 = {} 字节",
                    identity.public_key_pem.len()
                );

                // ★ 公钥必须是标准 EC P-256 SPKI。
                let body: String = identity
                    .public_key_pem
                    .lines()
                    .filter(|line| !line.starts_with("-----"))
                    .collect();
                use base64::Engine as _;
                let der = base64::engine::general_purpose::STANDARD
                    .decode(body)
                    .expect("真机公钥必须是合法 base64 SPKI");
                assert_eq!(
                    der.len(),
                    91,
                    "EC P-256 SPKI 应为 91 字节，实得 {}",
                    der.len()
                );
                assert_eq!(
                    &der[..SPKI_PREFIX.len()],
                    &SPKI_PREFIX[..],
                    "真机公钥 DER 头不是标准 EC P-256 SPKI"
                );

                // 脱敏：Debug 与诊断视图都不得含私钥。
                let debug = format!("{identity:?}");
                assert!(
                    !debug.contains("PRIVATE"),
                    "DeviceIdentity::Debug 泄漏了私钥"
                );
                let status = icube::credential_status_for(variant);
                let status_text = serde_json::to_string(&status).unwrap();
                assert!(!status_text.contains("BEGIN"), "{status_text}");
                assert!(!status_text.contains("PRIVATE KEY"), "{status_text}");
                assert_eq!(
                    status.get("available").and_then(|v| v.as_bool()),
                    Some(true)
                );
                println!("  credential_status_for = {status}");
                checked += 1;
            }
            Err(error) => {
                println!(
                    "  设备身份不可用: {}（kind={}）",
                    error.user_message(variant),
                    error.kind()
                );
            }
        }

        // 2) 设备凭证（refresh 路径）：私钥必须能解析并**真的能签名**。
        match icube::device_credential_for(variant) {
            Ok(credential) => {
                let debug = format!("{credential:?}");
                assert!(
                    debug.contains("<redacted>"),
                    "DeviceCredential::Debug 未脱敏: {debug}"
                );
                assert!(!debug.contains("BEGIN PRIVATE KEY"), "Debug 泄漏私钥正文");
                let proof = icube::device_proof(
                    &credential,
                    buddy_switch_core::modules::trae::TRAE_EXCHANGE_TOKEN_PATH,
                    buddy_switch_core::modules::trae::TRAE_OAUTH_CLIENT_ID,
                    "<refresh-token-not-printed>",
                    icube::ProofSigFormat::P1363,
                )
                .expect("真机私钥必须能完成 P-256 签名");
                use base64::Engine as _;
                let signature = base64::engine::general_purpose::STANDARD
                    .decode(proof.get("Signature").and_then(|v| v.as_str()).unwrap())
                    .expect("签名必须是 base64");
                assert_eq!(signature.len(), 64, "P1363 签名必须是 64 字节");
                println!(
                    "  DeviceProof 可用：签名 64B、Timestamp={}",
                    proof.get("Timestamp").unwrap()
                );
            }
            Err(error) => {
                println!(
                    "  设备凭证不可用: {}（kind={}）",
                    error.user_message(variant),
                    error.kind()
                );
            }
        }

        // 3) cloudide 键（同一 tc_decrypt，仅诊断用）。
        match icube::cloudide_auth_info_for(variant) {
            Ok(info) => {
                let debug = format!("{info:?}");
                assert!(
                    !debug.contains(&info.token),
                    "CloudideAuthInfo::Debug 泄漏了 token"
                );
                println!(
                    "  cloudide：host={} userRegion={:?} userId={:?} token长度={} refreshToken长度={}",
                    info.host,
                    info.user_region,
                    info.user_id,
                    info.token.len(),
                    info.refresh_token.len()
                );
            }
            Err(error) => {
                println!(
                    "  cloudide 键不可用: {}（kind={}）",
                    error.user_message(variant),
                    error.kind()
                );
            }
        }
    }

    assert!(
        checked > 0,
        "本机存在 storage.json，但没有任何一条产品线能取到设备身份 —— tc 解密链路有问题"
    );
    println!("\n✅ 真机 tc 解密链路验证通过（{checked} 条产品线）");
}
