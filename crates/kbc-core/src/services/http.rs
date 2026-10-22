//! Timeout・Response上限・同時実行数を共有するHTTP Client。

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::Duration;

use crate::asset_platform::{AssetPlatform, INDEX_LIMIT, RAW_BASE, split_url};
use reqwest::header::HeaderMap;
use reqwest::header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED};
use reqwest::{Client, Method, Response, StatusCode};
use tokio::sync::Semaphore;

#[derive(Clone, Copy, Debug)]
pub(crate) struct HttpServiceConfig {
    pub(crate) max_concurrency: usize,
    pub(crate) request_timeout: Duration,
    pub(crate) max_response_bytes: usize,
}

pub(crate) struct HttpService {
    client: Client,
    permits: Arc<Semaphore>,
    max_response_bytes: usize,
    assets: AssetPlatform,
}

pub(crate) struct HttpResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: Vec<u8>,
}

pub(crate) enum ConditionalTextResponse {
    NotModified {
        etag: Option<String>,
        last_modified: Option<String>,
    },
    Modified {
        text: String,
        etag: Option<String>,
        last_modified: Option<String>,
    },
}

impl HttpService {
    pub(crate) fn new(config: HttpServiceConfig) -> Result<Self, HttpServiceError> {
        let client = Client::builder()
            .timeout(config.request_timeout)
            .user_agent(concat!(
                "kbc-rakv0-discord-bot-v2/",
                env!("CARGO_PKG_VERSION")
            ))
            .build()
            .map_err(HttpServiceError::Initialization)?;

        Ok(Self {
            client,
            permits: Arc::new(Semaphore::new(config.max_concurrency)),
            max_response_bytes: config.max_response_bytes,
            assets: AssetPlatform::default(),
        })
    }

    pub(crate) async fn get_text(&self, url: &str) -> Result<String, HttpServiceError> {
        self.get_text_inner(url, false)
            .await?
            .ok_or(HttpServiceError::UnexpectedNotFound)
    }

    pub(crate) async fn get_optional_text(
        &self,
        url: &str,
    ) -> Result<Option<String>, HttpServiceError> {
        self.get_text_inner(url, true).await
    }

    pub(crate) async fn get_conditional_text(
        &self,
        url: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
    ) -> Result<ConditionalTextResponse, HttpServiceError> {
        if split_url(url).is_some() {
            let text = self.get_text(url).await?;
            return Ok(ConditionalTextResponse::Modified {
                text,
                etag: None,
                last_modified: None,
            });
        }
        let _permit = self.acquire().await?;
        let mut request = self.client.get(url);
        if let Some(etag) = etag {
            request = request.header(IF_NONE_MATCH, etag);
        }
        if let Some(last_modified) = last_modified {
            request = request.header(IF_MODIFIED_SINCE, last_modified);
        }
        let response = request.send().await.map_err(HttpServiceError::Request)?;
        let response_etag = header_text(&response, ETAG);
        let response_last_modified = header_text(&response, LAST_MODIFIED);
        if response.status() == StatusCode::NOT_MODIFIED {
            return Ok(ConditionalTextResponse::NotModified {
                etag: response_etag,
                last_modified: response_last_modified,
            });
        }
        if !response.status().is_success() {
            return Err(HttpServiceError::Status(response.status()));
        }
        let text = String::from_utf8(self.read_body(response).await?)
            .map_err(|_| HttpServiceError::InvalidUtf8)?;
        Ok(ConditionalTextResponse::Modified {
            text,
            etag: response_etag,
            last_modified: response_last_modified,
        })
    }

    pub(crate) async fn get_bytes(&self, url: &str) -> Result<Vec<u8>, HttpServiceError> {
        if let Some((reference, path)) = split_url(url) {
            return self
                .asset_bytes(reference, path)
                .await?
                .ok_or(HttpServiceError::Status(StatusCode::NOT_FOUND));
        }
        self.request_bytes(Method::GET, url, &[], None).await
    }

    pub(crate) async fn request_bytes(
        &self,
        method: Method,
        url: &str,
        headers: &[(String, String)],
        body: Option<Vec<u8>>,
    ) -> Result<Vec<u8>, HttpServiceError> {
        let response = self.request(method, url, headers, body).await?;
        if !response.status.is_success() {
            return Err(HttpServiceError::Status(response.status));
        }
        Ok(response.body)
    }

    pub(crate) async fn request(
        &self,
        method: Method,
        url: &str,
        headers: &[(String, String)],
        body: Option<Vec<u8>>,
    ) -> Result<HttpResponse, HttpServiceError> {
        let _permit = self.acquire().await?;
        let mut request = self.client.request(method, url);
        for (name, value) in headers {
            request = request.header(name, value);
        }
        if let Some(body) = body {
            request = request.body(body);
        }
        let response = request.send().await.map_err(HttpServiceError::Request)?;
        let status = response.status();
        let headers = response.headers().clone();
        let body = self.read_body(response).await?;
        Ok(HttpResponse {
            status,
            headers,
            body,
        })
    }

    pub(crate) async fn head_exists(&self, url: &str) -> Result<bool, HttpServiceError> {
        if let Some((reference, path)) = split_url(url) {
            let dataset = self
                .assets
                .load(reference, |url, max| self.repository_read(url, max))
                .await
                .map_err(HttpServiceError::Dataset)?;
            let Some(file) = dataset.resolve(path).map_err(HttpServiceError::Dataset)? else {
                return Ok(false);
            };
            if file.proof.is_some() {
                return Ok(true);
            }
        }
        let _permit = self.acquire().await?;
        let response = self
            .client
            .head(url)
            .send()
            .await
            .map_err(HttpServiceError::Request)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(false);
        }
        if !response.status().is_success() {
            return Err(HttpServiceError::Status(response.status()));
        }
        Ok(true)
    }

    async fn get_text_inner(
        &self,
        url: &str,
        allow_not_found: bool,
    ) -> Result<Option<String>, HttpServiceError> {
        if let Some((reference, path)) = split_url(url) {
            let bytes = self.asset_bytes(reference, path).await?;
            if bytes.is_none() && !allow_not_found {
                return Err(HttpServiceError::Status(StatusCode::NOT_FOUND));
            }
            return bytes
                .map(|bytes| {
                    String::from_utf8(bytes)
                        .map(|text| text.trim_start_matches('\u{feff}').to_owned())
                })
                .transpose()
                .map_err(|_| HttpServiceError::InvalidUtf8);
        }
        let _permit = self.acquire().await?;
        let response = self
            .client
            .get(url)
            .send()
            .await
            .map_err(HttpServiceError::Request)?;

        if allow_not_found && response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(HttpServiceError::Status(response.status()));
        }
        String::from_utf8(self.read_body(response).await?)
            .map(Some)
            .map_err(|_| HttpServiceError::InvalidUtf8)
    }

    async fn acquire(&self) -> Result<tokio::sync::SemaphorePermit<'_>, HttpServiceError> {
        self.permits
            .acquire()
            .await
            .map_err(|_| HttpServiceError::Closed)
    }

    pub(crate) async fn pin_asset_base(&self, base: &str) -> Result<String, HttpServiceError> {
        let url = format!("{}/_", base.trim_end_matches('/'));
        let Some((reference, _)) = split_url(&url) else {
            return Ok(base.to_owned());
        };
        let dataset = self
            .assets
            .load(reference, |url, max| self.repository_read(url, max))
            .await
            .map_err(HttpServiceError::Dataset)?;
        Ok(format!("{RAW_BASE}{}/jp/sitedata", dataset.revision))
    }

    async fn asset_bytes(
        &self,
        reference: &str,
        path: &str,
    ) -> Result<Option<Vec<u8>>, HttpServiceError> {
        let dataset = self
            .assets
            .load(reference, |url, max| self.repository_read(url, max))
            .await
            .map_err(HttpServiceError::Dataset)?;
        let Some(file) = dataset.resolve(path).map_err(HttpServiceError::Dataset)? else {
            return Ok(None);
        };
        let bytes = self
            .repository_read(file.url, self.max_response_bytes)
            .await
            .map_err(HttpServiceError::Dataset)?
            .ok_or(HttpServiceError::Status(StatusCode::NOT_FOUND))?;
        if let Some(proof) = file.proof {
            proof.verify(&bytes).map_err(HttpServiceError::Dataset)?;
        }
        Ok(Some(bytes))
    }

    async fn repository_read(
        &self,
        url: String,
        maximum: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        if maximum > INDEX_LIMIT
            || !(url.starts_with(RAW_BASE)
                || url
                    .starts_with("https://api.github.com/repos/sinsuirakv0/KBC-rakv0-assets/git/"))
        {
            return Err("Invalid dataset request".into());
        }
        let _permit = self.acquire().await.map_err(|e| e.to_string())?;
        let mut request = self
            .client
            .get(&url)
            .header(reqwest::header::CACHE_CONTROL, "no-cache");
        if url.starts_with("https://api.github.com/")
            && let Ok(token) =
                std::env::var("GITHUB_TOKEN").or_else(|_| std::env::var("GITHUB_DATA_TOKEN"))
            && !token.is_empty()
        {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(|e| e.to_string())?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response.error_for_status().map_err(|e| e.to_string())?;
        self.read_body_limit(response, maximum)
            .await
            .map(Some)
            .map_err(|e| e.to_string())
    }

    async fn read_body(&self, response: Response) -> Result<Vec<u8>, HttpServiceError> {
        self.read_body_limit(response, self.max_response_bytes)
            .await
    }
    async fn read_body_limit(
        &self,
        mut response: Response,
        maximum: usize,
    ) -> Result<Vec<u8>, HttpServiceError> {
        if response
            .content_length()
            .is_some_and(|length| length > maximum as u64)
        {
            return Err(HttpServiceError::ResponseTooLarge { maximum });
        }
        let capacity = response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(maximum);
        let mut body = Vec::with_capacity(capacity);
        while let Some(chunk) = response.chunk().await.map_err(HttpServiceError::Request)? {
            if body.len().saturating_add(chunk.len()) > maximum {
                return Err(HttpServiceError::ResponseTooLarge { maximum });
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

fn header_text(response: &Response, name: reqwest::header::HeaderName) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[derive(Debug)]
pub(crate) enum HttpServiceError {
    Dataset(String),
    Initialization(reqwest::Error),
    Request(reqwest::Error),
    Status(StatusCode),
    ResponseTooLarge { maximum: usize },
    InvalidUtf8,
    Closed,
    UnexpectedNotFound,
}

impl Display for HttpServiceError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Dataset(error) => write!(formatter, "asset dataset: {error}"),
            Self::Initialization(error) => {
                write!(formatter, "HTTP client initialization failed: {error}")
            }
            Self::Request(error) => write!(formatter, "HTTP request failed: {error}"),
            Self::Status(status) => write!(formatter, "HTTP request returned status {status}"),
            Self::ResponseTooLarge { maximum } => {
                write!(formatter, "HTTP response exceeded {maximum} bytes")
            }
            Self::InvalidUtf8 => formatter.write_str("HTTP response was not valid UTF-8"),
            Self::Closed => formatter.write_str("HTTP concurrency limiter is closed"),
            Self::UnexpectedNotFound => formatter.write_str("HTTP resource was not found"),
        }
    }
}

impl Error for HttpServiceError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Initialization(error) | Self::Request(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod platform_live {
    use super::*;
    #[tokio::test]
    #[ignore = "公開assetsの疎通検証。Discordへ接続しない"]
    async fn asset_platform_live() {
        let http = HttpService::new(HttpServiceConfig {
            max_concurrency: 4,
            request_timeout: Duration::from_secs(15),
            max_response_bytes: 8 * 1024 * 1024,
        })
        .unwrap();
        let revision = std::env::var("KBC_ASSET_TEST_REF").unwrap_or("main".into());
        let base = http
            .pin_asset_base(&format!("{RAW_BASE}{revision}/jp/sitedata"))
            .await
            .unwrap();
        let chars = http
            .get_text(&format!("{base}/character-index.json"))
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_str(&chars).unwrap();
        assert!(value["units"].as_array().unwrap().len() > 800);
        for path in [
            "Data/unitbuy.csv",
            "res/Enemyname.tsv",
            "res/Map_Name.csv",
            "res/All_day_event.tsv",
            "res/Mission_Name.csv",
            "Data/GatyaData_Option_SetR.tsv",
        ] {
            assert!(
                !http
                    .get_text(&format!("{base}/{path}"))
                    .await
                    .unwrap()
                    .is_empty()
            );
        }
        let image = http
            .get_bytes(&format!("{base}/Number/000_f.png"))
            .await
            .unwrap();
        assert!(image.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert!(
            !http
                .head_exists(&format!("{base}/Number/not-selected.png"))
                .await
                .unwrap()
        );
        println!("Discord verified assets: {base}");
    }
}
