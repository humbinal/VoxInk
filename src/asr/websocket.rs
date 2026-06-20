//! WebSocket 客户端封装（M6，任务 6.1）。
//!
//! 仅负责"带鉴权头连接"。心跳由 tokio-tungstenite 自动回应 ping/pong；重连与协议
//! 由流式后端（bailian_streaming）处理。TLS 用 rustls（纯 Rust，无 OpenSSL）。

use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

use super::error::AsrError;

pub type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// 以 `Authorization: Bearer <key>` 连接 DashScope 实时识别 WebSocket。
pub async fn connect(url: &str, api_key: &str) -> Result<WsStream, AsrError> {
    let mut request = url
        .into_client_request()
        .map_err(|e| AsrError::WebSocketError(format!("构造 WS 请求失败: {e}")))?;

    let bearer = format!("Bearer {api_key}");
    let headers = request.headers_mut();
    headers.insert(
        "Authorization",
        HeaderValue::from_str(&bearer)
            .map_err(|e| AsrError::WebSocketError(format!("无效鉴权头: {e}")))?,
    );
    headers.insert(
        "X-DashScope-DataInspection",
        HeaderValue::from_static("enable"),
    );

    handshake(request).await
}

/// 非 DashScope 的 WebSocket 连接（自建服务用）。仅当 `api_key` 非空时附加
/// `X-NLS-Token: <key>` 头；不发送任何 DashScope 专属头（区别于 [`connect`]）。
///
/// 注意：自建 qwen3-asr 服务的 WS 鉴权只认 `X-NLS-Token` 头或 `token` 查询参数，
/// **不读 `Authorization: Bearer`**（见服务端 `core/security.py::extract_websocket_token`）。
/// 故此处用 `X-NLS-Token` 而非 Bearer，与 HTTP 侧（reqwest `bearer_auth`）有意不同。
pub async fn connect_plain(url: &str, api_key: &str) -> Result<WsStream, AsrError> {
    let mut request = url
        .into_client_request()
        .map_err(|e| AsrError::WebSocketError(format!("构造 WS 请求失败: {e}")))?;

    if !api_key.is_empty() {
        request.headers_mut().insert(
            "X-NLS-Token",
            HeaderValue::from_str(api_key)
                .map_err(|e| AsrError::WebSocketError(format!("无效鉴权头: {e}")))?,
        );
    }

    handshake(request).await
}

/// 执行握手，把 401/403 映射为 [`AsrError::AuthError`]（不重试）。
async fn handshake(
    request: tokio_tungstenite::tungstenite::handshake::client::Request,
) -> Result<WsStream, AsrError> {
    let (stream, _resp) = match connect_async(request).await {
        Ok(ok) => ok,
        // 握手返回 401/403 → 鉴权失败（API Key 无效），映射为 AuthError 不重试。
        Err(tokio_tungstenite::tungstenite::Error::Http(resp))
            if resp.status().as_u16() == 401 || resp.status().as_u16() == 403 =>
        {
            return Err(AsrError::AuthError);
        }
        Err(e) => return Err(AsrError::WebSocketError(format!("WS 连接失败: {e}"))),
    };
    Ok(stream)
}
