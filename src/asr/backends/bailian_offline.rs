//! 阿里云百炼离线后端（任务 4.3）。
//!
//! ⚠️ 接口现实与 PRD 附录 B 的差异（M4 落地修订，详见 PRD 更新）：
//! 附录 B 所述 `/api/v1/services/audio/asr/transcription`（multipart 上传）实际是
//! DashScope **异步、需公网 file_urls** 的录音文件识别接口，无法对本地 WAV 字节做同步转写，
//! 也不契合 §2.2 的 `transcribe_offline(audio_data: Vec<u8>) -> String`。
//!
//! 因此本后端改用 **Qwen3-ASR-Flash** 的 OpenAI 兼容同步接口：直接提交 base64 本地音频，
//! 同步返回文本，契合契约且无需公网 URL。单次音频上限约 10MB（约 3-4 分钟）。

use async_trait::async_trait;
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::Deserialize;

use crate::asr::client::build_http_client;
use crate::asr::config::AsrConfig;
use crate::asr::error::AsrError;
use crate::asr::traits::{AsrBackend, StreamingResult};

const DEFAULT_ENDPOINT: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions";
const MODEL: &str = "qwen3-asr-flash";
/// 同步接口音频上限约 10MB；base64 膨胀约 1.33x，故对原始字节取更保守的 7MB 上限。
const MAX_AUDIO_BYTES: usize = 7 * 1024 * 1024;
/// 16kHz / mono / PCM16 的字节率。
const BYTES_PER_SEC: usize = 16_000 * 2;
/// 由 `MAX_AUDIO_BYTES` 推导的单次录音秒数上限，再留 10% 余量防止边界超限。
/// 约 7MB / 32KB·s⁻¹ ≈ 229s → ~206s（3:26），契合"约 3-4 分钟"。
pub const MAX_RECORDING_SECS: u32 = (MAX_AUDIO_BYTES / BYTES_PER_SEC * 9 / 10) as u32;

pub struct BailianOfflineBackend {
    client: reqwest::Client,
}

impl BailianOfflineBackend {
    pub fn new() -> Self {
        // 客户端构建极少失败；失败时回退到默认客户端以保证后端可用。
        let client = build_http_client().unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

impl Default for BailianOfflineBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// OpenAI 兼容 chat/completions 响应（仅取转写文本所在字段）。
#[derive(Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[derive(Deserialize)]
struct ChoiceMessage {
    content: Option<String>,
}

#[async_trait]
impl AsrBackend for BailianOfflineBackend {
    fn backend_id(&self) -> &str {
        "aliyun_bailian_offline"
    }

    fn display_name(&self) -> &str {
        "阿里云百炼（离线）"
    }

    fn supports_streaming(&self) -> bool {
        false
    }

    fn supports_offline(&self) -> bool {
        true
    }

    fn max_recording_seconds(&self) -> Option<u32> {
        Some(MAX_RECORDING_SECS)
    }

    async fn validate_config(&self, config: &AsrConfig) -> Result<(), AsrError> {
        if config.api_key.trim().is_empty() {
            return Err(AsrError::AuthError);
        }
        Ok(())
    }

    async fn transcribe_streaming(
        &self,
        _config: &AsrConfig,
        _audio_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        _result_tx: tokio::sync::mpsc::Sender<StreamingResult>,
    ) -> Result<(), AsrError> {
        Err(AsrError::InvalidConfig(
            "离线后端不支持实时流式识别".to_string(),
        ))
    }

    async fn transcribe_offline(
        &self,
        config: &AsrConfig,
        audio_data: Vec<u8>,
    ) -> Result<String, AsrError> {
        if audio_data.is_empty() {
            return Err(AsrError::EmptyAudio);
        }
        if audio_data.len() > MAX_AUDIO_BYTES {
            return Err(AsrError::UnsupportedFormat(format!(
                "音频过大（约 {} MB）。离线同步识别上限约 10MB（约 3-4 分钟），请缩短录音时长",
                audio_data.len() / 1024 / 1024
            )));
        }

        let api_key = config.api_key.trim();
        if api_key.is_empty() {
            return Err(AsrError::AuthError);
        }

        // §2.7 仅有单一 api_endpoint（默认是流式 wss URL）；离线后端使用自身默认 HTTPS 端点，
        // 仅当用户显式配置了 https 端点时才覆盖。
        let endpoint = if config.api_endpoint.starts_with("https://") {
            config.api_endpoint.as_str()
        } else {
            DEFAULT_ENDPOINT
        };

        let data_url = format!("data:audio/wav;base64,{}", BASE64.encode(&audio_data));
        let body = serde_json::json!({
            "model": MODEL,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "input_audio",
                    "input_audio": { "data": data_url }
                }]
            }],
            "stream": false
        });

        let response = self
            .client
            .post(endpoint)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest_error)?;

        let status = response.status();
        match status.as_u16() {
            401 => return Err(AsrError::AuthError),
            403 => {
                // 403 多为"模型未开通 / API Key 所属业务空间无该模型权限"，与 401（Key 本身无效）
                // 含义不同。若混为 AuthError 会误导用户去查 API Key，故单独识别 AccessDenied。
                let detail = response.text().await.unwrap_or_default();
                if detail.contains("AccessDenied")
                    || detail.to_lowercase().contains("access denied")
                {
                    return Err(AsrError::InvalidConfig(format!(
                        "模型访问被拒绝：请在百炼控制台开通 {MODEL} 模型，并确认 API Key 所属业务空间有该模型权限。详情: {detail}"
                    )));
                }
                return Err(AsrError::AuthError);
            }
            429 => {
                let detail = response.text().await.unwrap_or_default();
                return Err(AsrError::QuotaExceeded(detail));
            }
            code if !(200..300).contains(&code) => {
                let detail = response.text().await.unwrap_or_default();
                return Err(AsrError::NetworkError(format!("HTTP {code}: {detail}")));
            }
            _ => {}
        }

        let parsed: ChatCompletionResponse = response
            .json()
            .await
            .map_err(|e| AsrError::NetworkError(format!("解析转写响应失败: {e}")))?;

        let text = parsed
            .choices
            .into_iter()
            .next()
            .and_then(|choice| choice.message.content)
            .unwrap_or_default()
            .trim()
            .to_string();

        if text.is_empty() {
            return Err(AsrError::EmptyResult);
        }
        Ok(text)
    }
}

/// 将 reqwest 传输错误转换为 `AsrError`（§2.4：传输库是实现细节）。
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
    fn reports_recording_cap_within_byte_budget() {
        let cap = BailianOfflineBackend::new()
            .max_recording_seconds()
            .expect("离线后端应有硬时长上限");
        // 上限对应的字节数必须留在 MAX_AUDIO_BYTES 之内（含 10% 余量），否则录满仍会上传失败。
        assert!((cap as usize) * BYTES_PER_SEC <= MAX_AUDIO_BYTES);
        // 落在文案宣称的"约 3-4 分钟"区间。
        assert!((180..=240).contains(&cap), "cap = {cap}");
    }
}
