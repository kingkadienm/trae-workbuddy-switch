//! 协议编解码：OpenAI 与 Anthropic 两种对外协议 ↔ 上游（OpenAI 形态）。

pub mod anthropic;
pub mod error_sniffer;
pub mod openai;
pub mod reasoning;
pub mod responses;
pub mod usage_tap;
