//! Qwen3-ASR 自建服务后端（私有化部署，实时 + 离线一体）。
//!
//! 对接 Quantatirsk/qwen3-asr 单一部署服务，**同时支持实时与离线**（离线含大文件，
//! 服务端按 VAD 自动切分，无需 OSS 中转）。用户在配置里填**基址**（如 `http://host:17003`），
//! 本后端从基址派生 HTTP 与 WebSocket 两个端点。鉴权为**可选** Bearer（留空即匿名）。
//!
//! - 离线：`POST {base}/v1/audio/transcriptions`，multipart `file` + `response_format=text`，
//!   服务端直接返回纯文本。最大约 2GB（受服务端 `MAX_AUDIO_SIZE` 限制）。
//! - 实时：WebSocket `{base}/ws/v1/asr/qwen`。协议（见服务端 `qwen3_websocket_asr.py`）：
//!   客户端发 `{"type":"start","payload":{format,sample_rate,language,...}}` → 服务端回
//!   `{"type":"started"}` → 客户端发**二进制 PCM16 帧** → 服务端推
//!   `{"type":"result",results:[{current_segment_text,is_partial}]}`（中间）/
//!   `{"type":"segment_end",result:{segment_text}}`（一段确认）→ 客户端发 `{"type":"stop"}` →
//!   服务端回 `{"type":"final",result:{text,full_text}}`。出错为 `{"type":"error",error_code,message}`。
//!   断开自动重连（≤3 次，1/2/4s 退避；音频在 MPSC 通道缓冲不丢）。

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
use reqwest::multipart;
use serde_json::{json, Value};
use tokio::sync::mpsc::{Receiver, Sender};
use tokio_tungstenite::tungstenite::Message;

use crate::asr::config::AsrConfig;
use crate::asr::error::AsrError;
use crate::asr::traits::{AsrBackend, StreamingResult};
use crate::asr::websocket::connect_plain;

const MAX_RETRIES: usize = 3;

pub struct Qwen3AsrSelfhostedBackend {
    client: reqwest::Client,
}

impl Qwen3AsrSelfhostedBackend {
    pub fn new() -> Self {
        // 不设总超时：大文件上传 + 服务端推理可能耗时数分钟；仅设连接超时以便服务不可达时快速失败。
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

impl Default for Qwen3AsrSelfhostedBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AsrBackend for Qwen3AsrSelfhostedBackend {
    fn backend_id(&self) -> &str {
        "qwen3_asr_selfhosted"
    }

    fn display_name(&self) -> &str {
        "Qwen3-ASR 自建服务"
    }

    fn supports_streaming(&self) -> bool {
        true
    }

    fn supports_offline(&self) -> bool {
        true
    }

    async fn validate_config(&self, config: &AsrConfig) -> Result<(), AsrError> {
        let base = base_url(config)?;
        // 轻量探测：GET {base}/v1/models（若服务端配置了 API_KEY 则校验之，401 → AuthError）。
        let url = format!("{base}/v1/models");
        let mut req = self.client.get(&url);
        let api_key = config.api_key.trim();
        if !api_key.is_empty() {
            req = req.bearer_auth(api_key);
        }
        let resp = req.send().await.map_err(map_reqwest_error)?;
        match resp.status().as_u16() {
            401 | 403 => Err(AsrError::AuthError),
            code if (200..300).contains(&code) => Ok(()),
            code => {
                let detail = resp.text().await.unwrap_or_default();
                Err(AsrError::NetworkError(format!("HTTP {code}: {detail}")))
            }
        }
    }

    async fn transcribe_streaming(
        &self,
        config: &AsrConfig,
        mut audio_rx: Receiver<Vec<u8>>,
        result_tx: Sender<StreamingResult>,
    ) -> Result<(), AsrError> {
        let ws_url = ws_url(&base_url(config)?);
        let api_key = config.api_key.trim().to_string();
        let lang = language_hint(&config.language);

        let mut attempt = 0usize;
        loop {
            match run_session(&ws_url, &api_key, lang, &mut audio_rx, &result_tx).await {
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
                        "自建服务实时识别断开，准备重连（音频在通道中缓冲，不丢失）"
                    );
                    tokio::time::sleep(Duration::from_secs(backoff)).await;
                }
            }
        }
    }

    async fn transcribe_offline(
        &self,
        config: &AsrConfig,
        audio_data: Vec<u8>,
    ) -> Result<String, AsrError> {
        if audio_data.is_empty() {
            return Err(AsrError::EmptyAudio);
        }
        let base = base_url(config)?;
        let url = format!("{base}/v1/audio/transcriptions");

        let part = multipart::Part::bytes(audio_data)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(|e| AsrError::NetworkError(format!("构造上传表单失败: {e}")))?;
        let mut form = multipart::Form::new()
            .part("file", part)
            .text("response_format", "text");
        if let Some(lang) = language_hint(&config.language) {
            form = form.text("language", lang);
        }

        let mut req = self.client.post(&url).multipart(form);
        let api_key = config.api_key.trim();
        if !api_key.is_empty() {
            req = req.bearer_auth(api_key);
        }

        let response = req.send().await.map_err(map_reqwest_error)?;
        let status = response.status();
        match status.as_u16() {
            401 | 403 => return Err(AsrError::AuthError),
            code if !(200..300).contains(&code) => {
                let detail = response.text().await.unwrap_or_default();
                return Err(AsrError::NetworkError(format!("HTTP {code}: {detail}")));
            }
            _ => {}
        }

        // response_format=text → 纯文本响应体。
        let text = response
            .text()
            .await
            .map_err(|e| AsrError::NetworkError(format!("读取转写响应失败: {e}")))?
            .trim()
            .to_string();
        if text.is_empty() {
            return Err(AsrError::EmptyResult);
        }
        Ok(text)
    }
}

/// 一次连接会话的结果。
enum SessionOutcome {
    /// 正常收到 final。
    Finished,
    /// 连接层断开 → 可重试。
    Disconnected(AsrError),
    /// 服务端错误 → 不重试。
    Fatal(AsrError),
}

/// 单次 WebSocket 连接会话：start → 流式收发 → stop → final。
async fn run_session(
    ws_url: &str,
    api_key: &str,
    lang: Option<&str>,
    audio_rx: &mut Receiver<Vec<u8>>,
    result_tx: &Sender<StreamingResult>,
) -> SessionOutcome {
    let ws = match connect_plain(ws_url, api_key).await {
        Ok(w) => w,
        Err(e @ AsrError::AuthError) => return SessionOutcome::Fatal(e),
        Err(e) => return SessionOutcome::Disconnected(e),
    };
    let (mut sink, mut stream) = ws.split();

    // 连接后立即发送 start；服务端就绪后回 started，届时才开始发音频。
    if let Err(e) = sink.send(Message::Text(start_message(lang).to_string())).await {
        return SessionOutcome::Disconnected(AsrError::WebSocketError(format!(
            "发送 start 失败: {e}"
        )));
    }

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
                        // 录音停止（通道关闭）→ stop，等待 final。
                        if let Err(e) = sink.send(Message::Text(stop_message().to_string())).await {
                            return SessionOutcome::Disconnected(AsrError::WebSocketError(format!("发送 stop 失败: {e}")));
                        }
                        finishing = true;
                    }
                }
            }
            maybe_msg = stream.next() => {
                match maybe_msg {
                    Some(Ok(Message::Text(text))) => match handle_text(&text, result_tx).await {
                        Event::Started => {
                            if !started {
                                started = true;
                                tracing::info!("自建服务实时识别：会话就绪，开始发送音频");
                            }
                        }
                        Event::Continue => {}
                        Event::Finished => return SessionOutcome::Finished,
                        Event::Failed(e) => return SessionOutcome::Fatal(e),
                    },
                    Some(Ok(Message::Close(_))) | None => {
                        return if finishing {
                            SessionOutcome::Finished
                        } else {
                            SessionOutcome::Disconnected(AsrError::WebSocketError("连接意外结束".to_string()))
                        };
                    }
                    // Binary/Ping/Pong/Frame：忽略（ping/pong 由库自动处理）。
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        return SessionOutcome::Disconnected(AsrError::WebSocketError(format!("WS 接收错误: {e}")));
                    }
                }
            }
        }
    }
}

/// 服务端事件处理结果。
enum Event {
    /// 会话就绪（started）。
    Started,
    Continue,
    /// final（识别结束）。
    Finished,
    /// error。
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
        "started" => Event::Started,

        // 中间结果：取最新一条的当前段文本（仅进行中的当前段，已确认段已由 segment_end 固化）。
        "result" => {
            if let Some(cur) = value["results"]
                .as_array()
                .and_then(|arr| arr.last())
                .and_then(|r| r["current_segment_text"].as_str())
                && !cur.is_empty()
            {
                send_result(result_tx, cur.to_string(), false).await;
            }
            Event::Continue
        }

        // 一段确认（静音/超长触发）：固化该段文本。
        "segment_end" => {
            if let Some(seg) = value["result"]["segment_text"].as_str()
                && !seg.is_empty()
            {
                send_result(result_tx, seg.to_string(), true).await;
            }
            Event::Continue
        }

        "segment_start" => Event::Continue,

        // 结束：固化最后一段（其余段已在 segment_end 固化）。
        "final" => {
            if let Some(last) = value["result"]["text"].as_str()
                && !last.is_empty()
            {
                send_result(result_tx, last.to_string(), true).await;
            }
            Event::Finished
        }

        "error" => {
            let code = value["error_code"].as_str().unwrap_or("");
            let message = value["message"].as_str().unwrap_or("未知错误");
            tracing::error!(code, message, "自建服务实时识别错误事件");
            if code.to_ascii_uppercase().contains("AUTH") {
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

fn start_message(lang: Option<&str>) -> Value {
    let mut payload = json!({
        "format": "pcm",
        "sample_rate": 16000,
        "enable_inverse_text_normalization": true,
        "chunk_size_sec": 2.0
    });
    if let Some(language) = lang {
        payload["language"] = json!(language);
    }
    json!({ "type": "start", "payload": payload })
}

fn stop_message() -> Value {
    json!({ "type": "stop" })
}

/// 解析配置中的服务基址（去尾部 `/`）；为空则报错。
fn base_url(config: &AsrConfig) -> Result<String, AsrError> {
    let base = config.api_endpoint.trim().trim_end_matches('/');
    if base.is_empty() {
        return Err(AsrError::InvalidConfig(
            "请在配置中填写自建服务基址（endpoint），如 http://host:17003".to_string(),
        ));
    }
    Ok(base.to_string())
}

/// 从基址派生 WebSocket 端点（http→ws / https→wss；已是 ws(s) 则保持）。
fn ws_url(base: &str) -> String {
    let ws_base = if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        base.to_string()
    };
    format!("{ws_base}/ws/v1/asr/qwen")
}

/// 语言：明确时下发 ISO-639-1 码，"auto" 时省略让服务端自动判别。
fn language_hint(language: &str) -> Option<&'static str> {
    match language {
        "zh" => Some("zh"),
        "en" => Some("en"),
        _ => None,
    }
}

/// 将 reqwest 传输错误转换为 `AsrError`。
fn map_reqwest_error(e: reqwest::Error) -> AsrError {
    if e.is_timeout() {
        AsrError::Timeout
    } else {
        AsrError::NetworkError(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_url_derives_scheme_and_path() {
        assert_eq!(
            ws_url("http://host:17003"),
            "ws://host:17003/ws/v1/asr/qwen"
        );
        assert_eq!(
            ws_url("https://asr.example.com"),
            "wss://asr.example.com/ws/v1/asr/qwen"
        );
        // 已是 ws(s):// 则保持 scheme。
        assert_eq!(ws_url("ws://host:17003"), "ws://host:17003/ws/v1/asr/qwen");
    }

    #[test]
    fn base_url_trims_trailing_slash() {
        let cfg = AsrConfig {
            api_endpoint: "http://host:17003/".to_string(),
            ..Default::default()
        };
        assert_eq!(base_url(&cfg).unwrap(), "http://host:17003");
    }

    #[test]
    fn base_url_empty_is_error() {
        let cfg = AsrConfig::default();
        assert!(matches!(base_url(&cfg), Err(AsrError::InvalidConfig(_))));
    }

    #[test]
    fn start_message_omits_language_for_auto() {
        let msg = start_message(None);
        assert_eq!(msg["payload"]["language"], Value::Null);
        assert_eq!(msg["payload"]["format"], "pcm");
        let zh = start_message(Some("zh"));
        assert_eq!(zh["payload"]["language"], "zh");
    }
}
