//! `ut`と`tut`が共有する、有限なRemote snapshotとAsset存在確認を提供する。

use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::time::Duration;

use kbc_protocol::CoreActionData;
use tokio::sync::Mutex;
use tokio::task::JoinSet;
use tokio::time::Instant;

use crate::services::{ConditionalTextResponse, HttpService};

const MAX_ASSET_EXISTENCE_ENTRIES: usize = 512;
const ASSET_CHECK_BATCH_SIZE: usize = 6;

struct TextSnapshot<T> {
    value: Arc<T>,
    etag: Option<String>,
    last_modified: Option<String>,
    validated_at: Instant,
}

pub(in crate::commands) struct CachedTextResource<T> {
    label: &'static str,
    url: &'static str,
    ttl: Duration,
    http: Arc<HttpService>,
    parse: fn(&str) -> Result<T, String>,
    snapshot: Mutex<Option<TextSnapshot<T>>>,
}

impl<T> CachedTextResource<T> {
    pub(in crate::commands) fn new(
        label: &'static str,
        url: &'static str,
        ttl: Duration,
        http: Arc<HttpService>,
        parse: fn(&str) -> Result<T, String>,
    ) -> Self {
        Self {
            label,
            url,
            ttl,
            http,
            parse,
            snapshot: Mutex::new(None),
        }
    }

    pub(in crate::commands) async fn fetch(&self) -> Result<Arc<T>, RemoteDataError> {
        let mut snapshot = self.snapshot.lock().await;
        let now = Instant::now();
        if let Some(snapshot) = snapshot.as_ref()
            && now.duration_since(snapshot.validated_at) < self.ttl
        {
            return Ok(Arc::clone(&snapshot.value));
        }

        let response = self
            .http
            .get_conditional_text(
                self.url,
                snapshot.as_ref().and_then(|entry| entry.etag.as_deref()),
                snapshot
                    .as_ref()
                    .and_then(|entry| entry.last_modified.as_deref()),
            )
            .await;
        let refreshed = match response {
            Ok(ConditionalTextResponse::NotModified {
                etag,
                last_modified,
            }) => {
                let Some(current) = snapshot.as_mut() else {
                    return Err(RemoteDataError::new(
                        self.label,
                        "304 response was received without a snapshot",
                    ));
                };
                if etag.is_some() {
                    current.etag = etag;
                }
                if last_modified.is_some() {
                    current.last_modified = last_modified;
                }
                current.validated_at = now;
                Ok(Arc::clone(&current.value))
            }
            Ok(ConditionalTextResponse::Modified {
                text,
                etag,
                last_modified,
            }) => match (self.parse)(&text) {
                Ok(value) => {
                    let value = Arc::new(value);
                    *snapshot = Some(TextSnapshot {
                        value: Arc::clone(&value),
                        etag,
                        last_modified,
                        validated_at: now,
                    });
                    Ok(value)
                }
                Err(reason) => Err(RemoteDataError::new(self.label, reason)),
            },
            Err(error) => Err(RemoteDataError::new(self.label, error.to_string())),
        };

        match refreshed {
            Ok(value) => Ok(value),
            Err(error) => match snapshot.as_ref() {
                Some(stale) => {
                    eprintln!(
                        "{} refresh failed; stale snapshot is used: {error}",
                        self.label
                    );
                    Ok(Arc::clone(&stale.value))
                }
                None => Err(error),
            },
        }
    }
}

#[derive(Clone)]
pub(crate) struct RemoteAssetSource {
    base_url: &'static str,
    ttl: Duration,
    http: Arc<HttpService>,
    existence: Arc<Mutex<HashMap<String, AssetExistence>>>,
}

#[derive(Clone, Copy)]
struct AssetExistence {
    exists: bool,
    validated_at: Instant,
}

impl RemoteAssetSource {
    pub(crate) fn new(base_url: &'static str, ttl: Duration, http: Arc<HttpService>) -> Self {
        Self {
            base_url,
            ttl,
            http,
            existence: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub(in crate::commands) async fn find_existing(
        &self,
        relative_paths: &[String],
    ) -> Result<HashSet<String>, RemoteDataError> {
        let mut unique = Vec::new();
        let mut seen = HashSet::new();
        for path in relative_paths {
            if seen.insert(path.clone()) {
                unique.push(path.clone());
            }
        }

        let mut existing = HashSet::new();
        for batch in unique.chunks(ASSET_CHECK_BATCH_SIZE) {
            let mut checks = JoinSet::new();
            for path in batch {
                let source = self.clone();
                let path = path.clone();
                checks.spawn(async move {
                    let exists = source.check_exists(&path).await?;
                    Ok::<_, RemoteDataError>((path, exists))
                });
            }
            while let Some(result) = checks.join_next().await {
                let (path, path_exists) =
                    result.map_err(|error| RemoteDataError::new("asset", error.to_string()))??;
                if path_exists {
                    existing.insert(path);
                }
            }
        }
        Ok(existing)
    }

    pub(in crate::commands) async fn attachment(
        &self,
        channel_id: &str,
        relative_path: &str,
    ) -> Result<CoreActionData, RemoteDataError> {
        let data = self
            .http
            .get_bytes(&self.build_url(relative_path)?)
            .await
            .map_err(|error| RemoteDataError::new("asset", error.to_string()))?;
        let file_name = relative_path
            .rsplit('/')
            .next()
            .filter(|name| !name.is_empty())
            .unwrap_or("asset.bin")
            .to_owned();
        let content_type = relative_path
            .to_ascii_lowercase()
            .ends_with(".png")
            .then(|| "image/png".to_owned());
        Ok(CoreActionData::SendAttachment {
            channel_id: channel_id.to_owned(),
            file_name,
            content_type,
            message: None,
            data,
        })
    }

    pub(crate) async fn bytes(&self, relative_path: &str) -> Result<Vec<u8>, RemoteDataError> {
        self.http
            .get_bytes(&self.build_url(relative_path)?)
            .await
            .map_err(|error| RemoteDataError::new("asset", error.to_string()))
    }

    async fn check_exists(&self, relative_path: &str) -> Result<bool, RemoteDataError> {
        let now = Instant::now();
        let cached = {
            let existence = self.existence.lock().await;
            existence.get(relative_path).copied()
        };
        if let Some(cached) = cached
            && now.duration_since(cached.validated_at) < self.ttl
        {
            return Ok(cached.exists);
        }

        let checked = self.http.head_exists(&self.build_url(relative_path)?).await;
        match checked {
            Ok(exists) => {
                let mut existence = self.existence.lock().await;
                if !existence.contains_key(relative_path)
                    && existence.len() >= MAX_ASSET_EXISTENCE_ENTRIES
                    && let Some(oldest) = existence
                        .iter()
                        .min_by_key(|(_, entry)| entry.validated_at)
                        .map(|(path, _)| path.clone())
                {
                    existence.remove(&oldest);
                }
                existence.insert(
                    relative_path.to_owned(),
                    AssetExistence {
                        exists,
                        validated_at: now,
                    },
                );
                Ok(exists)
            }
            Err(error) => cached.map(|entry| entry.exists).ok_or_else(|| {
                RemoteDataError::new("asset", format!("existence check failed: {error}"))
            }),
        }
    }

    fn build_url(&self, relative_path: &str) -> Result<String, RemoteDataError> {
        if !is_safe_asset_path(relative_path) {
            return Err(RemoteDataError::new("asset", "unsafe relative path"));
        }
        Ok(format!(
            "{}/{}",
            self.base_url.trim_end_matches('/'),
            relative_path
        ))
    }
}

fn is_safe_asset_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains('\\')
        && !value.contains(['?', '#', '%'])
        && !value.chars().any(char::is_control)
        && value.split('/').all(|segment| {
            !segment.is_empty()
                && segment != "."
                && segment != ".."
                && segment
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
        })
        && ["png", "imgcut", "mamodel", "maanim"]
            .iter()
            .any(|extension| {
                value
                    .to_ascii_lowercase()
                    .ends_with(&format!(".{extension}"))
            })
}

#[derive(Debug)]
pub(crate) struct RemoteDataError {
    label: &'static str,
    reason: String,
}

impl RemoteDataError {
    pub(crate) fn new(label: &'static str, reason: impl Into<String>) -> Self {
        Self {
            label,
            reason: reason.into(),
        }
    }
}

impl Display for RemoteDataError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} retrieval failed: {}",
            self.label, self.reason
        )
    }
}
