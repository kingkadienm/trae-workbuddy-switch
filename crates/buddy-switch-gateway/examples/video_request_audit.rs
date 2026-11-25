//! 离线解析最终请求并生成上游负载摘要；不创建客户端、不读取密钥、不访问网络。
use base64::{engine::general_purpose::STANDARD, Engine};
use buddy_switch_gateway::video::{models::parse_video_models, service::VideoSubmitInput};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: video_request_audit REQUEST_JSON [CATALOG_JSON]")?;
    let body: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let catalog = if let Some(path) = args.next() {
        std::fs::read_to_string(path)?
    } else {
        include_str!("../src/video/fixtures/config-cn.json").into()
    };
    let models = parse_video_models(&serde_json::from_str(&catalog)?, "cn")
        .map_err(|e| std::io::Error::other(e.message()))?;
    let payload = VideoSubmitInput::parse(&body)
        .map_err(|e| std::io::Error::other(e.message()))?
        .payload(&models)
        .map_err(|e| std::io::Error::other(e.message()))?;
    let mut images = Vec::new();
    for (index, image) in payload["reference_images"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        let url = image["image_url"].as_str().ok_or("missing image_url")?;
        if let Some((header, data)) = url.split_once(',').filter(|(h, _)| h.starts_with("data:")) {
            let bytes = STANDARD.decode(data)?;
            images.push(json!({"index":index+1,"type":header,"bytes":bytes.len(),"sha256":format!("{:x}",Sha256::digest(&bytes))}));
        } else {
            images.push(json!({"index":index+1,"type":"https","url_sha256":format!("{:x}",Sha256::digest(url.as_bytes()))}));
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"model":payload["model"],"reference_count":images.len(),"images":images,"network_requests":0,"catalog_source":"provided JSON or bundled snapshot; no live generation"})
        )?
    );
    Ok(())
}
