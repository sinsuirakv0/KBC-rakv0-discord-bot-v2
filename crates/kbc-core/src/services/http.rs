//! Timeout・Response上限・同時実行数を共有するHTTP Client。

use std::error::Error;
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::Duration;

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

    async fn read_body(&self, mut response: Response) -> Result<Vec<u8>, HttpServiceError> {
        if response
            .content_length()
            .is_some_and(|length| length > self.max_response_bytes as u64)
        {
            return Err(HttpServiceError::ResponseTooLarge {
                maximum: self.max_response_bytes,
            });
        }
        let capacity = response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or(0)
            .min(self.max_response_bytes);
        let mut body = Vec::with_capacity(capacity);
        while let Some(chunk) = response.chunk().await.map_err(HttpServiceError::Request)? {
            if body.len().saturating_add(chunk.len()) > self.max_response_bytes {
                return Err(HttpServiceError::ResponseTooLarge {
                    maximum: self.max_response_bytes,
                });
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
