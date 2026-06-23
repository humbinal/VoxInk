//! 阿里云百炼大文件离线后端：qwen3-asr-flash-filetrans（异步文件转写）。
//!
//! 该模型只接受**公网可访问的文件 URL**、不支持本地/base64 上传（官方文档明确）。
//! 因此流程为：本地 WAV → 上传到用户 OSS（私有）→ 预签名 GET URL → 提交异步任务 →
//! 轮询任务状态 → 拉取结果 JSON 提取文本。支持超大/超长音频（文档称可达 12 小时）。
//!
//! ⚠️ OSS 凭证（endpoint/bucket/ak/sk）M4 阶段经环境变量提供（见 build_asr_config），
//! M11 设置面板上线后改由加密配置提供。上传的对象不会自动删除，建议在 OSS 配置生命周期规则。

use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;

use crate::asr::client::build_http_client;
use crate::asr::config::AsrConfig;
use crate::asr::error::AsrError;
use crate::asr::oss::OssClient;
use crate::asr::traits::{AsrBackend, OfflineAudio, StreamingResult};

const SUBMIT_URL: &str = "https://dashscope.aliyuncs.com/api/v1/services/audio/asr/transcription";
const TASK_URL_PREFIX: &str = "https://dashscope.aliyuncs.com/api/v1/tasks/";
const MODEL: &str = "qwen3-asr-flash-filetrans";
const POLL_INTERVAL: Duration = Duration::from_secs(3);
const MAX_POLLS: usize = 200; // 约 10 分钟轮询上限
const URL_EXPIRE_SECS: i64 = 3600; // 预签名 URL 有效期

pub struct BailianFiletransBackend {
    client: reqwest::Client,
}

impl BailianFiletransBackend {
    pub fn new() -> Self {
        let client = build_http_client().unwrap_or_else(|_| reqwest::Client::new());
        Self { client }
    }
}

impl Default for BailianFiletransBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AsrBackend for BailianFiletransBackend {
    fn backend_id(&self) -> &str {
        "aliyun_bailian_filetrans"
    }

    fn display_name(&self) -> &str {
        "阿里云百炼（离线大文件）"
    }

    fn supports_streaming(&self) -> bool {
        false
    }

    fn supports_offline(&self) -> bool {
        true
    }

    async fn validate_config(&self, config: &AsrConfig) -> Result<(), AsrError> {
        if config.api_key.trim().is_empty() {
            return Err(AsrError::AuthError);
        }
        ensure_oss_configured(config)?;
        Ok(())
    }

    async fn transcribe_streaming(
        &self,
        _config: &AsrConfig,
        _audio_rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
        _result_tx: tokio::sync::mpsc::Sender<StreamingResult>,
    ) -> Result<(), AsrError> {
        Err(AsrError::InvalidConfig(
            "大文件离线后端不支持实时流式识别".to_string(),
        ))
    }

    async fn transcribe_offline(
        &self,
        config: &AsrConfig,
        audio: OfflineAudio,
    ) -> Result<String, AsrError> {
        let OfflineAudio { data, format } = audio;
        if data.is_empty() {
            return Err(AsrError::EmptyAudio);
        }
        let api_key = config.api_key.trim();
        if api_key.is_empty() {
            return Err(AsrError::AuthError);
        }
        ensure_oss_configured(config)?;

        // 1) 上传到 OSS（私有）。
        let oss = OssClient::new(
            &self.client,
            &config.oss_endpoint,
            &config.oss_bucket,
            &config.oss_access_key_id,
            &config.oss_access_key_secret,
        );
        let key = format!(
            "voxink/{}.{}",
            Utc::now().format("%Y%m%d_%H%M%S_%6f"),
            format.extension()
        );
        tracing::info!(%key, bytes = data.len(), "filetrans：上传录音到 OSS");
        oss.put_object(&key, format.mime(), data).await?;

        // 2) 预签名 GET URL 供 DashScope 拉取。
        let file_url = oss.presigned_get_url(&key, URL_EXPIRE_SECS);

        // 3) 提交异步转写任务。
        let task_id = self.submit_task(api_key, &file_url).await?;
        tracing::info!(%task_id, "filetrans：任务已提交，开始轮询");

        // 4) 轮询直至完成。
        let transcription_url = self.poll_task(api_key, &task_id).await?;

        // 5) 拉取结果 JSON 提取文本。
        let text = self.fetch_transcription(&transcription_url).await?;
        if text.trim().is_empty() {
            return Err(AsrError::EmptyResult);
        }
        Ok(text.trim().to_string())
    }
}

impl BailianFiletransBackend {
    async fn submit_task(&self, api_key: &str, file_url: &str) -> Result<String, AsrError> {
        let body = serde_json::json!({
            "model": MODEL,
            "input": { "file_url": file_url },
            "parameters": { "channel_id": [0], "enable_itn": false }
        });

        let response = self
            .client
            .post(SUBMIT_URL)
            .bearer_auth(api_key)
            .header("X-DashScope-Async", "enable")
            .json(&body)
            .send()
            .await
            .map_err(map_reqwest_error)?;

        let value = check_and_parse(response).await?;
        value["output"]["task_id"]
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| AsrError::NetworkError(format!("提交任务未返回 task_id: {value}")))
    }

    /// 轮询任务，成功返回 transcription_url。
    async fn poll_task(&self, api_key: &str, task_id: &str) -> Result<String, AsrError> {
        let url = format!("{TASK_URL_PREFIX}{task_id}");
        for _ in 0..MAX_POLLS {
            tokio::time::sleep(POLL_INTERVAL).await;
            let response = self
                .client
                .get(&url)
                .bearer_auth(api_key)
                .send()
                .await
                .map_err(map_reqwest_error)?;
            let value = check_and_parse(response).await?;
            let status = value["output"]["task_status"].as_str().unwrap_or_default();
            match status {
                "SUCCEEDED" => {
                    return extract_transcription_url(&value).ok_or_else(|| {
                        AsrError::NetworkError(format!("任务成功但缺少 transcription_url: {value}"))
                    });
                }
                "FAILED" => {
                    let msg = value["output"]["message"]
                        .as_str()
                        .or_else(|| value["output"]["code"].as_str())
                        .unwrap_or("未知错误");
                    return Err(AsrError::NetworkError(format!("转写任务失败: {msg}")));
                }
                // PENDING / RUNNING：继续轮询。
                _ => continue,
            }
        }
        Err(AsrError::Timeout)
    }

    /// 拉取结果 JSON（公网签名 URL，无需鉴权），拼接所有声道文本。
    async fn fetch_transcription(&self, transcription_url: &str) -> Result<String, AsrError> {
        let response = self
            .client
            .get(transcription_url)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        if !response.status().is_success() {
            let code = response.status().as_u16();
            return Err(AsrError::NetworkError(format!(
                "拉取转写结果失败 HTTP {code}"
            )));
        }
        let value: Value = response
            .json()
            .await
            .map_err(|e| AsrError::NetworkError(format!("解析转写结果失败: {e}")))?;

        // 结果结构：{"transcripts":[{"text":"...","channel_id":0}, ...]}
        let text = value["transcripts"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|t| t["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        Ok(text)
    }
}

/// 检查 OSS 四要素均已配置。
fn ensure_oss_configured(config: &AsrConfig) -> Result<(), AsrError> {
    if config.oss_endpoint.trim().is_empty()
        || config.oss_bucket.trim().is_empty()
        || config.oss_access_key_id.trim().is_empty()
        || config.oss_access_key_secret.trim().is_empty()
    {
        return Err(AsrError::InvalidConfig(
            "大文件转写需配置 OSS（endpoint/bucket/access_key_id/access_key_secret）".to_string(),
        ));
    }
    Ok(())
}

/// 在 SUCCEEDED 响应中尽量稳健地取 transcription_url（兼容 results[] / result / 顶层）。
fn extract_transcription_url(value: &Value) -> Option<String> {
    let output = &value["output"];
    output["results"][0]["transcription_url"]
        .as_str()
        .or_else(|| output["result"]["transcription_url"].as_str())
        .or_else(|| output["transcription_url"].as_str())
        .map(|s| s.to_string())
}

/// 校验 HTTP 状态并解析 JSON（鉴权/配额错误映射为对应 AsrError）。
async fn check_and_parse(response: reqwest::Response) -> Result<Value, AsrError> {
    let status = response.status();
    match status.as_u16() {
        401 | 403 => return Err(AsrError::AuthError),
        429 => {
            return Err(AsrError::QuotaExceeded(
                response.text().await.unwrap_or_default(),
            ));
        }
        code if !(200..300).contains(&code) => {
            let detail = response.text().await.unwrap_or_default();
            return Err(AsrError::NetworkError(format!("HTTP {code}: {detail}")));
        }
        _ => {}
    }
    response
        .json::<Value>()
        .await
        .map_err(|e| AsrError::NetworkError(format!("解析响应失败: {e}")))
}

fn map_reqwest_error(e: reqwest::Error) -> AsrError {
    if e.is_timeout() {
        AsrError::Timeout
    } else {
        AsrError::NetworkError(e.to_string())
    }
}
