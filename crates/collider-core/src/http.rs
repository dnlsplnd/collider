//! HTTP client with retry and exponential backoff.

use std::time::Duration;

use bytes::Bytes;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, RANGE};
use url::Url;

use crate::error::{Error, Result};

#[derive(Clone)]
pub struct Client {
    inner: reqwest::Client,
    retries: u32,
}

impl Client {
    /// `headers` are sent with every request (e.g. `Referer`, `Cookie`, `User-Agent`).
    pub fn new(headers: &[(String, String)], retries: u32, timeout: Duration) -> Result<Self> {
        let mut map = HeaderMap::new();
        for (k, v) in headers {
            let name = HeaderName::from_bytes(k.trim().as_bytes())
                .map_err(|e| Error::Unsupported(format!("bad header name '{k}': {e}")))?;
            let value = HeaderValue::from_str(v.trim())
                .map_err(|e| Error::Unsupported(format!("bad header value for '{k}': {e}")))?;
            map.insert(name, value);
        }
        let inner = reqwest::Client::builder()
            .user_agent(concat!("collider/", env!("CARGO_PKG_VERSION")))
            .default_headers(map)
            .timeout(timeout)
            .connect_timeout(Duration::from_secs(15))
            .pool_max_idle_per_host(64)
            .build()?;
        Ok(Self { inner, retries })
    }

    /// GET a resource, optionally a byte range given as `(offset, length)`.
    pub async fn get_bytes(&self, url: &Url, range: Option<(u64, u64)>) -> Result<Bytes> {
        let mut attempt = 0;
        loop {
            match self.try_get(url, range).await {
                Ok(b) => return Ok(b),
                Err(e) if attempt < self.retries && is_retryable(&e) => {
                    let delay = backoff(attempt);
                    tracing::warn!(%url, attempt = attempt + 1, ?delay, error = %e, "retrying");
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn try_get(&self, url: &Url, range: Option<(u64, u64)>) -> Result<Bytes> {
        let mut req = self.inner.get(url.clone());
        if let Some((offset, len)) = range.filter(|(_, len)| *len > 0) {
            req = req.header(RANGE, format!("bytes={}-{}", offset, offset + len - 1));
        }
        let resp = req.send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(Error::Status {
                status: status.as_u16(),
                url: url.to_string(),
            });
        }
        let body = resp.bytes().await?;
        match range {
            // Some servers ignore Range and answer 200 with the whole resource: slice locally.
            Some((offset, len)) if status != reqwest::StatusCode::PARTIAL_CONTENT && len > 0 => {
                let (start, end) = (offset as usize, (offset + len) as usize);
                if body.len() < end {
                    return Err(Error::Status {
                        status: status.as_u16(),
                        url: format!(
                            "{url} (range {offset}+{len} not honoured, got {} bytes)",
                            body.len()
                        ),
                    });
                }
                tracing::debug!(%url, "server ignored Range header; slicing response");
                Ok(body.slice(start..end))
            }
            _ => Ok(body),
        }
    }
}

fn is_retryable(e: &Error) -> bool {
    match e {
        Error::Http(_) => true,
        Error::Status { status, .. } => matches!(*status, 408 | 425 | 429) || *status >= 500,
        _ => false,
    }
}

fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(250u64.saturating_mul(1 << attempt.min(6))).min(Duration::from_secs(10))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff(0), Duration::from_millis(250));
        assert_eq!(backoff(2), Duration::from_millis(1000));
        assert_eq!(backoff(30), Duration::from_secs(10));
    }

    #[test]
    fn retry_policy() {
        assert!(is_retryable(&Error::Status {
            status: 503,
            url: String::new()
        }));
        assert!(is_retryable(&Error::Status {
            status: 429,
            url: String::new()
        }));
        assert!(!is_retryable(&Error::Status {
            status: 404,
            url: String::new()
        }));
    }
}
