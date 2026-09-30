//! 成长任务判据事件构造器（对照参考实现 `internal/upstream/desktop.go` / `school.go` / `report.go`）。
//!
//! 全部为**纯函数**：给定 uid / 时间戳 / 会话标识即可生成事件 JSON，不依赖网络。
//! 事件形状逐字段对齐 Go 参考实现——上游对字段形状敏感（缺 `userId` 会被静默丢弃），
//! 因此不在此做「简化」，单测按 Go 源码钉住关键字段。
//!
//! 指纹：
//! - **桌面指纹**（`workbuddy-desktop`）：`copilot.tencent.com/v2/report` 数组上报
//! - **mp 指纹**（`workbuddy-mp`）：`www.codebuddy.cn/v2/report` 数组上报（小程序口径任务）
//! - **chat_request_send**：billing 域 `/v2/report` 单元素数组（`userId` 必填）

use serde_json::{json, Value};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// 稳定派生标识（对照 Go `deriveID`：sha256(salt:uid) 前 18 字节 hex，共 36 字符）
// ---------------------------------------------------------------------------

/// 由 uid + salt 稳定派生 36 位 hex 设备标识（幂等，模拟固定设备）。
pub fn derive_id(uid: &str, salt: &str) -> String {
    use sha2::Digest;
    let sum = sha2::Sha256::digest(format!("{salt}:{uid}").as_bytes());
    sum[..18].iter().map(|b| format!("{b:02x}")).collect()
}

// ---------------------------------------------------------------------------
// 桌面指纹（desktop.go desktopFingerprint）
// ---------------------------------------------------------------------------

/// 桌面端公共指纹字段（注入每个事件；业务字段可覆盖同名键）。
pub fn desktop_fingerprint(uid: &str, nickname: &str, now_ms: i64) -> HashMap<String, Value> {
    let mut m: HashMap<String, Value> = HashMap::new();
    m.insert("timezone".into(), json!("Asia/Shanghai"));
    m.insert("reportDelay".into(), json!(2000));
    m.insert("userId".into(), json!(uid));
    m.insert("username".into(), json!(nickname));
    m.insert("userNickname".into(), json!(nickname));
    m.insert("product".into(), json!("SaaS"));
    m.insert("releaseDate".into(), json!(1789036585355i64));
    m.insert("commit".into(), json!("5f9692923c93033111c51ad7b003eb80204a9b75"));
    m.insert("ideName".into(), json!("WorkBuddy"));
    m.insert("ideType".into(), json!("WorkBuddy"));
    m.insert("ideVersion".into(), json!("5.5.6"));
    m.insert("machineId".into(), json!(derive_id(uid, "machine")));
    m.insert("sessionId".into(), json!(derive_id(uid, "session")));
    m.insert("extName".into(), json!("workbuddy-desktop"));
    m.insert("extVersion".into(), json!("5.5.6"));
    m.insert("os".into(), json!("win32"));
    m.insert("arch".into(), json!("x64"));
    m.insert("osVersion".into(), json!("10.0.26220"));
    m.insert("cpuCores".into(), json!(20));
    m.insert("memorySize".into(), json!(24));
    m.insert("timestamp".into(), json!(now_ms));
    m.insert("presentAt".into(), json!(now_ms));
    m
}

/// 把事件业务字段叠加到桌面指纹上（业务字段优先），生成单个上报事件。
pub fn with_desktop_fp(uid: &str, nickname: &str, now_ms: i64, code: &str, extra: Value) -> Value {
    let base = desktop_fingerprint(uid, nickname, now_ms);
    let mut m: serde_json::Map<String, Value> = base.into_iter().collect();
    if let Value::Object(mut biz) = extra {
        biz.insert("eventCode".into(), json!(code));
        m.extend(biz);
    }
    Value::Object(m)
}

/// 批量桌面事件上报数组（`/v2/report` 的 body 形状 = 事件数组）。
pub fn desktop_events_array(
    uid: &str,
    nickname: &str,
    now_ms: i64,
    events: &[(String, Value)],
) -> Value {
    json!(events
        .iter()
        .map(|(code, extra)| with_desktop_fp(uid, nickname, now_ms, code, extra.clone()))
        .collect::<Vec<Value>>())
}

// ---------------------------------------------------------------------------
// 桌面端对话事件链（desktop.go DesktopChatSequence）
// ---------------------------------------------------------------------------

/// 一次「桌面端成功对话」完整事件链（6 事件）。
pub fn desktop_chat_sequence(
    conv: &str,
    req: &str,
    msg: &str,
    model_id: &str,
    model_name: &str,
) -> Vec<(String, Value)> {
    let assistant = format!("{msg}-assistant");
    let uuid = Value::String(req.to_string());
    vec![
        (
            "agent_task_created".into(),
            json!({
                "source": "LOCAL", "name": "working", "task_target": "local", "mode": "craft",
                "requestModelId": model_id, "requestModelName": model_name,
                "has_repo": false, "repo_type": "none", "workspace_type": "empty",
                "has_connector": false, "connector_types": [],
                "has_mention": false, "mention_types": [],
                "has_template": false, "action": "", "template_name": "",
                "has_expert": false, "expert_id": "", "expert_name": "", "expert_industry_id": "",
                "has_skill": false, "skill_names": [],
                "conversationId": conv, "messageId": msg,
                "buddyId": "", "buddyName": ""
            }),
        ),
        (
            "chat_message_send".into(),
            json!({
                "messageId": assistant, "historyCount": 0,
                "isContextTruncated": false, "currentStepCount": 1,
                "traceId": uuid, "rootRequestId": req,
                "parentConversationId": conv,
                "agentName": "cli", "agentType": "main"
            }),
        ),
        (
            "chat_request_send".into(),
            json!({
                "inputLength": 24, "isPlan": false, "isAutoExecuteTerminal": false,
                "isAutoModify": false, "codebaseEnable": false, "maxToken": 0,
                "maxSteps": 500, "temperature": 0, "maxRetries": 0,
                "mentionContexts": [], "knowledgeId": [], "knowledgeName": [],
                "codebaseId": "", "mentionContextCount": 0, "command": "",
                "recommendId": "", "skillId": "", "skillCount": 0, "totalCount": 0,
                "traceId": uuid, "rootRequestId": req,
                "parentConversationId": conv,
                "agentName": "cli", "agentType": "main",
                "codebuddy.session_id": conv,
                "codebuddy.conversation_request_id": req
            }),
        ),
        (
            "chat_message_response".into(),
            json!({
                "messageId": assistant, "responseModelId": model_id,
                "inputToken": 120, "outputToken": 80, "totalToken": 200,
                "cachedTokens": 0, "cachedWriteTokens": 0, "cachedMissTokens": 0,
                "isSuccessful": true, "messageErrorCode": "", "finishReason": "stop",
                "traceId": uuid,
                "conversationId": conv,
                "rootRequestId": req, "parentConversationId": conv,
                "agentName": "cli", "agentType": "main",
                "codebuddy.session_id": conv,
                "codebuddy.conversation_request_id": req
            }),
        ),
        (
            "chat_message_status".into(),
            json!({
                "messageId": assistant, "messageErrorCode": "0",
                "traceId": uuid, "rootRequestId": req,
                "parentConversationId": conv,
                "agentName": "cli", "agentType": "main"
            }),
        ),
        (
            "chat_request_response".into(),
            json!({
                "mode": "craft", "toolCallCount": 0,
                "inputToken": 120, "outputToken": 80, "totalToken": 200,
                "cachedTokens": 0, "cachedWriteTokens": 0, "cachedMissTokens": 0,
                "isSuccessful": true, "messageErrorCode": "", "finishReason": "stop",
                "rootRequestId": req, "parentConversationId": conv
            }),
        ),
    ]
}

// ---------------------------------------------------------------------------
// Buddy 应用五连（desktop.go DesktopBuddyAppSequence）
// ---------------------------------------------------------------------------

/// 「进入 Buddy 应用」五连事件（Buddy_App / Buddy_App_QQ 共用载体）。
pub fn buddy_app_sequence(buddy_id: &str, buddy_name: &str) -> Vec<(String, Value)> {
    let base = |code: &str, extra: Value| -> (String, Value) {
        let mut m = extra;
        if let Value::Object(map) = &mut m {
            map.insert("mode".into(), json!("LOCAL"));
            map.insert("buddyId".into(), json!(buddy_id));
            map.insert("buddyName".into(), json!(buddy_name));
        }
        (code.to_string(), m)
    };
    vec![
        base("buddyapp_discover_click", json!({})),
        base(
            "buddyapp_show",
            json!({"elementId": buddy_id, "elementName": buddy_name, "position": 2}),
        ),
        base(
            "buddyapp_enter_click",
            json!({"elementId": buddy_id, "elementName": buddy_name, "position": 2, "isFirstPage": "1"}),
        ),
        base(
            "buddyapp_auth_confirm_click",
            json!({"elementId": buddy_id, "elementName": buddy_name}),
        ),
        base(
            "buddyapp_bindaccount_skip_click",
            json!({"elementId": buddy_id, "elementName": buddy_name}),
        ),
    ]
}

// ---------------------------------------------------------------------------
// 定时任务创建（desktop.go DesktopAutomationCreateEvent）
// ---------------------------------------------------------------------------

pub fn automation_create_event(name: &str) -> (String, Value) {
    (
        "automated_task_create_suc".into(),
        json!({
            "name": name,
            "source": "manually", "modelId": "fast-model", "modelIsThinking": true,
            "connectorCount": 0, "skills": "", "skillCount": 0,
            "scheduleType": "once", "mode": "LOCAL"
        }),
    )
}

// ---------------------------------------------------------------------------
// 资料库阅读（Library_read：web 域事件）
// ---------------------------------------------------------------------------

/// 资料库介绍阅读事件（Library_read 判据）。web 域 `/v2/report` 单事件。
pub fn library_read_event(uid: &str, nickname: &str, enterprise_id: &str, now_ms: i64) -> Value {
    let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36";
    let page_url = "https://www.workbuddy.cn/space/d/o0KWYeynteVv06UnAZqIFm";
    json!({
        "eventCode": "web_element_click",
        "timestamp": now_ms,
        "reportDelay": 0,
        "pageURL": page_url,
        "elementId": "library_doc_intro_click",
        "elementName": "WorkBuddy资料库介绍",
        "os": "Win32",
        "arch": "",
        "osVersion": "10.0",
        "userAgent": ua,
        "machineId": derive_id(uid, "webmachine"),
        "userId": uid,
        "userNickname": nickname,
        "enterpriseId": enterprise_id
    })
}

// ---------------------------------------------------------------------------
// 模板 ×5（desktop.go DesktopTemplateUseSequence）
// ---------------------------------------------------------------------------

/// 一组「使用模板创建任务」事件（对话链 + agent_task_created_with_template + template_used）。
pub fn template_use_sequence(conv: &str, req: &str, template_id: &str, template_name: &str) -> Vec<(String, Value)> {
    let mut events = desktop_chat_sequence(conv, req, &format!("msg-{template_id}"), "fast-model", "fast-model");
    events.push((
        "agent_task_created_with_template".into(),
        json!({
            "mode": "working", "isCustomModel": false,
            "id": template_id, "name": template_name, "requestId": req
        }),
    ));
    events.push((
        "template_used".into(),
        json!({"template_id": template_id, "task_mode": "working"}),
    ));
    events
}

/// 默认 5 组模板（panel runTemplateUse 同口径；template_id 服务端不校验真实性）。
pub const TEMPLATE_GROUPS: [&str; 5] = ["1", "2", "3", "4", "5"];
pub const TEMPLATE_NAMES: [&str; 5] = ["深度研究", "周报生成", "竞品分析", "活动策划", "代码评审"];

// ---------------------------------------------------------------------------
// 灵感案例（desktop.go DesktopPlaybookPromptSequence / MiniPlaybookEvents）
// ---------------------------------------------------------------------------

/// PC 域「灵感案例做同款」事件组（playbook_prompt 判据）。
pub fn playbook_prompt_sequence(conv: &str, req: &str, case_id: &str, case_name: &str) -> Vec<(String, Value)> {
    let mut events = desktop_chat_sequence(conv, req, "msg-pb", "fast-model", "fast-model");
    let payload = json!({
        "id": case_id, "name": case_name, "type": "document",
        "categoryId": "", "categoryName": ""
    });
    events.push((
        "web_element_click".into(),
        json!({
            "pageName": "playbook_detail",
            "elementId": "playbook_ctaClick", "elementName": case_name, "source": "discover"
        }),
    ));
    let mut cta = json!({"eventCode": "playbook_cta_click", "source": "discover", "position": 0});
    cta.as_object_mut().unwrap().extend(payload.as_object().unwrap().clone());
    events.push(("playbook_cta_click".into(), cta));
    let mut send = json!({
        "eventCode": "playbook_prompt_send",
        "conversationId": conv, "requestId": req
    });
    send.as_object_mut().unwrap().extend(payload.as_object().unwrap().clone());
    events.push(("playbook_prompt_send".into(), send));
    events
}

/// mp 域灵感事件组（Sequential_Tasks_7 fallback 形态）。
pub fn mini_playbook_events(case_id: &str, case_name: &str) -> Vec<Value> {
    let base = json!({
        "id": case_id, "name": case_name, "type": "document",
        "categoryId": "", "categoryName": "",
        "skills": "", "skillNames": ""
    });
    let mut cta = json!({"eventCode": "playbook_cta_click", "source": "discover", "position": 1, "extVersion": "2.2.8"});
    cta.as_object_mut().unwrap().extend(base.as_object().unwrap().clone());
    let mut send = json!({
        "eventCode": "playbook_prompt_send", "source": "discover",
        "promptLength": 96, "isOfficial": 1,
        "conversationId": format!("buddyswitch-mp-pb-{}", uuid::Uuid::new_v4().simple()),
        "extVersion": "2.2.8"
    });
    send.as_object_mut().unwrap().extend(base.as_object().unwrap().clone());
    vec![cta, send]
}

// ---------------------------------------------------------------------------
// 设计创意画布（desktop.go DesktopDesignCanvasSequence）
// ---------------------------------------------------------------------------

pub fn design_canvas_sequence(conv: &str, req: &str) -> Vec<(String, Value)> {
    let mut events = desktop_chat_sequence(conv, req, "msg-canvas", "fast-model", "fast-model");
    let canvas_id = format!("ardot-file-{}", req_suffix8(req));
    events.push((
        "wbx_design_canvas_task_create".into(),
        json!({
            "conversationId": conv, "requestId": req, "source": "summon_keyword",
            "cost": 12000, "isSuccessful": true
        }),
    ));
    events.push((
        "wbx_design_canvas_open".into(),
        json!({
            "conversationId": conv, "requestId": req,
            "id": canvas_id,
            "source": "summon_keyword", "type": "page",
            "cost": 13000, "isSuccessful": true
        }),
    ));
    events
}

/// 取字符串尾 8 字（不足则全取）。
fn req_suffix8(s: &str) -> &str {
    &s[s.len().saturating_sub(8)..]
}

// ---------------------------------------------------------------------------
// 专家市场（desktop.go MarketExpert / DesktopExpertSummonSequence / actual_use）
// ---------------------------------------------------------------------------

/// 专家市场条目的本地视图（/portal/operation-platform/market/expert/list 响应子集）。
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketExpert {
    pub expert_id: String,
    #[serde(default)]
    pub expert_type: String,
    #[serde(default)]
    pub display_name_zh: String,
    #[serde(default)]
    pub profession_zh: String,
    #[serde(default)]
    pub version: String,
}

impl MarketExpert {
    /// 从上游 JSON 数组解析（字段名 snake_case，对照 Go struct tag）。
    pub fn from_json(list: &Value) -> Vec<MarketExpert> {
        list.get("experts")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| serde_json::from_value::<MarketExpertC>(e.clone()).ok())
                    .map(|c| MarketExpert {
                        expert_id: c.expert_id,
                        expert_type: c.expert_type,
                        display_name_zh: c.display_name_zh,
                        profession_zh: c.profession_zh,
                        version: c.version,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 专家显示名（中文名缺失回落职业名，再回落 id）。
    pub fn display(&self) -> &str {
        if !self.display_name_zh.is_empty() {
            &self.display_name_zh
        } else if !self.profession_zh.is_empty() {
            &self.profession_zh
        } else {
            &self.expert_id
        }
    }

    fn category(&self) -> String {
        "expert-all".to_string()
    }

    fn version_or_default(&self) -> String {
        if self.version.is_empty() {
            "1.0.0".to_string()
        } else {
            self.version.clone()
        }
    }
}

/// 上游原始字段形状（snake_case）。
#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct MarketExpertC {
    expert_id: String,
    #[serde(default)]
    expert_type: String,
    #[serde(default)]
    display_name_zh: String,
    #[serde(default)]
    profession_zh: String,
    #[serde(default)]
    version: String,
}

/// 专家召唤链（3 事件，web 域 /v2/report；载荷对齐真实抓包样本）。
pub fn expert_summon_sequence(e: &MarketExpert) -> Vec<(String, Value)> {
    let cat = e.category();
    let ver = e.version_or_default();
    vec![
        (
            "web_element_click".into(),
            json!({
                "source": e.expert_id, "type": cat, "version": ver,
                "elementId": "expert_summon_click", "elementName": "立即召唤",
                "pageURL": "/C:/Program%20Files/WorkBuddy/resources/app.asar/renderer/index.html"
            }),
        ),
        (
            "expert_summon_click".into(),
            json!({
                "id": e.expert_id, "name": e.display_name_zh,
                "expertTitle": e.profession_zh, "type": "expert-all", "position": 0,
                "expertType": e.expert_type, "version": ver, "mode": "LOCAL"
            }),
        ),
        (
            "expert_summoned".into(),
            json!({
                "id": e.expert_id, "name": e.display_name_zh,
                "expertTitle": e.profession_zh, "type": "expert-all"
            }),
        ),
    ]
}

fn expert_actual_use(e: &MarketExpert, conv: &str, req: &str, mode: &str) -> (String, Value) {
    let cat = e.category();
    let ver = e.version_or_default();
    (
        "expert_actual_use".into(),
        json!({
            "eventCode": "expert_actual_use",
            "id": e.expert_id, "name": e.display_name_zh, "expertTitle": e.profession_zh,
            "type": cat, "expertType": e.expert_type, "source": "builtin", "version": ver,
            "cost": 9000, "characterCount": 14,
            "conversationId": conv, "requestId": req, "messageId": "msg-",
            "requestModelId": "fast-model", "requestModelName": "fast-model",
            "mode": mode
        }),
    )
}

/// expert_5 / Expert_team_use_3：expert_actual_use（mode=craft）。
pub fn expert_actual_use_event(e: &MarketExpert, conv: &str, req: &str) -> (String, Value) {
    expert_actual_use(e, conv, req, "craft")
}

/// Expert_lighthouse：expert_actual_use（mode=LOCAL，type 空、cost 0，对齐真实样本）。
pub fn expert_actual_use_local(e: &MarketExpert, conv: &str, req: &str) -> (String, Value) {
    let (_code, mut v) = expert_actual_use(e, conv, req, "LOCAL");
    if let Value::Object(map) = &mut v {
        map.insert("type".into(), json!(""));
        map.insert("cost".into(), json!(0));
    }
    ("expert_actual_use".into(), v)
}

/// skill_1：skill_info 事件（桌面指纹，JOIN 真实会话 id）。
pub fn skill_info_event(
    conv: &str,
    req: &str,
    msg_id: &str,
    skill_name: &str,
    skill_id: &str,
    skill_version: &str,
) -> (String, Value) {
    (
        "skill_info".into(),
        json!({
            "id": skill_name, "skillId": skill_id, "skillVersion": skill_version,
            "toolStatus": "success", "fileCount": 56, "source": "workbuddy-desktop",
            "conversationId": conv, "requestId": req, "messageId": msg_id,
            "requestModelId": "fast-model", "requestModelName": "fast-model",
            "traceId": req
        }),
    )
}

// ---------------------------------------------------------------------------
// 外观主题（Hp_Appearance）
// ---------------------------------------------------------------------------

/// 皮肤生效事件（appearance_skin_apply；需先调 appearance/set API 留痕）。
pub fn appearance_skin_apply_event(resource_key: &str) -> (String, Value) {
    (
        "appearance_skin_apply".into(),
        json!({
            "action": "apply", "source": "settings_close",
            "id": resource_key, "vipLevel": 0, "series": "", "type": "unknown"
        }),
    )
}

/// 外观主题设置 API body（POST /v2/user-asset/appearance/set）。
pub fn appearance_set_body(resource_key: &str) -> Value {
    json!({"kind": "theme", "resource_key": resource_key})
}

// ---------------------------------------------------------------------------
// mp 指纹（school.go mpEventBase + 事件）
// ---------------------------------------------------------------------------

/// mp 公共指纹 base。
pub fn mp_event_base(uid: &str, nickname: &str, now_ms: i64) -> HashMap<String, Value> {
    let mut m: HashMap<String, Value> = HashMap::new();
    m.insert("timestamp".into(), json!(now_ms));
    m.insert("ideType".into(), json!("WorkBuddy_MP"));
    m.insert("ideVersion".into(), json!("2.4.0"));
    m.insert("extName".into(), json!("workbuddy-mp"));
    m.insert("extVersion".into(), json!("2.4.0"));
    m.insert("product".into(), json!("SaaS"));
    m.insert("ideName".into(), json!("wx_app_cloud"));
    m.insert("platform".into(), json!("mini_program"));
    m.insert("os".into(), json!("windows"));
    m.insert("osVersion".into(), json!("11"));
    m.insert("arch".into(), json!("x64"));
    m.insert("machineId".into(), json!("0655736a-607f-4d9d-b430-58176ee9a090"));
    m.insert("timezone".into(), json!("Asia/Shanghai"));
    m.insert("userId".into(), json!(uid));
    m.insert("userNickname".into(), json!(nickname));
    m
}

/// 把业务字段叠加到 mp base 上（业务字段优先）。
pub fn with_mp_fp(uid: &str, nickname: &str, now_ms: i64, ev: &Value) -> Value {
    let base = mp_event_base(uid, nickname, now_ms);
    let mut m: serde_json::Map<String, Value> = base.into_iter().collect();
    if let Value::Object(biz) = ev {
        for (k, v) in biz {
            m.insert(k.clone(), v.clone());
        }
    }
    Value::Object(m)
}

/// mp 批量上报数组。
pub fn mp_events_array(uid: &str, nickname: &str, now_ms: i64, events: &[Value]) -> Value {
    json!(events
        .iter()
        .map(|ev| with_mp_fp(uid, nickname, now_ms, ev))
        .collect::<Vec<Value>>())
}

/// 一条 mp chat_request_send（school_chat_times / sequential 补报）。
pub fn mp_chat_event(conv: &str, rid: &str) -> Value {
    json!({
        "eventCode": "chat_request_send",
        "inputLength": 14, "isPlan": false, "isAutoExecuteTerminal": false,
        "isAutoModify": false, "codebaseEnable": false, "maxToken": 0,
        "maxSteps": 500, "temperature": 0, "maxRetries": 0,
        "mentionContexts": [], "knowledgeId": [], "knowledgeName": [],
        "codebaseId": "", "mentionContextCount": 0, "command": "",
        "recommendId": "", "skillId": "", "skillCount": 0, "totalCount": 0,
        "traceId": rid, "rootRequestId": rid,
        "parentConversationId": conv, "conversationId": conv,
        "messageId": format!("msg-{}", req_suffix8(rid)),
        "agentName": "mp", "agentType": "main",
        "codebuddy.session_id": conv,
        "codebuddy.conversation_request_id": rid
    })
}

/// school_season「校园日」判据事件（mp chat + activityId）。
pub const SCHOOL_OPEN_DAY_ACTIVITY_ID: &str = "school_open_day_2026";

pub fn school_season_chat_event(conv: &str) -> Value {
    let rid = format!("wb2api-{}", uuid::Uuid::new_v4().simple());
    let mut ev = mp_chat_event(conv, &rid);
    if let Value::Object(map) = &mut ev {
        map.insert("activityId".into(), json!(SCHOOL_OPEN_DAY_ACTIVITY_ID));
    }
    ev
}

/// 裸 mp 对话（Sequential_Tasks_1/3/6 判据：不带 activityId）。
pub fn school_chat_times_event(conv: &str) -> Value {
    let rid = format!("wb2api-{}", uuid::Uuid::new_v4().simple());
    mp_chat_event(conv, &rid)
}

/// mp 对话 + 模型字段（Sequential_Tasks_5「使用 GLM5.2」判据）。
pub fn mini_chat_model_event(conv: &str, model_id: &str, model_name: &str) -> Value {
    let rid = format!("wb2api-{}", uuid::Uuid::new_v4().simple());
    let mut ev = mp_chat_event(conv, &rid);
    if let Value::Object(map) = &mut ev {
        map.insert("requestModelId".into(), json!(model_id));
        map.insert("requestModelName".into(), json!(model_name));
    }
    ev
}

/// mp 域专家使用事件（Sequential_Tasks_2 判据；不带 conversationId/activityId）。
pub fn mini_expert_use_event(expert_id: &str, expert_name: &str, expert_type: &str) -> Value {
    let expert_type = if expert_type.is_empty() { "agent" } else { expert_type };
    let name = if expert_name.is_empty() { expert_id } else { expert_name };
    json!({
        "eventCode": "expert_actual_use", "reportDelay": 0,
        "extVersion": "2.2.8", "source": "mini_program",
        "id": expert_id, "name": expert_id,
        "expertTitle": name, "type": "send_message",
        "characterCount": 12, "expertType": expert_type
    })
}

// ---------------------------------------------------------------------------
// billing 域 chat_request_send（report.go 全字段形状；userId 必填）
// ---------------------------------------------------------------------------

/// 一条 billing 域 `chat_request_send`（全字段，缺 `userId` 会被上游静默丢弃）。
pub fn chat_request_send_event(uid: &str, cid: &str, request_id: &str, model_id: &str, model_name: &str, now_ms: i64) -> Value {
    let request_id = if request_id.is_empty() { cid } else { request_id };
    let model_id = if model_id.is_empty() { "deepseek-v4-flash" } else { model_id };
    let model_name = if model_name.is_empty() { model_id } else { model_name };
    json!({
        "eventCode": "chat_request_send",
        "timestamp": now_ms,
        "reportDelay": 0,
        "mode": "craft",
        "conversationId": cid,
        "requestId": request_id,
        "inputLength": 12,
        "requestModelId": model_id,
        "requestModelName": model_name,
        "isPlan": false,
        "isAutoExecuteTerminal": false,
        "isAutoModify": false,
        "codebaseEnable": false,
        "maxToken": 0,
        "maxSteps": 0,
        "temperature": 0,
        "maxRetries": 0,
        "mentionContexts": [],
        "knowledgeId": [],
        "knowledgeName": [],
        "codebaseId": "",
        "mentionContextCount": 0,
        "command": "",
        "expertId": "",
        "recommendId": "",
        "skillId": "",
        "skillCount": 0,
        "totalCount": 0,
        "fileUri": "",
        "presentAt": now_ms,
        "traceId": "",
        "rootRequestId": request_id,
        "parentConversationId": cid,
        "agentName": "default",
        "agentType": "conversation",
        "userId": uid
    })
}

// ---------------------------------------------------------------------------
// 专家市场请求体
// ---------------------------------------------------------------------------

/// 专家市场列表请求体（POST /portal/operation-platform/market/expert/list）。
pub fn expert_list_body(expert_type: &str) -> Value {
    let mut body = json!({"page": 1, "page_size": 20, "sort_by": "reco_rank", "sort_order": "desc"});
    if !expert_type.is_empty() {
        body["expert_type"] = json!(expert_type);
    }
    body
}

// ---------------------------------------------------------------------------
// 真实对话请求体（SSE；/v2/chat/completions）
// ---------------------------------------------------------------------------

/// 真实对话 body（fast-model 短问，消耗可忽略；SSE 返回服务端 requestId）。
pub fn real_chat_body() -> Value {
    json!({
        "model": "fast-model",
        "messages": [
            {"role": "system", "content": "You are a helpful assistant. 当前处于中文环境，使用简体中文回答。"},
            {"role": "user", "content": "1+1等于几？直接回答。"}
        ],
        "agent": "cli",
        "temperature": 1,
        "stream": true,
        "stream_options": {"include_usage": true}
    })
}

/// 指定模型的真实对话 body（Model_chat / 夜猫子夜间补足）。
pub fn real_chat_body_model(model: &str) -> Value {
    json!({
        "model": model,
        "messages": [
            {"role": "user", "content": "1+1等于几？直接回答。"}
        ],
        "stream": true
    })
}

/// 服务端 requestId 合法形状（SSE 首个 `data.id`；`cmb-` 前缀 32hex 或裸 32hex）。
pub fn server_request_id_ok(id: &str) -> bool {
    let core = id.strip_prefix("cmb-").unwrap_or(id);
    core.len() == 32 && core.bytes().all(|b| b.is_ascii_hexdigit())
}

// ---------------------------------------------------------------------------
// 单测：事件形状与 Go 参考实现对照
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_id_is_stable_and_36_hex() {
        let a = derive_id("uid-1", "machine");
        let b = derive_id("uid-1", "machine");
        assert_eq!(a, b, "同 uid+salt 必须幂等");
        assert_eq!(a.len(), 36);
        assert!(a.bytes().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(derive_id("uid-2", "machine"), a, "不同 uid 不同值");
    }

    #[test]
    fn desktop_chat_sequence_has_six_events_with_join_ids() {
        let events = desktop_chat_sequence("conv1", "req1", "msg1", "fast-model", "fast-model");
        assert_eq!(events.len(), 6);
        assert_eq!(events[0].0, "agent_task_created");
        assert_eq!(events[3].0, "chat_message_response");
        // 所有事件都必须 JOIN 会话/请求 id。
        assert_eq!(events[0].1["conversationId"], "conv1");
        assert_eq!(events[5].1["rootRequestId"], "req1");
    }

    #[test]
    fn buddy_app_sequence_covers_five_events() {
        let events = buddy_app_sequence("cb_x", "企鹅教师助手");
        assert_eq!(events.len(), 5);
        let codes: Vec<&str> = events.iter().map(|(c, _)| c.as_str()).collect();
        assert!(codes[0].ends_with("discover_click"));
        assert!(codes[4].contains("bindaccount_skip"));
        assert_eq!(events[1].1["buddyId"], "cb_x");
    }

    #[test]
    fn mp_events_are_deduped_by_base_override() {
        let ev = json!({"eventCode": "chat_request_send", "conversationId": "c1", "userId": "u1"});
        let merged = with_mp_fp("u1", "nick", 42, &ev);
        // mp base 的 userId 与业务字段一致时不冲突；ideType 必须是 mp 形态。
        assert_eq!(merged["ideType"], "WorkBuddy_MP");
        assert_eq!(merged["eventCode"], "chat_request_send");
        assert_eq!(merged["conversationId"], "c1");
    }

    #[test]
    fn school_season_event_carries_activity_id() {
        let ev = school_season_chat_event("conv-9");
        assert_eq!(ev["activityId"], SCHOOL_OPEN_DAY_ACTIVITY_ID);
        assert_eq!(ev["eventCode"], "chat_request_send");
    }

    #[test]
    fn chat_request_send_requires_user_id_field() {
        let ev = chat_request_send_event("u-1", "c", "", "", "", 7);
        assert_eq!(ev["userId"], "u-1");
        assert_eq!(ev["requestModelId"], "deepseek-v4-flash", "空模型回落默认");
    }

    #[test]
    fn server_request_id_shape() {
        assert!(server_request_id_ok("abcdef0123456789abcdef0123456789"));
        assert!(server_request_id_ok("cmb-abcdef0123456789abcdef0123456789"));
        assert!(!server_request_id_ok("cmb-short"));
        assert!(!server_request_id_ok("zzzz"));
    }

    #[test]
    fn appearance_set_body_shape() {
        assert_eq!(appearance_set_body("theme-tkmw7j"), json!({"kind": "theme", "resource_key": "theme-tkmw7j"}));
    }

    #[test]
    fn expert_list_body_conditional_type() {
        assert!(expert_list_body("").get("expert_type").is_none());
        assert_eq!(expert_list_body("agent")["expert_type"], "agent");
    }
}
