use anyhow::{Context, Result, bail};
use chrono::Utc;
use reqwest::header::HeaderMap;
use serde_json::Value;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{debug, info, warn};

const BASE_URL: &str = "https://api.tumblr.com/v2";
const REQUEST_DELAY: Duration = Duration::from_millis(1100);

pub struct TumblrClient {
    consumer_key: String,
    consumer_secret: String,
    token: String,
    token_secret: String,
    http: reqwest::Client,
    raw_dir: PathBuf,
}

struct RateLimit {
    hourly_remaining: Option<u32>,
    hourly_reset_secs: Option<u32>,
    daily_remaining: Option<u32>,
    daily_reset_secs: Option<u32>,
}

impl TumblrClient {
    pub fn new(
        consumer_key: String,
        consumer_secret: String,
        token: String,
        token_secret: String,
        raw_dir: PathBuf,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("notesdaq/0.1")
            .build()?;

        std::fs::create_dir_all(&raw_dir)?;

        Ok(Self {
            consumer_key,
            consumer_secret,
            token,
            token_secret,
            http,
            raw_dir,
        })
    }

    /// Make an OAuth1-signed GET request to the Tumblr API.
    /// Respects rate limits and saves raw responses.
    async fn get(&self, path: &str, params: &[(&str, &str)]) -> Result<Value> {
        let url = format!("{BASE_URL}{path}");

        // Build OAuth1 authorization header
        let auth_header = self.build_oauth_header("GET", &url, params);

        // Build the request
        let mut req = self.http.get(&url).header("Authorization", &auth_header);
        for (k, v) in params {
            req = req.query(&[(k, v)]);
        }

        // Flat delay between requests
        tokio::time::sleep(REQUEST_DELAY).await;

        debug!("GET {url}");
        let resp = req.send().await.context("HTTP request failed")?;

        let status = resp.status();
        let headers = resp.headers().clone();
        let body = resp.text().await.context("Failed to read response body")?;

        // Save raw response
        self.save_raw_response(path, params, status.as_u16(), &headers, &body)
            .await;

        // Check rate limits
        let rate_limit = parse_rate_limits(&headers);
        if let Some(remaining) = rate_limit.hourly_remaining {
            if remaining < 50 {
                let reset = rate_limit.hourly_reset_secs.unwrap_or(3600);
                warn!(
                    "Rate limit nearly exhausted: {remaining} calls remaining, resets in {reset}s"
                );
                if remaining < 10 {
                    info!("Pausing for {reset}s to respect rate limits...");
                    tokio::time::sleep(Duration::from_secs(reset as u64)).await;
                }
            }
        }
        if let Some(remaining) = rate_limit.daily_remaining {
            if remaining < 100 {
                warn!("Daily rate limit nearly exhausted: {remaining} calls remaining");
                if remaining < 10 {
                    let reset = rate_limit.daily_reset_secs.unwrap_or(86400);
                    info!("Daily limit critical. Pausing for {reset}s...");
                    tokio::time::sleep(Duration::from_secs(reset as u64)).await;
                }
            }
        }

        if !status.is_success() {
            bail!("API returned HTTP {status}: {}", &body[..body.len().min(500)]);
        }

        let parsed: Value = serde_json::from_str(&body).context("Failed to parse JSON")?;
        Ok(parsed.get("response").cloned().unwrap_or(parsed))
    }

    pub async fn get_blog_info(&self, blog: &str) -> Result<Value> {
        let data = self.get(&format!("/blog/{blog}/info"), &[]).await?;
        Ok(data.get("blog").cloned().unwrap_or(data))
    }

    pub async fn get_blog_posts(&self, blog: &str, offset: u32, limit: u32) -> Result<Value> {
        let offset_str = offset.to_string();
        let limit_str = limit.to_string();
        self.get(
            &format!("/blog/{blog}/posts"),
            &[
                ("offset", &offset_str),
                ("limit", &limit_str),
                ("npf", "true"),
                ("reblog_info", "true"),
                ("notes_info", "true"),
            ],
        )
        .await
    }

    pub async fn get_notes(
        &self,
        blog: &str,
        post_id: i64,
        before_timestamp: Option<i64>,
    ) -> Result<Value> {
        let post_id_str = post_id.to_string();
        let ts_str;
        let mut params = vec![("id", post_id_str.as_str()), ("mode", "all")];
        if let Some(ts) = before_timestamp {
            ts_str = ts.to_string();
            params.push(("before_timestamp", &ts_str));
        }
        self.get(&format!("/blog/{blog}/notes"), &params).await
    }

    /// Build an OAuth1 Authorization header using HMAC-SHA1.
    fn build_oauth_header(&self, method: &str, url: &str, params: &[(&str, &str)]) -> String {
        use std::collections::BTreeMap;

        let timestamp = Utc::now().timestamp().to_string();
        let nonce = format!("{:x}{:x}", rand_u64(), rand_u64());

        let mut oauth_params = BTreeMap::new();
        oauth_params.insert("oauth_consumer_key", self.consumer_key.as_str());
        oauth_params.insert("oauth_nonce", &nonce);
        oauth_params.insert("oauth_signature_method", "HMAC-SHA1");
        oauth_params.insert("oauth_timestamp", &timestamp);
        oauth_params.insert("oauth_token", self.token.as_str());
        oauth_params.insert("oauth_version", "1.0");

        // Combine OAuth params and query params for signature base
        let mut all_params: BTreeMap<&str, &str> = oauth_params.clone();
        for (k, v) in params {
            all_params.insert(k, v);
        }

        let param_string: String = all_params
            .iter()
            .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
            .collect::<Vec<_>>()
            .join("&");

        let base_string = format!(
            "{}&{}&{}",
            method.to_uppercase(),
            percent_encode(url),
            percent_encode(&param_string)
        );

        let signing_key = format!(
            "{}&{}",
            percent_encode(&self.consumer_secret),
            percent_encode(&self.token_secret)
        );

        let signature = hmac_sha1(signing_key.as_bytes(), base_string.as_bytes());
        let sig_b64 = base64_encode(&signature);

        format!(
            "OAuth oauth_consumer_key=\"{}\", oauth_nonce=\"{}\", oauth_signature=\"{}\", oauth_signature_method=\"HMAC-SHA1\", oauth_timestamp=\"{}\", oauth_token=\"{}\", oauth_version=\"1.0\"",
            percent_encode(&self.consumer_key),
            percent_encode(&nonce),
            percent_encode(&sig_b64),
            percent_encode(&timestamp),
            percent_encode(&self.token),
        )
    }

    async fn save_raw_response(
        &self,
        path: &str,
        params: &[(&str, &str)],
        status: u16,
        headers: &HeaderMap,
        body: &str,
    ) {
        let now = Utc::now();
        let date_dir = now.format("%Y-%m-%d").to_string();
        let dir = self.raw_dir.join(&date_dir);

        if let Err(e) = std::fs::create_dir_all(&dir) {
            warn!("Failed to create raw response dir: {e}");
            return;
        }

        let filename = format!(
            "{}_{}.json",
            now.format("%H%M%S_%3f"),
            path.replace('/', "_").trim_matches('_')
        );

        let rate_headers: serde_json::Map<String, Value> = headers
            .iter()
            .filter(|(k, _)| {
                let name = k.as_str().to_lowercase();
                name.contains("rate") || name.contains("limit")
            })
            .filter_map(|(k, v)| {
                v.to_str()
                    .ok()
                    .map(|v| (k.as_str().to_string(), Value::String(v.to_string())))
            })
            .collect();

        let envelope = serde_json::json!({
            "request": {
                "method": "GET",
                "path": path,
                "params": params.iter().copied().collect::<std::collections::HashMap<_, _>>(),
                "timestamp": now.to_rfc3339(),
            },
            "response": {
                "status": status,
                "rate_limit_headers": rate_headers,
                "body": serde_json::from_str::<Value>(body).unwrap_or(Value::String(body.to_string())),
            }
        });

        let filepath = dir.join(&filename);
        if let Err(e) = std::fs::write(&filepath, serde_json::to_string_pretty(&envelope).unwrap_or_default()) {
            warn!("Failed to save raw response to {}: {e}", filepath.display());
        }
    }
}

fn parse_rate_limits(headers: &HeaderMap) -> RateLimit {
    let get = |name: &str| -> Option<u32> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
    };

    RateLimit {
        hourly_remaining: get("x-ratelimit-perhour-remaining"),
        hourly_reset_secs: get("x-ratelimit-perhour-reset"),
        daily_remaining: get("x-ratelimit-perday-remaining"),
        daily_reset_secs: get("x-ratelimit-perday-reset"),
    }
}

// ── Crypto helpers (OAuth1 HMAC-SHA1, no external crate needed) ────

fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    const BLOCK_SIZE: usize = 64;

    let mut key_block = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        let hash = sha1(key);
        key_block[..20].copy_from_slice(&hash);
    } else {
        key_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] ^= key_block[i];
        opad[i] ^= key_block[i];
    }

    let mut inner = Vec::with_capacity(BLOCK_SIZE + message.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(message);
    let inner_hash = sha1(&inner);

    let mut outer = Vec::with_capacity(BLOCK_SIZE + 20);
    outer.extend_from_slice(&opad);
    outer.extend_from_slice(&inner_hash);
    sha1(&outer)
}

fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h0: u32 = 0x67452301;
    let mut h1: u32 = 0xEFCDAB89;
    let mut h2: u32 = 0x98BADCFE;
    let mut h3: u32 = 0x10325476;
    let mut h4: u32 = 0xC3D2E1F0;

    let bit_len = (data.len() as u64) * 8;
    let mut padded = data.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let (mut a, mut b, mut c, mut d, mut e) = (h0, h1, h2, h3, h4);

        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };

            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }

        h0 = h0.wrapping_add(a);
        h1 = h1.wrapping_add(b);
        h2 = h2.wrapping_add(c);
        h3 = h3.wrapping_add(d);
        h4 = h4.wrapping_add(e);
    }

    let mut result = [0u8; 20];
    result[0..4].copy_from_slice(&h0.to_be_bytes());
    result[4..8].copy_from_slice(&h1.to_be_bytes());
    result[8..12].copy_from_slice(&h2.to_be_bytes());
    result[12..16].copy_from_slice(&h3.to_be_bytes());
    result[16..20].copy_from_slice(&h4.to_be_bytes());
    result
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

fn percent_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                result.push(b as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", b));
            }
        }
    }
    result
}

/// Simple PRNG for nonce generation (doesn't need to be cryptographic).
fn rand_u64() -> u64 {
    use std::time::SystemTime;
    let seed = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    // xorshift64
    let mut x = seed ^ 0x1234567890abcdef;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    x
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percent_encode() {
        assert_eq!(percent_encode("hello world"), "hello%20world");
        assert_eq!(percent_encode("a=b&c=d"), "a%3Db%26c%3Dd");
        assert_eq!(percent_encode("foo-bar_baz.qux~"), "foo-bar_baz.qux~");
    }

    #[test]
    fn test_base64() {
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"a"), "YQ==");
    }

    #[test]
    fn test_sha1() {
        // SHA1 of empty string
        let result = sha1(b"");
        let hex: String = result.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(hex, "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    #[test]
    fn test_hmac_sha1() {
        // RFC 2202 test vector 1
        let key = [0x0b; 20];
        let data = b"Hi There";
        let result = hmac_sha1(&key, data);
        let hex: String = result.iter().map(|b| format!("{:02x}", b)).collect();
        assert_eq!(hex, "b617318655057264e28bc0b6fb378c8ef146be00");
    }
}
