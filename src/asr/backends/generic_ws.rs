//! 通用 WebSocket 后端（M7，任务 7.2）。
//!
//! 面向用户自建的 ASR 服务：在配置里填自定义 WS URL（`api_endpoint`，ws:// 或 wss://）与
//! 鉴权（`api_key` 非空时作为 `Authorization: Bearer <key>`）。
//!
//! **约定协议**（自建服务需遵循）：
//! - 客户端 → 服务端：二进制帧，原始 PCM（16kHz/16-bit/单声道），每 100ms 一帧。
//! - 服务端 → 客户端：文本帧，优先 JSON `{"text":"...","is_final":bool}`（缺省 `is_final=false`），
//!   也接受纯文本（视为中间结果）。录音停止时客户端发 Close 帧，服务端给出最终结果后关闭连接。

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_tungstenite::tungstenite::Message;

use crate::asr::config::AsrConfig;
use crate::asr::error::AsrError;
use crate::asr::traits::{AsrBackend, StreamingResult};
use crate::asr::websocket::connect;

const MAX_RETRIES: usize = 3;

pub struct GenericWsBackend;

impl GenericWsBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for GenericWsBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AsrBackend for GenericWsBackend {
    fn backend_id(&self) -> &str {
        "generic_ws"
    }

    fn display_name(&self) -> &str {
        "通用 WebSocket"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_offline(&self) -> bool {
        false
    }

    async fn validate_config(&self, config: &AsrConfig) -> Result<(), AsrError> {
        let url = config.api_endpoint.trim();
        if url.is_empty() || !url.starts_with("ws") {
            return Err(AsrError::InvalidConfig(
                "请在配置 api_endpoint 填写通用 WebSocket 的 ws:// 或 wss:// URL".to_string(),
            ));
        }
        // 真实握手测试（401/403 → AuthError）。
        let _ws = connect(url, config.api_key.trim()).await?;
        Ok(())
    }

    async fn transcribe_streaming(
        &self,
        config: &AsrConfig,
        mut audio_rx: Receiver<Vec<u8>>,
        result_tx: Sender<StreamingResult>,
    ) -> Result<(), AsrError> {
        let url = config.api_endpoint.trim().to_string();
        if url.is_empty() || !url.starts_with("ws") {
            return Err(AsrError::InvalidConfig(
                "请在配置 api_endpoint 填写通用 WebSocket 的 ws:// 或 wss:// URL".to_string(),
            ));
        }
        let api_key = config.api_key.trim().to_string();

        let mut attempt = 0usize;
        loop {
            match run_session(&url, &api_key, &mut audio_rx, &result_tx).await {
                Outcome::Finished => return Ok(()),
                Outcome::Fatal(e) => return Err(e),
                Outcome::Disconnected(e) => {
                    if attempt >= MAX_RETRIES {
                        return Err(e);
                    }
                    attempt += 1;
                    let backoff = 1u64 << (attempt - 1);
                    tracing::warn!(attempt, backoff_secs = backoff, error = %e, "通用 WS 断开，准备重连");
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                }
            }
        }
    }

    async fn transcribe_offline(
        &self,
        _config: &AsrConfig,
        _audio_data: Vec<u8>,
    ) -> Result<String, AsrError> {
        Err(AsrError::InvalidConfig(
            "通用 WebSocket 后端不支持离线整段识别".to_string(),
        ))
    }
}

enum Outcome {
    Finished,
    Disconnected(AsrError),
    Fatal(AsrError),
}

async fn run_session(
    url: &str,
    api_key: &str,
    audio_rx: &mut Receiver<Vec<u8>>,
    result_tx: &Sender<StreamingResult>,
) -> Outcome {
    let ws = match connect(url, api_key).await {
        Ok(w) => w,
        Err(e @ AsrError::AuthError) => return Outcome::Fatal(e),
        Err(e) => return Outcome::Disconnected(e),
    };
    let (mut sink, mut stream) = ws.split();
    let mut finishing = false;

    loop {
        tokio::select! {
            maybe_chunk = audio_rx.recv(), if !finishing => {
                match maybe_chunk {
                    Some(chunk) => {
                        if let Err(e) = sink.send(Message::Binary(chunk)).await {
                            return Outcome::Disconnected(AsrError::WebSocketError(format!("发送音频失败: {e}")));
                        }
                    }
                    None => {
                        // 录音停止：发 Close，等待服务端最终结果。
                        let _ = sink.send(Message::Close(None)).await;
                        finishing = true;
                    }
                }
            }
            maybe_msg = stream.next() => {
                match maybe_msg {
                    Some(Ok(Message::Text(text))) => {
                        if let Some((delta_text, is_final)) = parse_result(&text) {
                            let _ = result_tx.send(StreamingResult { delta_text, is_final, timestamp: Utc::now() }).await;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        return if finishing { Outcome::Finished } else { Outcome::Disconnected(AsrError::WebSocketError("连接结束".to_string())) };
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => return Outcome::Disconnected(AsrError::WebSocketError(format!("WS 接收错误: {e}"))),
                }
            }
        }
    }
}

/// 解析服务端文本：JSON `{"text","is_final"}` 优先，否则纯文本作中间结果。
fn parse_result(text: &str) -> Option<(String, bool)> {
    if let Ok(value) = serde_json::from_str::<Value>(text)
        && let Some(t) = value["text"].as_str()
    {
        if t.is_empty() {
            return None;
        }
        let is_final = value["is_final"]
            .as_bool()
            .or_else(|| value["final"].as_bool())
            .unwrap_or(false);
        return Some((t.to_string(), is_final));
    }
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some((trimmed.to_string(), false))
    }
}
