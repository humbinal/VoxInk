//! 阿里云 OSS 最小客户端（V1 签名）—— M4 混合：filetrans 大文件需先上传 OSS。
//!
//! 上传为私有对象，再生成**预签名 GET URL** 供 DashScope 拉取（避免对象公开，符合隐私优先）。
//! 仅实现本场景所需的 PUT 上传与 GET 预签名，不是通用 OSS SDK。

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use sha1::Sha1;

use super::error::AsrError;

type HmacSha1 = Hmac<Sha1>;

pub struct OssClient<'a> {
    client: &'a reqwest::Client,
    endpoint: String,
    bucket: String,
    access_key_id: String,
    access_key_secret: String,
}

impl<'a> OssClient<'a> {
    pub fn new(
        client: &'a reqwest::Client,
        endpoint: &str,
        bucket: &str,
        access_key_id: &str,
        access_key_secret: &str,
    ) -> Self {
        Self {
            client,
            endpoint: endpoint.trim().to_string(),
            bucket: bucket.trim().to_string(),
            access_key_id: access_key_id.trim().to_string(),
            access_key_secret: access_key_secret.trim().to_string(),
        }
    }

    fn host(&self) -> String {
        format!("{}.{}", self.bucket, self.endpoint)
    }

    fn sign(&self, string_to_sign: &str) -> String {
        let mut mac = HmacSha1::new_from_slice(self.access_key_secret.as_bytes())
            .expect("HMAC 接受任意长度密钥");
        mac.update(string_to_sign.as_bytes());
        BASE64.encode(mac.finalize().into_bytes())
    }

    /// PUT 上传对象（私有 ACL）。
    pub async fn put_object(
        &self,
        key: &str,
        content_type: &str,
        body: Vec<u8>,
    ) -> Result<(), AsrError> {
        let date = Utc::now()
            .format("%a, %d %b %Y %H:%M:%S GMT")
            .to_string();
        let resource = format!("/{}/{}", self.bucket, key);
        // V1: VERB\nContent-MD5\nContent-Type\nDate\nCanonicalizedResource
        let string_to_sign = format!("PUT\n\n{content_type}\n{date}\n{resource}");
        let signature = self.sign(&string_to_sign);
        let url = format!("https://{}/{}", self.host(), key);

        let response = self
            .client
            .put(&url)
            .header("Date", &date)
            .header("Content-Type", content_type)
            .header(
                "Authorization",
                format!("OSS {}:{}", self.access_key_id, signature),
            )
            .body(body)
            .send()
            .await
            .map_err(|e| AsrError::NetworkError(format!("OSS 上传失败: {e}")))?;

        if !response.status().is_success() {
            let code = response.status().as_u16();
            let detail = response.text().await.unwrap_or_default();
            return Err(AsrError::NetworkError(format!(
                "OSS 上传失败 HTTP {code}: {detail}"
            )));
        }
        Ok(())
    }

    /// 生成预签名 GET URL，供 DashScope 在有效期内拉取私有对象。
    pub fn presigned_get_url(&self, key: &str, expires_secs: i64) -> String {
        let expires = Utc::now().timestamp() + expires_secs;
        let resource = format!("/{}/{}", self.bucket, key);
        // V1: GET\nContent-MD5\nContent-Type\nExpires\nCanonicalizedResource
        let string_to_sign = format!("GET\n\n\n{expires}\n{resource}");
        let signature = self.sign(&string_to_sign);
        format!(
            "https://{}/{}?OSSAccessKeyId={}&Expires={}&Signature={}",
            self.host(),
            key,
            encode_query(&self.access_key_id),
            expires,
            encode_query(&signature),
        )
    }
}

/// 最小查询参数百分号编码（覆盖 base64 签名中的 `+ / =` 等字符）。
fn encode_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
