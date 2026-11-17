//! 视频任务的进程内存储：`tokio::sync::RwLock<HashMap<...>>`。
use std::collections::HashMap;

use tokio::sync::RwLock;

/// 一个视频任务的当前状态。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoTaskStatus {
    /// 已提交到上游，等待产出。
    Pending,
    /// 上游侧仍在生成。
    Generating,
    /// 上游已给出结果（P1 不做下载，只透传原始结果）。
    Completed,
    /// 上游明确返回任务失败。
    Failed(String),
}

impl VideoTaskStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed(_))
    }

    /// 稳定的小写字符串标识（对外响应用）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Generating => "generating",
            Self::Completed => "completed",
            Self::Failed(_) => "failed",
        }
    }
}

/// 进程内的视频任务记录。
#[derive(Debug, Clone)]
pub struct VideoTask {
    /// 本地 id（`vid_` 前缀）。
    pub id: String,
    /// Key 绑定的归属域（`cn` / `global`）。
    pub region: String,
    pub key_id: String,
    /// 上游任务 id（提交成功才有）。
    pub upstream_task_id: Option<String>,
    /// 提交时绑定的账号 uid（后续查询固定用同一账号）。
    pub uid: String,
    /// 提交时绑定的账号名（仅诊断展示用）。
    pub account: String,
    /// 提交时的模型 id（已归一为上游 id）。
    pub model: String,
    /// 当前状态。
    pub status: VideoTaskStatus,
    /// 最近一次查询的上游原始负载（`null` = 尚未查询过）。
    pub upstream_payload: serde_json::Value,
    /// 创建时间（毫秒）。
    pub created_at_ms: i64,
    /// 最近一次状态变更时间（毫秒）。
    pub updated_at_ms: i64,
}

impl VideoTask {
    /// 对外 DTO（P1 不含积分 / 下载字段）。
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "region": self.region,
            "model": self.model,
            "status": self.status.as_str(),
            "error": match &self.status {
                VideoTaskStatus::Failed(message) => serde_json::Value::String(message.clone()),
                _ => serde_json::Value::Null,
            },
            "upstream_task_id": self.upstream_task_id,
            "upstream": self.upstream_payload,
            "created_at_ms": self.created_at_ms,
            "updated_at_ms": self.updated_at_ms,
        })
    }
}

/// 进程内任务表（P1 不落盘；进程重启即清空，与模块头声明一致）。
#[derive(Default)]
pub struct VideoTaskStore {
    tasks: RwLock<HashMap<String, VideoTask>>,
}

impl VideoTaskStore {
    /// 落一条新任务（同 id 覆盖：id 由本模块生成，正常不冲突）。
    pub async fn insert(&self, task: VideoTask) {
        self.tasks.write().await.insert(task.id.clone(), task);
    }

    /// 按 id 读取任务。
    pub async fn get(&self, id: &str) -> Option<VideoTask> {
        self.tasks.read().await.get(id).cloned()
    }

    /// 更新一条任务（存在才写，返回是否命中）。
    pub async fn update(&self, task: VideoTask) -> bool {
        let mut tasks = self.tasks.write().await;
        if !tasks
            .get(&task.id)
            .is_some_and(|existing| !existing.status.is_terminal())
        {
            return false;
        }
        tasks.insert(task.id.clone(), task);
        true
    }
}
