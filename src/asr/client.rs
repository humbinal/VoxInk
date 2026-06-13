//! 共享 HTTP 客户端封装（任务 4.2）。
//!
//! 超时 120s（§4.2.2）。TLS 由 rustls 提供，仅支持 TLS 1.2/1.3，天然满足 §5.3 的
//! "TLS 1.2+" 要求，故无需显式设置最小版本。

use std::time::Duration;

use super::error::AsrError;

/// 构建一个配置了超时的共享 `reqwest::Client`。
pub fn build_http_client() -> Result<reqwest::Client, AsrError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| AsrError::NetworkError(e.to_string()))
}
