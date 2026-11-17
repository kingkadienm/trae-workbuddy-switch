//! 视频任务编排：真实目录校验、固定账号提交和按 Key 隔离的任务查询。
use std::future::Future;

use buddy_switch_core::modules::account;
use serde::Deserialize;
use serde_json::{json, Value};

use super::models::{AudioMode, VideoModel};
use super::provider::{read_response, response_body, VideoProvider};
use super::store::{VideoTask, VideoTaskStatus, VideoTaskStore};
use crate::apikey::{account_allowed, ApiKeyRecord, QuotaMode};
use crate::error::{GatewayError, NoCredentialCause};
use crate::state::GatewayState;

#[derive(Debug, Clone, Deserialize)]
pub struct VideoSubmitInput {
    pub model: String,
    pub prompt: String,
    pub seconds: Option<u64>,
    pub resolution: Option<String>,
    pub aspect_ratio: Option<String>,
    pub enable_audio: Option<bool>,
    pub image_url: Option<String>,
    pub negative_prompt: Option<String>,
    #[serde(default)]
    pub watermark: bool,
    pub extra_parameters: Option<ExtraParameters>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtraParameters {
    resolution: Option<String>,
    aspect_ratio: Option<String>,
    enable_audio: Option<bool>,
}

fn bad(message: impl Into<String>) -> GatewayError {
    GatewayError::BadRequest(message.into())
}

fn merge_parameter<T: PartialEq + Clone>(
    top: &Option<T>,
    extra: &Option<T>,
    name: &str,
) -> Result<Option<T>, GatewayError> {
    if let (Some(a), Some(b)) = (top, extra) {
        if a != b {
            return Err(bad(format!("{name} 与 extra_parameters.{name} 冲突")));
        }
    }
    Ok(top.clone().or_else(|| extra.clone()))
}

impl VideoSubmitInput {
    pub fn parse(body: &Value) -> Result<Self, GatewayError> {
        for key in [
            "seconds",
            "resolution",
            "aspect_ratio",
            "enable_audio",
            "image_url",
            "negative_prompt",
            "extra_parameters",
        ] {
            if body.get(key).is_some_and(Value::is_null) {
                return Err(bad(format!("{key} 不能为 null")));
            }
        }
        if let Some(extra) = body.get("extra_parameters").and_then(Value::as_object) {
            if extra.values().any(Value::is_null) {
                return Err(bad("extra_parameters 中的参数不能为 null"));
            }
        }
        let mut input: Self = serde_json::from_value(body.clone())
            .map_err(|error| bad(format!("视频参数格式错误：{error}")))?;
        input.model = input.model.trim().into();
        if input.model.is_empty() || input.prompt.trim().is_empty() {
            return Err(bad("model 和 prompt 必须是非空字符串"));
        }
        if input.seconds == Some(0) {
            return Err(bad("seconds 必须是正整数"));
        }
        if let Some(extra) = &input.extra_parameters {
            input.resolution = merge_parameter(&input.resolution, &extra.resolution, "resolution")?;
            input.aspect_ratio =
                merge_parameter(&input.aspect_ratio, &extra.aspect_ratio, "aspect_ratio")?;
            input.enable_audio =
                merge_parameter(&input.enable_audio, &extra.enable_audio, "enable_audio")?;
        }
        if let Some(image) = &input.image_url {
            let url = url::Url::parse(image).map_err(|_| bad("image_url 必须是有效 HTTPS URL"))?;
            if image.trim() != image
                || url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || url.fragment().is_some()
            {
                return Err(bad("image_url 必须是有效且不含凭据或片段的 HTTPS URL"));
            }
        }
        Ok(input)
    }

    pub fn payload(&self, models: &[VideoModel]) -> Result<Value, GatewayError> {
        let model = models
            .iter()
            .find(|m| m.id.eq_ignore_ascii_case(&self.model))
            .ok_or_else(|| {
                bad(format!(
                    "模型 `{}` 不在该 region 的上游视频目录中",
                    self.model
                ))
            })?;
        let seconds = self.seconds.unwrap_or(model.default_seconds);
        if !(model.min_seconds..=model.max_seconds).contains(&seconds)
            || (!model.seconds.is_empty() && !model.seconds.contains(&seconds))
        {
            return Err(bad(format!(
                "seconds 不符合模型 {} 能力（{}..={}，档位 {:?}）",
                model.id, model.min_seconds, model.max_seconds, model.seconds
            )));
        }
        if self.prompt.encode_utf16().count() > model.max_prompt_length {
            return Err(bad(format!(
                "prompt 超过模型 {} 的 {} 字符上限",
                model.id, model.max_prompt_length
            )));
        }
        let resolution = self
            .resolution
            .as_deref()
            .unwrap_or(&model.default_resolution);
        let resolution = model
            .resolutions
            .iter()
            .find(|s| s.eq_ignore_ascii_case(resolution))
            .ok_or_else(|| bad(format!("resolution 不受模型 {} 支持", model.id)))?;
        let aspect_ratio = self.aspect_ratio.as_deref().or_else(|| {
            self.image_url
                .is_none()
                .then_some(model.default_aspect_ratio.as_str())
        });
        let aspect_ratio = aspect_ratio
            .map(|ratio| {
                model
                    .aspect_ratios
                    .iter()
                    .find(|s| s.as_str() == ratio)
                    .ok_or_else(|| bad(format!("aspect_ratio 不受模型 {} 支持", model.id)))
            })
            .transpose()?;
        let enable_audio = self.enable_audio.unwrap_or(model.default_enable_audio);
        if (model.audio_mode == AudioMode::AlwaysOn && !enable_audio)
            || (model.audio_mode == AudioMode::AlwaysOff && enable_audio)
        {
            return Err(bad(format!(
                "enable_audio 不符合模型 {} 的 {:?} 能力",
                model.id, model.audio_mode
            )));
        }
        if self.image_url.is_some() && !model.supports_first_frame {
            return Err(bad(format!("模型 {} 不支持图生首帧", model.id)));
        }
        let mut extra = json!({"resolution":resolution, "enable_audio":enable_audio});
        if let Some(ratio) = aspect_ratio {
            extra["aspect_ratio"] = json!(ratio);
        }
        let mut payload = json!({"model":model.id, "prompt":self.prompt, "seconds":seconds,
            "watermark":self.watermark, "extra_parameters":extra});
        if let Some(image) = &self.image_url {
            payload["image_url"] = json!(image);
        }
        if let Some(negative) = &self.negative_prompt {
            payload["negative_prompt"] = json!(negative);
        }
        Ok(payload)
    }
}

pub fn ensure_supported_key(key: &ApiKeyRecord) -> Result<(), GatewayError> {
    if key.quota_mode == QuotaMode::Limited {
        return Err(bad(
            "Video P1 尚不支持配额受限 Key 的视频计量，请使用无配额限制的 Key",
        ));
    }
    Ok(())
}

fn usable_account(value: &Value, key: &ApiKeyRecord) -> bool {
    let Some(uid) = account::get_str(value, "uid").filter(|s| !s.trim().is_empty()) else {
        return false;
    };
    value.get("deactivated").and_then(Value::as_bool) != Some(true)
        && account::get_str(value, "access_token").is_some_and(|s| !s.trim().is_empty())
        && account::envelope_token_error(value).is_none()
        && crate::account_strategy::ensure_region(value, key.region).is_ok()
        && account_allowed(key, &uid, account::get_str(value, "groupName").as_deref())
}

fn select_account(key: &ApiKeyRecord, uid: Option<&str>) -> Result<Value, GatewayError> {
    account::load_accounts_for(key.region)
        .into_iter()
        .find(|acc| {
            usable_account(acc, key)
                && uid.is_none_or(|uid| account::get_str(acc, "uid").as_deref() == Some(uid))
        })
        .ok_or(GatewayError::NoCredential {
            region: key.region,
            cause: NoCredentialCause::StoreEmpty,
        })
}

pub async fn video_models(
    state: &GatewayState,
    key: &ApiKeyRecord,
) -> Result<Vec<VideoModel>, GatewayError> {
    ensure_supported_key(key)?;
    let account = select_account(key, None)?;
    VideoProvider::fetch_video_models(state, key.region, &account).await
}

pub async fn submit_video(
    state: &GatewayState,
    store: &VideoTaskStore,
    key: &ApiKeyRecord,
    input: &VideoSubmitInput,
) -> Result<(String, Value), GatewayError> {
    ensure_supported_key(key)?;
    let account_value = select_account(key, None)?;
    let models = VideoProvider::fetch_video_models(state, key.region, &account_value).await?;
    let payload = input.payload(&models)?;
    let upstream_payload =
        read_response(VideoProvider::submit(state, key.region, &account_value, &payload).await)
            .await?;
    let upstream_task_id = extract_upstream_task_id(&upstream_payload)?
        .ok_or_else(|| GatewayError::Internal("上游视频响应缺少任务 id".into()))?;
    let now = buddy_switch_core::modules::config::now_ms();
    let id = format!("vid_{}", uuid::Uuid::new_v4().simple());
    let task = VideoTask {
        id: id.clone(),
        region: key.region.as_str().into(),
        key_id: key.id.clone(),
        upstream_task_id: Some(upstream_task_id),
        uid: account::get_str(&account_value, "uid").unwrap_or_default(),
        account: account::get_str(&account_value, "nickname")
            .or_else(|| account::get_str(&account_value, "email"))
            .unwrap_or_default(),
        model: payload["model"].as_str().unwrap_or_default().into(),
        status: classify_upstream_status(&upstream_payload)?,
        upstream_payload,
        created_at_ms: now,
        updated_at_ms: now,
    };
    let value = task.to_value();
    store.insert(task).await;
    Ok((id, value))
}

async fn query_video_using<F, Fut>(
    store: &VideoTaskStore,
    key: &ApiKeyRecord,
    id: &str,
    query: F,
) -> Result<Option<Value>, GatewayError>
where
    F: FnOnce(VideoTask) -> Fut,
    Fut: Future<Output = Result<Value, GatewayError>>,
{
    ensure_supported_key(key)?;
    let Some(mut task) = store.get(id).await else {
        return Ok(None);
    };
    if task.region != key.region.as_str() || task.key_id != key.id {
        return Ok(None);
    }
    if task.status.is_terminal() || task.upstream_task_id.is_none() {
        return Ok(Some(task.to_value()));
    }
    let value = query(task.clone()).await?;
    let body = response_body(&value)?;
    let node = body.get("task").unwrap_or(&body);
    if !node
        .get("status")
        .or_else(|| node.get("state"))
        .and_then(Value::as_str)
        .is_some_and(|status| !status.trim().is_empty())
    {
        return Err(GatewayError::Internal(
            "上游视频查询响应缺少任务状态".into(),
        ));
    }
    let status = classify_upstream_status(&value)?;
    task.upstream_payload = value;
    task.status = status;
    task.updated_at_ms = buddy_switch_core::modules::config::now_ms();
    store.update(task).await;
    Ok(store.get(id).await.map(|task| task.to_value()))
}

pub async fn query_video(
    state: &GatewayState,
    store: &VideoTaskStore,
    key: &ApiKeyRecord,
    id: &str,
) -> Result<Option<Value>, GatewayError> {
    query_video_using(store, key, id, |task| async move {
        let account_value = select_account(key, Some(&task.uid))?;
        read_response(
            VideoProvider::query(
                state,
                key.region,
                &account_value,
                task.upstream_task_id.as_deref().unwrap_or_default(),
            )
            .await,
        )
        .await
    })
    .await
}

pub fn extract_upstream_task_id(value: &Value) -> Result<Option<String>, GatewayError> {
    let body = response_body(value)?;
    let node = body.get("task").unwrap_or(&body);
    Ok(["task_id", "id"].into_iter().find_map(|key| {
        node.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
    }))
}

pub fn classify_upstream_status(value: &Value) -> Result<VideoTaskStatus, GatewayError> {
    let body = response_body(value)?;
    let node = body.get("task").unwrap_or(&body);
    let state_text = node
        .get("status")
        .or_else(|| node.get("state"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    Ok(match state_text.as_str() {
        "succeed" | "success" | "succeeded" | "completed" | "complete" | "done" => {
            VideoTaskStatus::Completed
        }
        "fail" | "failed" | "error" => VideoTaskStatus::Failed(
            node.get("msg")
                .or_else(|| node.get("message"))
                .or_else(|| node.get("error"))
                .and_then(Value::as_str)
                .unwrap_or("上游任务失败")
                .into(),
        ),
        "pending" | "queued" | "waiting" | "" => VideoTaskStatus::Pending,
        _ => VideoTaskStatus::Generating,
    })
}

pub fn new_store() -> VideoTaskStore {
    VideoTaskStore::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::models::fixture;

    fn key(region: &str, id: &str) -> ApiKeyRecord {
        serde_json::from_value(json!({"id":id,"name":"fixture", "region":region,
            "prefix":"fixture", "hash":"fixture", "created_at":0,"revoked_at":null}))
        .unwrap()
    }

    fn task(status: VideoTaskStatus) -> VideoTask {
        VideoTask {
            id: "vid_fixture".into(),
            key_id: "key1".into(),
            region: "cn".into(),
            upstream_task_id: Some("upstream-fixture".into()),
            uid: "u1".into(),
            account: "fixture".into(),
            model: "kling-video-v3".into(),
            status,
            upstream_payload: json!({"status":"queued"}),
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    #[test]
    fn directory_defaults_and_nested_parameters_match_protocol() {
        let models = fixture("cn");
        let input =
            VideoSubmitInput::parse(&json!({"model":"KLING-VIDEO-V3","prompt":"test"})).unwrap();
        let payload = input.payload(&models).unwrap();
        assert_eq!(
            payload,
            json!({"model":"kling-video-v3","prompt":"test","seconds":5,"watermark":false,
            "extra_parameters":{"resolution":"720P","enable_audio":true,"aspect_ratio":"16:9"}})
        );
        let input =
            VideoSubmitInput::parse(&json!({"model":"minimax-video-h3", "prompt":"test"})).unwrap();
        let payload = input.payload(&models).unwrap();
        assert_eq!(
            payload["extra_parameters"],
            json!({"resolution":"768P","enable_audio":true,"aspect_ratio":"adaptive"})
        );
        assert!(input.payload(&fixture("global")).is_err());
        let input = VideoSubmitInput::parse(&json!({"model":"kling-video-v3","prompt":"test",
            "extra_parameters":{"resolution":"4K","enable_audio":false}, "negative_prompt":"blur", "watermark":true})).unwrap();
        let payload = input.payload(&models).unwrap();
        assert_eq!(payload["extra_parameters"]["resolution"], "4K");
        assert_eq!(payload["extra_parameters"]["enable_audio"], false);
        assert_eq!(payload["negative_prompt"], "blur");
        assert_eq!(payload["watermark"], true);
    }

    #[test]
    fn invalid_parameters_fail_before_generation() {
        let models = fixture("cn");
        for (field, value) in [
            ("seconds", json!(2)),
            ("seconds", json!(16)),
            ("seconds", json!("5")),
            ("seconds", json!(5.5)),
            ("resolution", json!("480p")),
            ("resolution", json!(42)),
            ("aspect_ratio", json!("4:3")),
            ("enable_audio", json!("false")),
            ("watermark", json!("false")),
            ("image_url", json!("http://example.com/first.png")),
            ("image_url", json!("https://")),
            ("image_url", json!("")),
            ("prompt", json!("x".repeat(2501))),
            (
                "extra_parameters",
                json!({"resolution":"4K", "unknown":true}),
            ),
        ] {
            let mut body = json!({"model":"kling-video-v3","prompt":"test"});
            body[field] = value;
            assert!(
                VideoSubmitInput::parse(&body)
                    .and_then(|input| input.payload(&models))
                    .is_err(),
                "{body}"
            );
        }
        let input = VideoSubmitInput::parse(
            &json!({"model":"minimax-video-h3","prompt":"test","enable_audio":false}),
        )
        .unwrap();
        assert!(input.payload(&models).is_err());
        assert!(VideoSubmitInput::parse(
            &json!({"model":"kling-video-v3","prompt":"x", "resolution":"720P",
            "extra_parameters":{"resolution":"4K"}})
        )
        .is_err());
        for body in [
            json!({}),
            json!({"model":"kling-video-v3"}),
            json!({"model":"x","prompt":" "}),
        ] {
            assert!(VideoSubmitInput::parse(&body).is_err());
        }
    }

    #[test]
    fn image_input_omits_ratio_default_and_preserves_explicit_ratio() {
        let mut body = json!({"model":"kling-video-v3","prompt":"test","image_url":"https://example.com/first.png"});
        let payload = VideoSubmitInput::parse(&body)
            .unwrap()
            .payload(&fixture("cn"))
            .unwrap();
        assert!(payload["extra_parameters"].get("aspect_ratio").is_none());
        assert_eq!(payload["image_url"], body["image_url"]);
        body["aspect_ratio"] = json!("1:1");
        assert_eq!(
            VideoSubmitInput::parse(&body)
                .unwrap()
                .payload(&fixture("cn"))
                .unwrap()["extra_parameters"]["aspect_ratio"],
            "1:1"
        );
    }

    #[test]
    fn nested_task_bodies_and_business_errors() {
        let value = json!({"code":0,"data":{"body":{"task_id":"up1","status":"completed"}}});
        assert_eq!(
            extract_upstream_task_id(&value).unwrap().as_deref(),
            Some("up1")
        );
        assert_eq!(
            classify_upstream_status(&value).unwrap(),
            VideoTaskStatus::Completed
        );
        let error =
            json!({"code":42,"data":{"body":{"task_id":"not-created","status":"completed"}}});
        assert!(extract_upstream_task_id(&error).is_err());
        assert!(classify_upstream_status(&error).is_err());
    }

    #[tokio::test]
    async fn task_reads_are_isolated_by_region_and_key() {
        let store = new_store();
        store.insert(task(VideoTaskStatus::Generating)).await;
        for key in [key("global", "key1"), key("cn", "other-key")] {
            assert!(query_video_using(&store, &key, "vid_fixture", |_| async {
                panic!("不能访问上游")
            })
            .await
            .unwrap()
            .is_none());
        }
    }

    #[tokio::test]
    async fn terminal_queries_never_call_upstream() {
        for status in [
            VideoTaskStatus::Completed,
            VideoTaskStatus::Failed("fixture".into()),
        ] {
            let store = new_store();
            store.insert(task(status.clone())).await;
            let result = query_video_using(&store, &key("cn", "key1"), "vid_fixture", |_| async {
                panic!("终态不能重查")
            })
            .await
            .unwrap()
            .unwrap();
            assert_eq!(result["status"], status.as_str());
        }
    }

    #[tokio::test]
    async fn transient_query_failure_keeps_original_state_and_payload() {
        let store = new_store();
        let original = task(VideoTaskStatus::Generating);
        store.insert(original.clone()).await;
        let result = query_video_using(&store, &key("cn", "key1"), "vid_fixture", |_| async {
            read_response(
                buddy_switch_core::modules::upstream::UpstreamChatResult::Err {
                    status: 503,
                    kind: buddy_switch_core::modules::upstream::UpstreamErrorKind::Server,
                    message: "temporary".into(),
                },
            )
            .await
        })
        .await;
        assert!(result.is_err());
        let result = store.get("vid_fixture").await.unwrap();
        assert_eq!(result.status, original.status);
        assert_eq!(result.updated_at_ms, original.updated_at_ms);
        assert_eq!(result.upstream_payload, original.upstream_payload);
        let result = query_video_using(&store, &key("cn", "key1"), "vid_fixture", |_| async {
            Ok(json!({"code":12,"data":{"status":"completed"}}))
        })
        .await;
        assert!(result.is_err());
        assert_eq!(
            store.get("vid_fixture").await.unwrap().status,
            VideoTaskStatus::Generating
        );
        let result = query_video_using(&store, &key("cn", "key1"), "vid_fixture", |_| async {
            Ok(json!({"code":0,"data":{}}))
        })
        .await;
        assert!(result.is_err());
        assert_eq!(
            store.get("vid_fixture").await.unwrap().status,
            VideoTaskStatus::Generating
        );
    }

    #[tokio::test]
    async fn successful_query_updates_store_but_cannot_overwrite_terminal_state() {
        let store = new_store();
        store.insert(task(VideoTaskStatus::Pending)).await;
        let result = query_video_using(&store, &key("cn", "key1"), "vid_fixture", |_| async {
            Ok(json!({"code":0,"data":{"body":{"status":"completed"}}}))
        })
        .await
        .unwrap()
        .unwrap();
        assert_eq!(result["status"], "completed");
        assert!(!store.update(task(VideoTaskStatus::Generating)).await);
        assert_eq!(
            store.get("vid_fixture").await.unwrap().status,
            VideoTaskStatus::Completed
        );
    }

    #[test]
    fn account_filters_require_matching_region_domain() {
        for (region, domain, expected) in [
            ("cn", Some("www.codebuddy.cn"), true),
            ("cn", Some("www.workbuddy.ai"), false),
            ("global", Some("www.codebuddy.cn"), false),
            ("global", Some("www.workbuddy.ai"), true),
            ("cn", None, true),
            ("global", None, false),
        ] {
            let mut account = json!({"uid":"u1","access_token":"fixture"});
            if let Some(domain) = domain {
                account["domain"] = json!(domain);
            }
            assert_eq!(
                usable_account(&account, &key(region, "key1")),
                expected,
                "region={region}, domain={domain:?}"
            );
        }
    }

    #[test]
    fn account_filters_honor_scope_deactivation_and_credentials() {
        let mut key = key("cn", "key1");
        let account = json!({"uid":"u1","access_token":"fixture","groupName":"group1"});
        assert!(usable_account(&account, &key));
        for (field, value) in [
            ("deactivated", json!(true)),
            ("access_token", json!("")),
            ("access_token", json!({"encrypted":"fixture"})),
            ("uid", json!("")),
        ] {
            let mut account = account.clone();
            account[field] = value;
            assert!(!usable_account(&account, &key));
        }
        key.scope_mode = crate::apikey::ScopeMode::Selected;
        assert!(!usable_account(&account, &key));
        key.scope_groups.push("group1".into());
        assert!(usable_account(&account, &key));
        key.scope_groups.clear();
        key.scope_accounts.push("u1".into());
        assert!(usable_account(&account, &key));
        key.quota_mode = QuotaMode::Limited;
        assert!(ensure_supported_key(&key).is_err());
    }
}
