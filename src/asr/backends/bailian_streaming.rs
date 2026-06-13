//! 阿里云百炼实时流式后端：paraformer-realtime-v2（WebSocket，M6，任务 6.2）。
//!
//! 协议（DashScope api-ws）：连接 → 发 run-task → 收 task-started → 发二进制 PCM 帧
//! → 收 result-generated（payload.output.sentence.{text,sentence_end}）→ 发 finish-task
//! → 收 task-finished。断开自动重连（≤3 次，1/2/4s 退避）；音频在 MPSC 通道中缓冲不丢。

use std::time::Duration;

use async_trait::async_trait;
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

const WS_URL: &str = "wss://dashscope.aliyuncs.com/api-ws/v1/inference";
const MODEL: &str = "qwen3-asr-flash-realtime";
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
        if config.api_key.trim().is_empty() {
            return Err(AsrError::AuthError);
        }
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
        let endpoint = if config.api_endpoint.starts_with("wss://") {
            config.api_endpoint.clone()
        } else {
            WS_URL.to_string()
        };
        let lang = language_hints(&config.language);

        let mut attempt = 0usize;
        loop {
            match run_session(&endpoint, &api_key, &lang, &mut audio_rx, &result_tx).await {
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
    /// 正常收到 task-finished。
    Finished,
    /// 连接层断开 → 可重试。
    Disconnected(AsrError),
    /// 鉴权/任务失败 → 不重试。
    Fatal(AsrError),
}

/// 单次 WebSocket 连接会话：run-task → 流式收发 → finish-task。
async fn run_session(
    endpoint: &str,
    api_key: &str,
    lang: &Option<Vec<String>>,
    audio_rx: &mut Receiver<Vec<u8>>,
    result_tx: &Sender<StreamingResult>,
) -> SessionOutcome {
    let ws = match connect(endpoint, api_key).await {
        Ok(w) => w,
        Err(e) => return SessionOutcome::Disconnected(e),
    };
    let (mut sink, mut stream) = ws.split();
    let task_id = Uuid::new_v4().to_string();

    let run_msg = run_task_message(&task_id, lang);
    if let Err(e) = sink.send(Message::Text(run_msg.to_string())).await {
        return SessionOutcome::Disconnected(AsrError::WebSocketError(format!(
            "发送 run-task 失败: {e}"
        )));
    }
    tracing::info!(%task_id, "实时识别：已发送 run-task");

    let mut started = false;
    let mut finishing = false;

    loop {
        tokio::select! {
            maybe_chunk = audio_rx.recv(), if started && !finishing => {
                match maybe_chunk {
                    Some(chunk) => {
                        if let Err(e) = sink.send(Message::Binary(chunk)).await {
                            return SessionOutcome::Disconnected(AsrError::WebSocketError(format!("发送音频失败: {e}")));
                        }
                    }
                    None => {
                        // 录音停止（通道关闭）→ finish-task，等待最终结果。
                        let fin = finish_task_message(&task_id);
                        if let Err(e) = sink.send(Message::Text(fin.to_string())).await {
                            return SessionOutcome::Disconnected(AsrError::WebSocketError(format!("发送 finish-task 失败: {e}")));
                        }
                        finishing = true;
                        tracing::info!("实时识别：已发送 finish-task");
                    }
                }
            }
            maybe_msg = stream.next() => {
                match maybe_msg {
                    Some(Ok(Message::Text(text))) => match handle_text(&text, result_tx).await {
                        Event::Started => {
                            started = true;
                            tracing::info!("实时识别：task-started");
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
    Started,
    Continue,
    Finished,
    Failed(AsrError),
}

/// 解析一条文本事件；若是识别结果则发送到 `result_tx`。
async fn handle_text(text: &str, result_tx: &Sender<StreamingResult>) -> Event {
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            tracing::debug!("无法解析 WS 文本事件: {e}");
            return Event::Continue;
        }
    };

    match value["header"]["event"].as_str().unwrap_or_default() {
        "task-started" => Event::Started,
        "result-generated" => {
            let sentence = &value["payload"]["output"]["sentence"];
            let heartbeat = sentence["heartbeat"].as_bool().unwrap_or(false);
            if let Some(sentence_text) = sentence["text"].as_str()
                && !heartbeat
                && !sentence_text.is_empty()
            {
                let is_final = sentence["sentence_end"].as_bool().unwrap_or(false);
                // delta_text 在此承载"当前整句文本"（DashScope 发整句而非增量）；
                // UI 据 is_final 决定替换 pending 还是固化到 text_content（§4.2.1）。
                let _ = result_tx
                    .send(StreamingResult {
                        delta_text: sentence_text.to_string(),
                        is_final,
                        timestamp: Utc::now(),
                    })
                    .await;
            }
            Event::Continue
        }
        "task-finished" => Event::Finished,
        "task-failed" => {
            let code = value["header"]["error_code"].as_str().unwrap_or("");
            let message = value["header"]["error_message"].as_str().unwrap_or("未知错误");
            tracing::error!(code, message, "实时识别任务失败");
            if is_auth_error(code, message) {
                Event::Failed(AsrError::AuthError)
            } else {
                Event::Failed(AsrError::WebSocketError(format!("任务失败[{code}]: {message}")))
            }
        }
        other => {
            tracing::debug!(event = other, "忽略未知 WS 事件");
            Event::Continue
        }
    }
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

fn run_task_message(task_id: &str, lang: &Option<Vec<String>>) -> Value {
    let mut parameters = json!({
        "format": "pcm",
        "sample_rate": 16000,
    });
    if let Some(hints) = lang {
        parameters["language_hints"] = json!(hints);
    }
    json!({
        "header": { "action": "run-task", "task_id": task_id, "streaming": "duplex" },
        "payload": {
            "task_group": "audio",
            "task": "asr",
            "function": "recognition",
            "model": MODEL,
            "parameters": parameters,
            "input": {}
        }
    })
}

fn finish_task_message(task_id: &str) -> Value {
    json!({
        "header": { "action": "finish-task", "task_id": task_id, "streaming": "duplex" },
        "payload": { "input": {} }
    })
}

/// 语言提示：明确语言时下发，"auto" 时省略让模型自动判别。
fn language_hints(language: &str) -> Option<Vec<String>> {
    match language {
        "zh" => Some(vec!["zh".to_string()]),
        "en" => Some(vec!["en".to_string()]),
        _ => None,
    }
}
