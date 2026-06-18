//! 阿里云百炼实时流式后端：qwen3-asr-flash-realtime（WebSocket，M6，任务 6.2）。
//!
//! 协议为 OpenAI-Realtime 风格（DashScope `api-ws/v1/realtime`，model 在 URL query）：
//! 连接 → 发 session.update（pcm/16k/语言 + server_vad）→ 收 session.created/updated →
//! 发 input_audio_buffer.append（**base64 PCM16**）→ 收
//! conversation.item.input_audio_transcription.text（中间，text+stash）/ .completed（最终，transcript）
//! → 发 session.finish → 收 session.finished。断开自动重连（≤3 次，1/2/4s 退避）；
//! 音频在 MPSC 通道中缓冲不丢。

use std::time::Duration;

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_tungstenite::tungstenite::Message;
use uuid::Uuid;

use crate::asr::config::AsrConfig;
use crate::asr::error::AsrError;
use crate::asr::traits::{AsrBackend, StreamingResult};
use crate::asr::websocket::connect;

const MODEL: &str = "qwen3-asr-flash-realtime";
const WS_BASE: &str = "wss://dashscope.aliyuncs.com/api-ws/v1/realtime";
const MAX_RETRIES: usize = 3;

pub struct BailianStreamingBackend;

impl BailianStreamingBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BailianStreamingBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AsrBackend for BailianStreamingBackend {
    fn backend_id(&self) -> &str {
        "aliyun_bailian_streaming"
    }

    fn display_name(&self) -> &str {
        "阿里云百炼（实时）"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_offline(&self) -> bool {
        false
    }

    async fn validate_config(&self, config: &AsrConfig) -> Result<(), AsrError> {
        let api_key = config.api_key.trim();
        if api_key.is_empty() {
            return Err(AsrError::AuthError);
        }
        // 真实握手测试（连接成功即通过；401/403 → AuthError）。
        let _ws = connect(&resolve_endpoint(config), api_key).await?;
        Ok(())
    }

    async fn transcribe_streaming(
        &self,
        config: &AsrConfig,
        mut audio_rx: Receiver<Vec<u8>>,
        result_tx: Sender<StreamingResult>,
    ) -> Result<(), AsrError> {
        let api_key = config.api_key.trim().to_string();
        if api_key.is_empty() {
            return Err(AsrError::AuthError);
        }
        let endpoint = resolve_endpoint(config);
        let lang = language_hint(&config.language);

        let mut attempt = 0usize;
        loop {
            match run_session(&endpoint, &api_key, lang, &mut audio_rx, &result_tx).await {
                SessionOutcome::Finished => return Ok(()),
                SessionOutcome::Fatal(e) => return Err(e),
                SessionOutcome::Disconnected(e) => {
                    if attempt >= MAX_RETRIES {
                        return Err(e);
                    }
                    attempt += 1;
                    let backoff = 1u64 << (attempt - 1); // 1s, 2s, 4s
                    tracing::warn!(
                        attempt,
                        backoff_secs = backoff,
                        error = %e,
                        "实时识别断开，准备重连（音频在通道中缓冲，不丢失）"
                    );
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
            "实时后端不支持离线整段识别".to_string(),
        ))
    }
}

/// 一次连接会话的结果。
enum SessionOutcome {
    /// 正常收到 session.finished。
    Finished,
    /// 连接层断开 → 可重试。
    Disconnected(AsrError),
    /// 鉴权/服务端错误 → 不重试。
    Fatal(AsrError),
}

/// 单次 WebSocket 连接会话：session.update → 流式收发 → session.finish。
async fn run_session(
    endpoint: &str,
    api_key: &str,
    lang: Option<&str>,
    audio_rx: &mut Receiver<Vec<u8>>,
    result_tx: &Sender<StreamingResult>,
) -> SessionOutcome {
    let ws = match connect(endpoint, api_key).await {
        Ok(w) => w,
        // 握手 401/403 → 鉴权错误，不重试。
        Err(e @ AsrError::AuthError) => return SessionOutcome::Fatal(e),
        Err(e) => return SessionOutcome::Disconnected(e),
    };
    let (mut sink, mut stream) = ws.split();

    // 连接后立即发送会话配置（OpenAI-Realtime 允许在 session.created 之前发送）。
    let update = session_update_message(lang);
    if let Err(e) = sink.send(Message::Text(update.to_string())).await {
        return SessionOutcome::Disconnected(AsrError::WebSocketError(format!(
            "发送 session.update 失败: {e}"
        )));
    }
    tracing::info!("实时识别：已发送 session.update");

    let mut started = false;
    let mut finishing = false;

    loop {
        tokio::select! {
            maybe_chunk = audio_rx.recv(), if started && !finishing => {
                match maybe_chunk {
                    Some(chunk) => {
                        let msg = append_message(&BASE64.encode(&chunk));
                        if let Err(e) = sink.send(Message::Text(msg.to_string())).await {
                            return SessionOutcome::Disconnected(AsrError::WebSocketError(format!("发送音频失败: {e}")));
                        }
                    }
                    None => {
                        // 录音停止（通道关闭）→ session.finish，等待最终结果。
                        let msg = finish_message();
                        if let Err(e) = sink.send(Message::Text(msg.to_string())).await {
                            return SessionOutcome::Disconnected(AsrError::WebSocketError(format!("发送 session.finish 失败: {e}")));
                        }
                        finishing = true;
                        tracing::info!("实时识别：已发送 session.finish");
                    }
                }
            }
            maybe_msg = stream.next() => {
                match maybe_msg {
                    Some(Ok(Message::Text(text))) => match handle_text(&text, result_tx).await {
                        Event::Started => {
                            if !started {
                                started = true;
                                tracing::info!("实时识别：会话就绪，开始发送音频");
                            }
                        }
                        Event::Continue => {}
                        Event::Finished => return SessionOutcome::Finished,
                        Event::Failed(e) => return SessionOutcome::Fatal(e),
                    },
                    Some(Ok(Message::Close(_))) => {
                        return if finishing {
                            SessionOutcome::Finished
                        } else {
                            SessionOutcome::Disconnected(AsrError::WebSocketError("服务端关闭连接".to_string()))
                        };
                    }
                    // Binary/Ping/Pong/Frame：忽略（ping/pong 由库自动处理）。
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        return SessionOutcome::Disconnected(AsrError::WebSocketError(format!("WS 接收错误: {e}")));
                    }
                    None => {
                        return if finishing {
                            SessionOutcome::Finished
                        } else {
                            SessionOutcome::Disconnected(AsrError::WebSocketError("连接意外结束".to_string()))
                        };
                    }
                }
            }
        }
    }
}

/// 服务端事件处理结果。
enum Event {
    /// 会话就绪（session.created / session.updated）。
    Started,
    Continue,
    /// session.finished。
    Finished,
    /// error / 鉴权失败。
    Failed(AsrError),
}

/// 解析一条服务端事件；识别结果发送到 `result_tx`。
async fn handle_text(text: &str, result_tx: &Sender<StreamingResult>) -> Event {
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("无法解析 WS 文本事件: {e}");
            return Event::Continue;
        }
    };

    match value["type"].as_str().unwrap_or_default() {
        "session.created" | "session.updated" => Event::Started,

        // 中间结果：text 为已确认前缀，stash 为暂定尾部，合并为当前整句。
        "conversation.item.input_audio_transcription.text" => {
            let confirmed = value["text"].as_str().unwrap_or("");
            let stash = value["stash"].as_str().unwrap_or("");
            let combined = format!("{confirmed}{stash}");
            if !combined.is_empty() {
                send_result(result_tx, combined, false).await;
            }
            Event::Continue
        }

        // 最终结果：整句固化。
        "conversation.item.input_audio_transcription.completed" => {
            if let Some(transcript) = value["transcript"].as_str()
                && !transcript.is_empty()
            {
                send_result(result_tx, transcript.to_string(), true).await;
            }
            Event::Continue
        }

        "conversation.item.input_audio_transcription.failed" => {
            tracing::warn!("实时识别：单条转写失败，继续会话");
            Event::Continue
        }

        "session.finished" => Event::Finished,

        "error" => {
            let code = value["error"]["code"].as_str().unwrap_or("");
            let message = value["error"]["message"].as_str().unwrap_or("未知错误");
            tracing::error!(code, message, "实时识别错误事件");
            if is_auth_error(code, message) {
                Event::Failed(AsrError::AuthError)
            } else {
                Event::Failed(AsrError::WebSocketError(format!(
                    "服务端错误[{code}]: {message}"
                )))
            }
        }

        other => {
            tracing::debug!(event = other, "忽略 WS 事件");
            Event::Continue
        }
    }
}

async fn send_result(result_tx: &Sender<StreamingResult>, text: String, is_final: bool) {
    let _ = result_tx
        .send(StreamingResult {
            delta_text: text,
            is_final,
            timestamp: Utc::now(),
        })
        .await;
}

fn is_auth_error(code: &str, message: &str) -> bool {
    let c = code.to_ascii_lowercase();
    let m = message.to_ascii_lowercase();
    c.contains("auth")
        || c.contains("apikey")
        || c.contains("accessdenied")
        || m.contains("api key")
        || m.contains("apikey")
        || m.contains("unauthorized")
}

fn session_update_message(lang: Option<&str>) -> Value {
    let mut transcription = json!({});
    if let Some(language) = lang {
        transcription["language"] = json!(language);
    }
    json!({
        "event_id": event_id(),
        "type": "session.update",
        "session": {
            "input_audio_format": "pcm",
            "sample_rate": 16000,
            "input_audio_transcription": transcription,
            "turn_detection": {
                "type": "server_vad",
                "threshold": 0.0,
                "silence_duration_ms": 400
            }
        }
    })
}

fn append_message(audio_b64: &str) -> Value {
    json!({
        "event_id": event_id(),
        "type": "input_audio_buffer.append",
        "audio": audio_b64
    })
}

fn finish_message() -> Value {
    json!({ "event_id": event_id(), "type": "session.finish" })
}

fn event_id() -> String {
    format!("event_{}", Uuid::new_v4())
}

/// 解析 WebSocket 端点。§2.7 默认 api_endpoint 是旧 inference URL，对本模型不适用；
/// 仅当用户显式配置了含 `/realtime` 的端点才覆盖，否则用本模型的 realtime URL。
fn resolve_endpoint(config: &AsrConfig) -> String {
    if config.api_endpoint.contains("/realtime") {
        config.api_endpoint.clone()
    } else {
        format!("{WS_BASE}?model={MODEL}")
    }
}

/// 语言：明确时下发，"auto" 时省略让模型自动判别。
fn language_hint(language: &str) -> Option<&'static str> {
    match language {
        "zh" => Some("zh"),
        "en" => Some("en"),
        _ => None,
    }
}
