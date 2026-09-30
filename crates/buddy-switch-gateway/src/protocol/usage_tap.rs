//! 流式响应的 `usage` 探针：从上游 SSE 里抓取末帧用量并回填成本账本。
//!
//! # 为什么需要它
//!
//! 出站改写管线**强制 `stream: true`**（上游不接受非流式），客户端默认也是流式；
//! 而成本账本（`(账号, 模型)` 每千 token 单价）此前**只在非流式客户端路径**上回填
//! （见 `routes/chat.rs` 的聚合分支）。后果是：在实际流量上账本几乎永远为空 →
//! `cost_tier` 恒为「无观测」→ **选号的成本分层硬过滤退化成空操作**，整个
//! 「账本择优」能力形同未启用。
//!
//! # 设计约束
//!
//! - 探针**嵌在** [`super::openai::SsePassthrough`] **之内**（更靠近上游），只观察字节、
//!   不修改字节，因此透传语义与「恰好补发一个 `[DONE]`」的保证完全不受影响。
//! - **只在拿到真实 `credit` 且 `tokens > 0` 时回报**。宁可不记，也不能记一个
//!   0 成本样本——那会把账号误标成「免费」，比没有样本更糟（会误导成本分层）。
//! - 回报是**尽力而为**：拿不到池的写锁就跳过本次样本，绝不阻塞流式响应。

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Bytes;
use futures_util::Stream;
use serde_json::Value;

/// 单帧缓冲区上限：畸形流（永不出现 `\n\n`）不得让内存无界增长。
const MAX_FRAME_BUFFER_BYTES: usize = 256 * 1024;

/// 流内观测到的用量：`(credit, prompt, completion)`；credit 或 tokens 缺位为 0。
type ObservedUsage = (f64, i64, i64);

/// 用量回报接收方。
///
/// 抽象成 trait 是为了让单测用记录型替身，不必真的起账号池；
/// 与 `cat.rs` 的 `CatIo` / `school.rs` 的 `SchoolIo` 是同一套可注入缝思路。
pub trait UsageSink: Send + Sync + 'static {
    /// 收到一次上游用量回报；prompt/completion 缺失时为 0（仍允许 credit>0 写入账本）。
    fn record(&self, credit: f64, prompt_tokens: i64, completion_tokens: i64);
}

/// 从一帧 JSON 里提取 `(credit, prompt, completion)`。
///
/// 返回 `None` 的情形（都**不得**写入账本）：无 `usage` 字段、`usage` 为 `null`、
/// **缺 `credit`**（拿不到单价就得不出成本）、`tokens <= 0`。
pub fn extract_usage(frame: &Value) -> Option<ObservedUsage> {
    let usage = frame.get("usage")?;
    if usage.is_null() {
        return None;
    }
    let credit = usage.get("credit").and_then(Value::as_f64)?;
    let mut prompt = usage
        .get("prompt_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let mut completion = usage
        .get("completion_tokens")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    prompt = prompt.max(0);
    completion = completion.max(0);
    if prompt + completion <= 0 {
        return None;
    }
    Some((credit, prompt, completion))
}

/// `usage` 探针流包装。**不改变字节流**，只在流结束（或出错）时回报一次用量。
pub struct UsageTap<S> {
    inner: Pin<Box<S>>,
    sink: Option<Arc<dyn UsageSink>>,
    buffer: Vec<u8>,
    /// 最近一次见到的用量（后到的帧覆盖先到的）。
    latest: Option<ObservedUsage>,
    reported: bool,
    finished: bool,
}

impl<S> UsageTap<S> {
    /// 包装上游流；`sink` 为 `None` 时只做缓冲（等价于不加探针）。
    pub fn new(inner: S, sink: Option<Arc<dyn UsageSink>>) -> Self {
        Self {
            inner: Box::pin(inner),
            sink,
            buffer: Vec::new(),
            latest: None,
            reported: false,
            finished: false,
        }
    }

    /// 累积字节并解析所有**完整**帧（以空行分隔）；残帧留在缓冲区等下一块。
    fn scan(&mut self, chunk: &[u8]) {
        self.buffer.extend_from_slice(chunk);
        while let Some(position) = find_subslice(&self.buffer, b"\n\n") {
            let frame: Vec<u8> = self.buffer.drain(..position + 2).collect();
            self.observe_frame(&frame);
        }
        if self.buffer.len() > MAX_FRAME_BUFFER_BYTES {
            self.buffer.clear();
        }
    }

    /// 解析单帧里的 `data:` 负载，命中 `usage` 就记住。
    fn observe_frame(&mut self, frame: &[u8]) {
        let text = String::from_utf8_lossy(frame);
        for line in text.lines() {
            let Some(payload) = line.strip_prefix("data:") else {
                continue;
            };
            let payload = payload.trim();
            if payload.is_empty() || payload == "[DONE]" {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(payload) else {
                continue;
            };
            if let Some(usage) = extract_usage(&value) {
                self.latest = Some(usage);
            }
        }
    }

    /// 回报一次（幂等）。
    fn report(&mut self) {
        if self.reported {
            return;
        }
        self.reported = true;
        if let (Some(sink), Some((credit, prompt, completion))) =
            (self.sink.as_ref(), self.latest)
        {
            sink.record(credit, prompt, completion);
        }
    }
}

impl<S, E> Stream for UsageTap<S>
where
    S: Stream<Item = Result<Bytes, E>> + Send,
    E: std::fmt::Debug,
{
    type Item = Result<Bytes, E>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        if this.finished {
            return Poll::Ready(None);
        }
        match this.inner.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                this.scan(&chunk);
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Err(error))) => {
                this.finished = true;
                this.report();
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                this.finished = true;
                this.report();
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// 子串查找（避免为此引入额外依赖）。
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;
    use serde_json::json;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingSink {
        calls: Mutex<Vec<(f64, i64, i64)>>,
    }

    impl RecordingSink {
        fn calls(&self) -> Vec<(f64, i64, i64)> {
            self.calls.lock().expect("lock").clone()
        }
    }

    impl UsageSink for RecordingSink {
        fn record(&self, credit: f64, prompt_tokens: i64, completion_tokens: i64) {
            self.calls
                .lock()
                .expect("lock")
                .push((credit, prompt_tokens, completion_tokens));
        }
    }

    fn stream_of(chunks: Vec<&'static str>) -> impl Stream<Item = Result<Bytes, std::io::Error>> {
        futures_util::stream::iter(
            chunks
                .into_iter()
                .map(|text| Ok(Bytes::from_static(text.as_bytes()))),
        )
    }

    const USAGE_FRAME: &str =
        "data: {\"choices\":[],\"usage\":{\"credit\":1.5,\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n";

    #[tokio::test]
    async fn reports_usage_once_at_stream_end() {
        let sink = Arc::new(RecordingSink::default());
        let stream = stream_of(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            USAGE_FRAME,
            "data: [DONE]\n\n",
        ]);
        let collected: Vec<_> = UsageTap::new(stream, Some(sink.clone())).collect().await;
        assert_eq!(collected.len(), 3, "字节必须原样透传，一帧不少");
        assert_eq!(sink.calls(), vec![(1.5, 10, 5)], "prompt/completion 分列回报");
    }

    #[tokio::test]
    async fn reassembles_frames_split_across_chunks() {
        let sink = Arc::new(RecordingSink::default());
        // 帧被切在中间（含切在 `\n\n` 边界之间），探针必须能重组
        let stream = stream_of(vec![
            "data: {\"choices\":[],\"us",
            "age\":{\"credit\":2.0,\"prompt_tokens\":7,\"completion_tokens\":3}}",
            "\n\ndata: [DONE]\n\n",
        ]);
        let _ = UsageTap::new(stream, Some(sink.clone())).collect::<Vec<_>>().await;
        assert_eq!(sink.calls(), vec![(2.0, 7, 3)], "跨块帧必须被重组");
    }

    #[tokio::test]
    async fn last_usage_wins_when_multiple_frames_carry_it() {
        let sink = Arc::new(RecordingSink::default());
        let stream = stream_of(vec![
            "data: {\"usage\":{\"credit\":1.0,\"prompt_tokens\":1,\"completion_tokens\":1}}\n\n",
            "data: {\"usage\":{\"credit\":9.0,\"prompt_tokens\":10,\"completion_tokens\":10}}\n\n",
        ]);
        let _ = UsageTap::new(stream, Some(sink.clone())).collect::<Vec<_>>().await;
        assert_eq!(sink.calls(), vec![(9.0, 10, 10)], "后到的用量应覆盖先到的");
    }

    #[tokio::test]
    async fn missing_or_unusable_usage_reports_nothing() {
        for chunks in vec![
            vec!["data: {\"choices\":[]}\n\n"],
            vec!["data: {\"usage\":null}\n\n"],
            // 缺 credit：拿不到单价，不得写样本
            vec!["data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n"],
            // tokens 为 0：不得写样本
            vec!["data: {\"usage\":{\"credit\":1.0,\"prompt_tokens\":0,\"completion_tokens\":0}}\n\n"],
            // 非 JSON 帧
            vec!["data: not json\n\n"],
        ] {
            let sink = Arc::new(RecordingSink::default());
            let _ = UsageTap::new(stream_of(chunks), Some(sink.clone()))
                .collect::<Vec<_>>()
                .await;
            assert!(sink.calls().is_empty(), "不该产生账本样本");
        }
    }

    #[tokio::test]
    async fn works_without_sink_and_still_passes_bytes_through() {
        let stream = stream_of(vec![USAGE_FRAME, "data: [DONE]\n\n"]);
        let collected: Vec<_> = UsageTap::new(stream, None).collect().await;
        assert_eq!(collected.len(), 2);
        assert!(collected.iter().all(|item| item.is_ok()));
    }

    #[tokio::test]
    async fn upstream_error_still_reports_and_propagates() {
        let sink = Arc::new(RecordingSink::default());
        let stream = futures_util::stream::iter(vec![
            Ok(Bytes::from_static(USAGE_FRAME.as_bytes())),
            Err(std::io::Error::other("upstream boomed")),
        ]);
        let collected: Vec<_> = UsageTap::new(stream, Some(sink.clone())).collect().await;
        assert_eq!(collected.len(), 2, "错误必须原样向上游调用方传播");
        assert!(collected[1].is_err());
        assert_eq!(
            sink.calls(),
            vec![(1.5, 10, 5)],
            "流中途异常时已见到的用量仍应回报（否则该次调用永远不记账）"
        );
    }

    #[test]
    fn extract_usage_rejects_degenerate_payloads() {
        assert_eq!(extract_usage(&json!({})), None);
        assert_eq!(extract_usage(&json!({"usage": null})), None);
        assert_eq!(
            extract_usage(&json!({"usage": {"prompt_tokens": 1, "completion_tokens": 1}})),
            None,
            "缺 credit 必须拒绝"
        );
        assert_eq!(
            extract_usage(&json!({"usage": {"credit": 1.0}})),
            None,
            "tokens 为 0 必须拒绝"
        );
        // 负数 token 被钳到 0 后仍为 0 → 拒绝
        assert_eq!(
            extract_usage(&json!({"usage": {"credit": 1.0, "prompt_tokens": -5, "completion_tokens": 2}})),
            Some((1.0, 0, 2)),
            "负数项应被钳制为 0，而不是让求和变成负数"
        );
        // 只有 prompt_tokens 也应可用
        assert_eq!(
            extract_usage(&json!({"usage": {"credit": 0.0, "prompt_tokens": 3}})),
            Some((0.0, 3, 0)),
            "显式 credit=0 是合法样本（免费模型），不同于「缺 credit」"
        );
    }

    #[test]
    fn find_subslice_handles_boundaries() {
        assert_eq!(find_subslice(b"a\n\nb", b"\n\n"), Some(1));
        assert_eq!(find_subslice(b"abc", b"\n\n"), None);
        assert_eq!(find_subslice(b"", b"\n\n"), None);
        assert_eq!(find_subslice(b"abc", b""), None);
    }
}
