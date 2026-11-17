//! 视频模型目录：仅接受上游声明的 videoGeneration 能力。
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::provider::response_body;
use crate::error::GatewayError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoModel {
    pub id: String,
    pub region: String,
    pub supports_audio: bool,
    pub seconds: Vec<u64>,
    pub resolutions: Vec<String>,
    pub aspect_ratios: Vec<String>,
    pub min_seconds: u64,
    pub max_seconds: u64,
    pub default_seconds: u64,
    pub default_resolution: String,
    pub default_aspect_ratio: String,
    pub audio_mode: AudioMode,
    pub default_enable_audio: bool,
    pub max_prompt_length: usize,
    pub supports_first_frame: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AudioMode {
    Switchable,
    AlwaysOn,
    AlwaysOff,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities {
    min_seconds: u64,
    max_seconds: u64,
    default_seconds: u64,
    supported_seconds: Vec<u64>,
    resolutions: Vec<String>,
    default_resolution: String,
    aspect_ratios: Vec<String>,
    default_aspect_ratio: String,
    audio_mode: AudioMode,
    default_enable_audio: bool,
    max_prompt_length: usize,
    supports_first_frame: bool,
}

pub fn parse_video_models(value: &Value, region: &str) -> Result<Vec<VideoModel>, GatewayError> {
    let body = response_body(value)?;
    let entries = body
        .get("models")
        .and_then(Value::as_array)
        .ok_or_else(|| GatewayError::Internal("上游视频目录缺少 data.models 数组".into()))?;
    let mut models = Vec::new();
    for entry in entries {
        let Some(capabilities) = entry.get("videoGeneration").filter(|v| !v.is_null()) else {
            continue;
        };
        if !capabilities.is_object() {
            return Err(GatewayError::Internal(
                "上游 videoGeneration 必须是对象".into(),
            ));
        }
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| GatewayError::Internal("上游视频模型缺少 id".into()))?;
        let c: Capabilities = serde_json::from_value(capabilities.clone()).map_err(|error| {
            GatewayError::Internal(format!("上游视频模型 {id} 能力不完整：{error}"))
        })?;
        if c.min_seconds == 0
            || c.max_seconds < c.min_seconds
            || !(c.min_seconds..=c.max_seconds).contains(&c.default_seconds)
            || c.supported_seconds
                .iter()
                .any(|s| !(c.min_seconds..=c.max_seconds).contains(s))
            || (!c.supported_seconds.is_empty()
                && !c.supported_seconds.contains(&c.default_seconds))
            || c.resolutions.is_empty()
            || c.aspect_ratios.is_empty()
            || c.resolutions
                .iter()
                .chain(c.aspect_ratios.iter())
                .any(|s| s.trim().is_empty())
            || !c.resolutions.contains(&c.default_resolution)
            || !c.aspect_ratios.contains(&c.default_aspect_ratio)
            || c.max_prompt_length == 0
            || (c.audio_mode == AudioMode::AlwaysOn && !c.default_enable_audio)
            || (c.audio_mode == AudioMode::AlwaysOff && c.default_enable_audio)
            || models
                .iter()
                .any(|m: &VideoModel| m.id.eq_ignore_ascii_case(id))
        {
            return Err(GatewayError::Internal(format!(
                "上游视频模型 {id} 能力声明不一致"
            )));
        }
        models.push(VideoModel {
            id: id.into(),
            region: region.into(),
            supports_audio: c.audio_mode != AudioMode::AlwaysOff,
            seconds: c.supported_seconds,
            resolutions: c.resolutions,
            aspect_ratios: c.aspect_ratios,
            min_seconds: c.min_seconds,
            max_seconds: c.max_seconds,
            default_seconds: c.default_seconds,
            default_resolution: c.default_resolution,
            default_aspect_ratio: c.default_aspect_ratio,
            audio_mode: c.audio_mode,
            default_enable_audio: c.default_enable_audio,
            max_prompt_length: c.max_prompt_length,
            supports_first_frame: c.supports_first_frame,
        });
    }
    if models.is_empty() {
        return Err(GatewayError::Internal(format!(
            "上游 {region} 目录未提供任何视频模型，无法生成视频"
        )));
    }
    Ok(models)
}

#[cfg(test)]
pub(crate) fn fixture(region: &str) -> Vec<VideoModel> {
    let text = match region {
        "cn" => include_str!("fixtures/config-cn.json"),
        _ => include_str!("fixtures/config-global.json"),
    };
    parse_video_models(&serde_json::from_str(text).unwrap(), region).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn real_catalog_filters_non_video_and_isolates_regions() {
        let cn = fixture("cn");
        let global = fixture("global");
        assert_eq!(
            cn.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["kling-video-v3", "minimax-video-h3"]
        );
        assert_eq!(global.len(), 1);
        assert_eq!(global[0].id, "kling-video-v3");
        assert_eq!(
            (cn[0].min_seconds, cn[0].max_seconds, cn[0].default_seconds),
            (3, 15, 5)
        );
        assert_eq!(cn[0].resolutions, ["720P", "1080P", "4K"]);
        assert_eq!(cn[1].audio_mode, AudioMode::AlwaysOn);
        assert_eq!(cn[1].default_resolution, "768P");
        assert_eq!(cn[1].default_aspect_ratio, "adaptive");
    }

    #[test]
    fn missing_empty_malformed_and_business_error_catalogs_fail() {
        for value in [
            json!({}),
            json!({"data":{"models":[]}}),
            json!({"data":{"models":[{"id":"video","videoGeneration":{}}]}}),
            json!({"code":42,"data":{"models":[]}}),
        ] {
            assert!(parse_video_models(&value, "cn").is_err());
        }
        let mut value: Value =
            serde_json::from_str(include_str!("fixtures/config-cn.json")).unwrap();
        value["code"] = json!(123);
        assert!(parse_video_models(&value, "cn")
            .unwrap_err()
            .message()
            .contains("123"));
    }
}
