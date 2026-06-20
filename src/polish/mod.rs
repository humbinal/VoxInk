//! 转录后 AI 润色 —— OpenAI 兼容 `chat/completions` 文本润色（IDEAS）。
//!
//! 纯 Rust：复用 reqwest（rustls-tls）+ serde_json，与 ASR 离线后端同款 HTTP 模式
//! （见 `asr/backends/bailian_offline.rs`），仅把"音频输入"换成"system 提示词 + 文本"。
//! 主流厂商（OpenAI/DeepSeek/Moonshot/百炼 compatible-mode/智谱…）均提供 OpenAI 兼容接口，
//! 故单一客户端 + `base_url` 即可覆盖。

use std::time::Duration;

use serde::Deserialize;

/// 润色错误（映射 HTTP 状态与传输错误，供 UI 友好提示）。
#[derive(Debug)]
pub enum PolishError {
    /// 401：API Key 无效。
    Auth,
    /// 403：模型无权限/未开通。
    AccessDenied(String),
    /// 429：限流/额度。
    Quota(String),
    /// 超时。
    Timeout,
    /// 其它网络/HTTP 错误。
    Network(String),
    /// 模型返回空结果。
    Empty,
}

impl std::fmt::Display for PolishError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolishError::Auth => write!(f, "API Key 无效，请检查后重试"),
            PolishError::AccessDenied(d) => write!(f, "模型无权限或未开通：{d}"),
            PolishError::Quota(d) => write!(f, "请求过于频繁或额度不足：{d}"),
            PolishError::Timeout => write!(f, "请求超时，请稍后重试"),
            PolishError::Network(d) => write!(f, "网络错误：{d}"),
            PolishError::Empty => write!(f, "模型未返回内容"),
        }
    }
}

/// 一次润色请求所需的全部参数（运行期由 `PolishConfig` + 选中模板组装）。
pub struct PolishRequest {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub temperature: f32,
    pub system_prompt: String,
    pub text: String,
}

/// 流式润色事件：增量文本 / 完成 / 出错。
pub enum PolishEvent {
    Delta(String),
    Done,
    Error(PolishError),
}

/// OpenAI 兼容流式分片：`choices[0].delta.content`。
#[derive(Deserialize)]
struct StreamChunk {
    choices: Vec<StreamChoice>,
}

#[derive(Deserialize)]
struct StreamChoice {
    delta: StreamDelta,
}

#[derive(Deserialize)]
struct StreamDelta {
    content: Option<String>,
}

/// 构建带超时的 reqwest 客户端（润色可能较慢，给 120s）。
pub fn build_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// 由 `base_url` 推导 chat/completions 端点（容错末尾斜杠与已带路径）。
fn chat_endpoint(base_url: &str) -> String {
    let b = base_url.trim().trim_end_matches('/');
    if b.ends_with("/chat/completions") {
        b.to_string()
    } else {
        format!("{b}/chat/completions")
    }
}

/// 流式润色：增量把 `delta.content` 通过 `tx` 送出（打字机效果），结束发 `Done`，
/// 出错发 `Error`。调用方（已校验过配置非空）只需消费事件并更新 UI。
pub async fn polish_stream(
    client: &reqwest::Client,
    req: PolishRequest,
    tx: tokio::sync::mpsc::Sender<PolishEvent>,
) {
    let body = serde_json::json!({
        "model": req.model,
        "temperature": req.temperature,
        "stream": true,
        "messages": [
            { "role": "system", "content": req.system_prompt },
            { "role": "user", "content": req.text },
        ],
    });

    let response = match client
        .post(chat_endpoint(&req.base_url))
        .bearer_auth(req.api_key.trim())
        .json(&body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(PolishEvent::Error(map_reqwest_error(e))).await;
            return;
        }
    };

    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        let detail = response.text().await.unwrap_or_default();
        let err = match status {
            401 => PolishError::Auth,
            403 => PolishError::AccessDenied(detail),
            429 => PolishError::Quota(detail),
            code => PolishError::Network(format!("HTTP {code}: {detail}")),
        };
        let _ = tx.send(PolishEvent::Error(err)).await;
        return;
    }

    // 逐分片读取 SSE：行以 `data: ` 开头，载荷为 JSON 或 `[DONE]`；跨分片的半行用 buffer 暂存。
    let mut response = response;
    let mut buffer = String::new();
    let mut emitted_any = false;
    loop {
        match response.chunk().await {
            Ok(Some(bytes)) => {
                buffer.push_str(&String::from_utf8_lossy(&bytes));
                while let Some(pos) = buffer.find('\n') {
                    let line: String = buffer.drain(..=pos).collect();
                    let line = line.trim();
                    let Some(payload) = line.strip_prefix("data:") else {
                        continue;
                    };
                    let payload = payload.trim();
                    if payload.is_empty() || payload == "[DONE]" {
                        continue;
                    }
                    if let Ok(chunk) = serde_json::from_str::<StreamChunk>(payload)
                        && let Some(delta) = chunk.choices.into_iter().next().and_then(|c| c.delta.content)
                        && !delta.is_empty()
                    {
                        emitted_any = true;
                        if tx.send(PolishEvent::Delta(delta)).await.is_err() {
                            return; // 接收端已关闭（面板关闭/取消）。
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(e) => {
                let _ = tx.send(PolishEvent::Error(map_reqwest_error(e))).await;
                return;
            }
        }
    }

    if emitted_any {
        let _ = tx.send(PolishEvent::Done).await;
    } else {
        let _ = tx.send(PolishEvent::Error(PolishError::Empty)).await;
    }
}

fn map_reqwest_error(e: reqwest::Error) -> PolishError {
    if e.is_timeout() {
        PolishError::Timeout
    } else {
        PolishError::Network(e.to_string())
    }
}

/// 由 base_url 推断 API Key 的兜底环境变量名（config 未填 key 时回退读取）。
///
/// 不同厂商约定俗成的环境变量名不同；按 base_url 主机关键字匹配，未知则回退 `OPENAI_API_KEY`。
pub fn api_key_env_var(base_url: &str) -> &'static str {
    let b = base_url.to_ascii_lowercase();
    if b.contains("deepseek") {
        "DEEPSEEK_API_KEY"
    } else if b.contains("moonshot") {
        "MOONSHOT_API_KEY"
    } else if b.contains("dashscope") || b.contains("aliyuncs") {
        "DASHSCOPE_API_KEY"
    } else if b.contains("bigmodel") {
        "ZHIPUAI_API_KEY"
    } else if b.contains("siliconflow") {
        "SILICONFLOW_API_KEY"
    } else {
        // OpenAI 及未知/自定义兼容服务默认走 OPENAI_API_KEY。
        "OPENAI_API_KEY"
    }
}

/// OpenAI 兼容厂商预设（名称, base_url）。供设置面板下拉快速填充 base_url。
pub const PROVIDER_PRESETS: &[(&str, &str)] = &[
    ("OpenAI", "https://api.openai.com/v1"),
    ("DeepSeek", "https://api.deepseek.com/v1"),
    ("Moonshot", "https://api.moonshot.cn/v1"),
    ("百炼", "https://dashscope.aliyuncs.com/compatible-mode/v1"),
    ("智谱", "https://open.bigmodel.cn/api/paas/v4"),
    ("硅基流动", "https://api.siliconflow.cn/v1"),
];
