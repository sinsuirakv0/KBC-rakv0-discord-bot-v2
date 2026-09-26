//! private GitHub Repositoryを正本とする通知設定Storage。

use std::collections::{HashMap, HashSet};
use std::fmt::{Debug, Display, Formatter};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use tokio::time::{Instant, sleep};

use crate::notification::EventRecord;
use crate::services::HttpService;
use crate::store_update::{StorePlatform, valid_version};

const MANIFEST_PATH: &str = "meta.json";
const MAINTAINERS_PATH: &str = "config/maintainers.json";
const STORE_VERSIONS_PATH: &str = "state/store-versions.json";
const MAX_DOCUMENT_BYTES: usize = 256 * 1024;
const MAX_DIRECTORY_FILES: usize = 999;
pub(crate) const MAX_MAINTAINER_SUBJECTS: usize = 64;
pub(crate) const MAX_NOTIFICATION_ROLES: usize = 9;
pub(crate) const MAX_SKD_RELATED_URLS: usize = 9;
const MAX_SKD_RELATED_URL_CHARS: usize = 1_200;
const WRITE_INTERVAL: Duration = Duration::from_secs(1);
const WRITE_ATTEMPTS: usize = 2;

#[derive(Clone, Eq, PartialEq)]
pub struct StorageConfig {
    pub owner: String,
    pub repository: String,
    pub branch: String,
    pub token: String,
}

impl Debug for StorageConfig {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StorageConfig")
            .field("owner", &self.owner)
            .field("repository", &self.repository)
            .field("branch", &self.branch)
            .field("token", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NotificationCategory {
    Skd,
    Ad,
    Notice,
    UpdateAndroid,
    UpdateIos,
}

impl NotificationCategory {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Skd => "skd",
            Self::Ad => "ad",
            Self::Notice => "notice",
            Self::UpdateAndroid => "update android",
            Self::UpdateIos => "update ios",
        }
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Subscription {
    pub(crate) guild_id: String,
    pub(crate) channel_id: String,
    pub(crate) category: NotificationCategory,
    #[serde(default)]
    pub(crate) role_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationRole {
    pub(crate) role_id: String,
    pub(crate) name: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotificationRolePanel {
    pub(crate) channel_id: String,
    pub(crate) message_id: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct NotificationRoleSettings {
    pub(crate) roles: Vec<NotificationRole>,
    pub(crate) panel: Option<NotificationRolePanel>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct GuildSettings {
    schema_version: u8,
    guild_id: String,
    health_maintainer_role_id: Option<String>,
    subscriptions: Vec<Subscription>,
    #[serde(default)]
    notification_roles: Vec<NotificationRole>,
    #[serde(default)]
    notification_role_panel: Option<NotificationRolePanel>,
    #[serde(default)]
    skd_related_urls: Vec<String>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MaintainerSettings {
    schema_version: u8,
    subject_ids: Vec<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct StoreVersionState {
    schema_version: u8,
    google_play: Option<String>,
    app_store: Option<String>,
}

impl Default for StoreVersionState {
    fn default() -> Self {
        Self {
            schema_version: 1,
            google_play: None,
            app_store: None,
        }
    }
}

impl StoreVersionState {
    fn version(&self, platform: StorePlatform) -> Option<&str> {
        match platform {
            StorePlatform::Android => self.google_play.as_deref(),
            StorePlatform::Ios => self.app_store.as_deref(),
        }
    }

    fn set_version(&mut self, platform: StorePlatform, version: String) {
        match platform {
            StorePlatform::Android => self.google_play = Some(version),
            StorePlatform::Ios => self.app_store = Some(version),
        }
    }
}

struct RepositoryFile {
    content: String,
    sha: String,
}

#[derive(Default)]
struct StorageState {
    initialized: bool,
    settings: HashMap<String, GuildSettings>,
    maintainers: MaintainerSettings,
    store_versions: StoreVersionState,
    last_write_at: Option<Instant>,
    cooldown_until: Option<Instant>,
}

pub(crate) struct StorageService {
    config: Option<StorageConfig>,
    http: Arc<HttpService>,
    state: Mutex<StorageState>,
}

impl StorageService {
    pub(crate) fn new(config: Option<StorageConfig>, http: Arc<HttpService>) -> Self {
        Self {
            config,
            http,
            state: Mutex::new(StorageState::default()),
        }
    }

    pub(crate) async fn set_subscription(
        &self,
        subscription: Subscription,
        enabled: bool,
    ) -> Result<(), StorageError> {
        validate_subscription(&subscription)?;
        let guild_id = subscription.guild_id.clone();
        self.update_guild_settings(&guild_id, |settings| {
            let exists = settings.subscriptions.iter().any(|current| {
                current.channel_id == subscription.channel_id
                    && current.category == subscription.category
            });
            if enabled && !exists {
                settings.subscriptions.push(subscription.clone());
            } else if !enabled && exists {
                settings.subscriptions.retain(|current| {
                    current.channel_id != subscription.channel_id
                        || current.category != subscription.category
                });
            } else {
                return Ok(false);
            }
            Ok(true)
        })
        .await
        .map(|_| ())
    }

    pub(crate) async fn subscriptions(&self) -> Result<Vec<Subscription>, StorageError> {
        let mut state = self.state.lock().await;
        self.ensure_initialized(&mut state).await?;
        Ok(state
            .settings
            .values()
            .flat_map(|settings| settings.subscriptions.iter().cloned())
            .collect())
    }

    pub(crate) async fn store_version(
        &self,
        platform: StorePlatform,
    ) -> Result<Option<String>, StorageError> {
        let mut state = self.state.lock().await;
        self.ensure_initialized(&mut state).await?;
        Ok(state.store_versions.version(platform).map(str::to_owned))
    }

    pub(crate) async fn set_store_version(
        &self,
        platform: StorePlatform,
        version: &str,
    ) -> Result<(), StorageError> {
        if !valid_version(version) {
            return Err(StorageError::new("invalid-store-version"));
        }
        let mut state = self.state.lock().await;
        self.ensure_initialized(&mut state).await?;
        for attempt in 0..WRITE_ATTEMPTS {
            let file = self.read_file(&mut state, STORE_VERSIONS_PATH).await?;
            let mut versions = match &file {
                Some(file) => parse_store_versions(&file.content)?,
                None => StoreVersionState::default(),
            };
            if versions.version(platform) == Some(version) {
                state.store_versions = versions;
                return Ok(());
            }
            versions.set_version(platform, version.to_owned());
            validate_store_versions(&versions)?;
            let content = serde_json::to_string(&versions)
                .map_err(|error| StorageError::detail("json-encode", error))?;
            match self
                .write_file(
                    &mut state,
                    STORE_VERSIONS_PATH,
                    &content,
                    file.as_ref().map(|current| current.sha.as_str()),
                )
                .await
            {
                Ok(()) => {
                    state.store_versions = versions;
                    return Ok(());
                }
                Err(error) if error.code() == "conflict" && attempt + 1 < WRITE_ATTEMPTS => {
                    eprintln!("Store version write conflicted; reloading the latest state");
                }
                Err(error) => return Err(error),
            }
        }
        Err(StorageError::new("conflict"))
    }

    pub(crate) async fn notification_role_settings(
        &self,
        guild_id: &str,
    ) -> Result<NotificationRoleSettings, StorageError> {
        if !valid_snowflake(guild_id) {
            return Err(StorageError::new("invalid-guild-id"));
        }
        let mut state = self.state.lock().await;
        self.ensure_initialized(&mut state).await?;
        let Some(settings) = state.settings.get(guild_id) else {
            return Ok(NotificationRoleSettings::default());
        };
        Ok(NotificationRoleSettings {
            roles: settings.notification_roles.clone(),
            panel: settings.notification_role_panel.clone(),
        })
    }

    pub(crate) async fn set_notification_role(
        &self,
        guild_id: &str,
        role: &NotificationRole,
        enabled: bool,
    ) -> Result<bool, StorageError> {
        validate_notification_role(role)?;
        self.update_guild_settings(guild_id, |settings| {
            let position = settings
                .notification_roles
                .iter()
                .position(|current| current.role_id == role.role_id);
            if enabled {
                if let Some(position) = position {
                    if settings.notification_roles[position] == *role {
                        return Ok(false);
                    }
                    settings.notification_roles[position] = role.clone();
                } else {
                    if settings.notification_roles.len() >= MAX_NOTIFICATION_ROLES {
                        return Err(StorageError::new("notification-role-limit-reached"));
                    }
                    settings.notification_roles.push(role.clone());
                }
            } else if position.is_some() {
                settings
                    .notification_roles
                    .retain(|current| current.role_id != role.role_id);
                for subscription in &mut settings.subscriptions {
                    subscription
                        .role_ids
                        .retain(|role_id| role_id != &role.role_id);
                }
            } else {
                return Ok(false);
            }
            Ok(true)
        })
        .await
    }

    pub(crate) async fn set_notification_role_panel(
        &self,
        guild_id: &str,
        panel: NotificationRolePanel,
    ) -> Result<(), StorageError> {
        validate_notification_role_panel(&panel)?;
        self.update_guild_settings(guild_id, |settings| {
            if settings.notification_role_panel.as_ref() == Some(&panel) {
                return Ok(false);
            }
            settings.notification_role_panel = Some(panel.clone());
            Ok(true)
        })
        .await
        .map(|_| ())
    }

    pub(crate) async fn set_subscription_role(
        &self,
        guild_id: &str,
        channel_id: &str,
        category: NotificationCategory,
        role_id: &str,
        enabled: bool,
    ) -> Result<bool, StorageError> {
        if !valid_snowflake(channel_id) || !valid_snowflake(role_id) {
            return Err(StorageError::new("invalid-subscription-role"));
        }
        self.update_guild_settings(guild_id, |settings| {
            if !settings
                .notification_roles
                .iter()
                .any(|role| role.role_id == role_id)
            {
                return Err(StorageError::new("notification-role-not-registered"));
            }
            let Some(subscription) = settings.subscriptions.iter_mut().find(|subscription| {
                subscription.channel_id == channel_id && subscription.category == category
            }) else {
                return Err(StorageError::new("subscription-not-found"));
            };
            let exists = subscription
                .role_ids
                .iter()
                .any(|current| current == role_id);
            if enabled == exists {
                return Ok(false);
            }
            if enabled {
                subscription.role_ids.push(role_id.to_owned());
            } else {
                subscription.role_ids.retain(|current| current != role_id);
            }
            Ok(true)
        })
        .await
    }

    pub(crate) async fn notification_role_for_reaction(
        &self,
        guild_id: &str,
        channel_id: &str,
        message_id: &str,
        index: usize,
    ) -> Result<Option<String>, StorageError> {
        if !valid_snowflake(guild_id)
            || !valid_snowflake(channel_id)
            || !valid_snowflake(message_id)
        {
            return Ok(None);
        }
        let mut state = self.state.lock().await;
        self.ensure_initialized(&mut state).await?;
        let Some(settings) = state.settings.get(guild_id) else {
            return Ok(None);
        };
        if settings
            .notification_role_panel
            .as_ref()
            .is_none_or(|panel| panel.channel_id != channel_id || panel.message_id != message_id)
        {
            return Ok(None);
        }
        Ok(settings
            .notification_roles
            .get(index)
            .map(|role| role.role_id.clone()))
    }

    pub(crate) async fn set_skd_related_url(
        &self,
        guild_id: &str,
        url: &str,
        enabled: bool,
    ) -> Result<bool, StorageError> {
        let url = normalize_web_url(url).ok_or_else(|| StorageError::new("invalid-related-url"))?;
        self.update_guild_settings(guild_id, |settings| {
            let exists = settings
                .skd_related_urls
                .iter()
                .any(|current| current == &url);
            if enabled == exists {
                return Ok(false);
            }
            if enabled {
                if settings.skd_related_urls.len() >= MAX_SKD_RELATED_URLS
                    || settings
                        .skd_related_urls
                        .iter()
                        .map(|current| current.chars().count())
                        .sum::<usize>()
                        + url.chars().count()
                        > MAX_SKD_RELATED_URL_CHARS
                {
                    return Err(StorageError::new("related-url-limit-reached"));
                }
                settings.skd_related_urls.push(url.clone());
            } else {
                settings.skd_related_urls.retain(|current| current != &url);
            }
            Ok(true)
        })
        .await
    }

    pub(crate) async fn skd_related_urls(
        &self,
        guild_id: &str,
    ) -> Result<Vec<String>, StorageError> {
        if !valid_snowflake(guild_id) {
            return Err(StorageError::new("invalid-guild-id"));
        }
        let mut state = self.state.lock().await;
        self.ensure_initialized(&mut state).await?;
        Ok(state
            .settings
            .get(guild_id)
            .map(|settings| settings.skd_related_urls.clone())
            .unwrap_or_default())
    }

    pub(crate) async fn set_maintainer(
        &self,
        subject_id: &str,
        enabled: bool,
    ) -> Result<bool, StorageError> {
        if !valid_maintainer_subject_id(subject_id) {
            return Err(StorageError::new("invalid-maintainer-subject"));
        }
        let mut state = self.state.lock().await;
        self.ensure_initialized(&mut state).await?;
        for attempt in 0..WRITE_ATTEMPTS {
            let file = self.read_file(&mut state, MAINTAINERS_PATH).await?;
            let mut settings = match &file {
                Some(file) => parse_maintainers(&file.content)?,
                None => MaintainerSettings {
                    schema_version: 1,
                    subject_ids: Vec::new(),
                },
            };
            let exists = settings
                .subject_ids
                .iter()
                .any(|current| current == subject_id);
            if enabled == exists {
                state.maintainers = settings;
                return Ok(false);
            }
            if enabled {
                if settings.subject_ids.len() >= MAX_MAINTAINER_SUBJECTS {
                    return Err(StorageError::new("maintainer-limit-reached"));
                }
                settings.subject_ids.push(subject_id.to_owned());
                settings.subject_ids.sort_unstable();
            } else {
                settings.subject_ids.retain(|current| current != subject_id);
            }
            validate_maintainers(&settings)?;
            let content = serde_json::to_string(&settings)
                .map_err(|error| StorageError::detail("json-encode", error))?;
            match self
                .write_file(
                    &mut state,
                    MAINTAINERS_PATH,
                    &content,
                    file.as_ref().map(|current| current.sha.as_str()),
                )
                .await
            {
                Ok(()) => {
                    state.maintainers = settings;
                    return Ok(true);
                }
                Err(error) if error.code() == "conflict" && attempt + 1 < WRITE_ATTEMPTS => {
                    eprintln!("Storage write conflicted; reloading the latest maintainer settings");
                }
                Err(error) => return Err(error),
            }
        }
        Err(StorageError::new("conflict"))
    }

    pub(crate) async fn maintainer_subject_ids(&self) -> Result<Vec<String>, StorageError> {
        let mut state = self.state.lock().await;
        self.ensure_initialized(&mut state).await?;
        Ok(state.maintainers.subject_ids.clone())
    }

    pub(crate) async fn is_maintainer(
        &self,
        user_id: &str,
        member_role_ids: &[String],
    ) -> Result<bool, StorageError> {
        let subjects = self.maintainer_subject_ids().await?;
        Ok(subjects.iter().any(|subject| subject == user_id)
            || member_role_ids
                .iter()
                .any(|role_id| subjects.iter().any(|subject| subject == role_id)))
    }

    pub(crate) async fn initialize(&self) -> Result<(), StorageError> {
        let mut state = self.state.lock().await;
        self.ensure_initialized(&mut state).await
    }

    pub(crate) async fn update_notification<F>(
        &self,
        event_id: &str,
        change: F,
    ) -> Result<EventRecord, StorageError>
    where
        F: FnOnce(Option<EventRecord>) -> Result<EventRecord, StorageError>,
    {
        let mut state = self.state.lock().await;
        self.ensure_initialized(&mut state).await?;
        let path = notification_path(event_id)?;
        let file = self.read_file(&mut state, &path).await?;
        let current = file
            .as_ref()
            .map(|file| {
                serde_json::from_str::<EventRecord>(file.content.trim_start_matches('\u{feff}'))
                    .map_err(|error| StorageError::detail("invalid-event-record", error))
            })
            .transpose()?;
        if current
            .as_ref()
            .is_some_and(|record| record.event.event_id != event_id)
        {
            return Err(StorageError::new("event-id-mismatch"));
        }
        let value = change(current.clone())?;
        value.validate()?;
        let content = serde_json::to_string(&value)
            .map_err(|error| StorageError::detail("json-encode", error))?;
        let unchanged = current
            .as_ref()
            .and_then(|record| serde_json::to_string(record).ok())
            .is_some_and(|current| current == content);
        if !unchanged {
            self.write_file(
                &mut state,
                &path,
                &content,
                file.as_ref().map(|current| current.sha.as_str()),
            )
            .await?;
        }
        Ok(value)
    }

    async fn update_guild_settings<F>(
        &self,
        guild_id: &str,
        change: F,
    ) -> Result<bool, StorageError>
    where
        F: Fn(&mut GuildSettings) -> Result<bool, StorageError>,
    {
        if !valid_snowflake(guild_id) {
            return Err(StorageError::new("invalid-guild-id"));
        }
        let mut state = self.state.lock().await;
        self.ensure_initialized(&mut state).await?;
        let path = guild_path(guild_id)?;
        for attempt in 0..WRITE_ATTEMPTS {
            let file = self.read_file(&mut state, &path).await?;
            let mut settings = match &file {
                Some(file) => parse_settings(&file.content)?,
                None => new_guild_settings(guild_id),
            };
            if settings.guild_id != guild_id {
                return Err(StorageError::new("invalid-guild-id"));
            }
            if !change(&mut settings)? {
                state.settings.insert(settings.guild_id.clone(), settings);
                return Ok(false);
            }
            validate_settings(&settings)?;
            let content = serde_json::to_string(&settings)
                .map_err(|error| StorageError::detail("json-encode", error))?;
            match self
                .write_file(
                    &mut state,
                    &path,
                    &content,
                    file.as_ref().map(|current| current.sha.as_str()),
                )
                .await
            {
                Ok(()) => {
                    state.settings.insert(settings.guild_id.clone(), settings);
                    return Ok(true);
                }
                Err(error) if error.code() == "conflict" && attempt + 1 < WRITE_ATTEMPTS => {
                    eprintln!("Storage write conflicted; reloading the latest guild settings");
                }
                Err(error) => return Err(error),
            }
        }
        Err(StorageError::new("conflict"))
    }

    async fn ensure_initialized(&self, state: &mut StorageState) -> Result<(), StorageError> {
        if state.initialized {
            return Ok(());
        }
        self.verify_repository(state).await?;
        let manifest = self
            .read_file(state, MANIFEST_PATH)
            .await?
            .ok_or_else(|| StorageError::new("initialization-required"))?;
        let manifest: Manifest =
            serde_json::from_str(manifest.content.trim_start_matches('\u{feff}'))
                .map_err(|error| StorageError::detail("invalid-manifest", error))?;
        if manifest.schema_version != 1 || manifest.application != "kbc-discord-bot-data" {
            return Err(StorageError::new("invalid-manifest"));
        }
        let paths = self.list_directory(state, "config/guilds").await?;
        let mut settings = HashMap::new();
        for path in paths {
            let file = self
                .read_file(state, &path)
                .await?
                .ok_or_else(|| StorageError::new("missing-guild-settings"))?;
            let value = parse_settings(&file.content)?;
            if path != guild_path(&value.guild_id)? {
                return Err(StorageError::new("invalid-guild-path"));
            }
            settings.insert(value.guild_id.clone(), value);
        }
        let maintainers = match self.read_file(state, MAINTAINERS_PATH).await? {
            Some(file) => parse_maintainers(&file.content)?,
            None => MaintainerSettings {
                schema_version: 1,
                subject_ids: Vec::new(),
            },
        };
        let store_versions = match self.read_file(state, STORE_VERSIONS_PATH).await? {
            Some(file) => parse_store_versions(&file.content)?,
            None => StoreVersionState::default(),
        };
        state.settings = settings;
        state.maintainers = maintainers;
        state.store_versions = store_versions;
        state.initialized = true;
        Ok(())
    }

    async fn verify_repository(&self, state: &mut StorageState) -> Result<(), StorageError> {
        let config = self.config()?;
        let repository: RepositoryResponse = self
            .request_json(state, Method::GET, &self.base_url()?, None)
            .await?;
        if repository.private != Some(true)
            || repository.archived.unwrap_or(false)
            || repository.disabled.unwrap_or(false)
        {
            return Err(StorageError::new("repository-unavailable"));
        }
        let _: serde_json::Value = self
            .request_json(
                state,
                Method::GET,
                &format!(
                    "{}/branches/{}",
                    self.base_url()?,
                    encode_path_segment(&config.branch)
                ),
                None,
            )
            .await?;
        Ok(())
    }

    async fn read_file(
        &self,
        state: &mut StorageState,
        path: &str,
    ) -> Result<Option<RepositoryFile>, StorageError> {
        validate_path(path)?;
        let config = self.config()?;
        let response = self
            .request(
                state,
                Method::GET,
                &format!(
                    "{}/contents/{path}?ref={}",
                    self.base_url()?,
                    encode_path_segment(&config.branch)
                ),
                None,
            )
            .await?;
        if response.status == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status.is_success() {
            return Err(status_error(response.status));
        }
        let value: ContentResponse = serde_json::from_slice(&response.body)
            .map_err(|error| StorageError::detail("invalid-response", error))?;
        if value.entry_type != "file" || value.encoding != "base64" || value.sha.is_empty() {
            return Err(StorageError::new("invalid-file"));
        }
        let encoded = value
            .content
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        let bytes = BASE64
            .decode(encoded)
            .map_err(|error| StorageError::detail("invalid-base64", error))?;
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err(StorageError::new("document-too-large"));
        }
        let content = String::from_utf8(bytes).map_err(|_| StorageError::new("invalid-utf8"))?;
        Ok(Some(RepositoryFile {
            content,
            sha: value.sha,
        }))
    }

    async fn list_directory(
        &self,
        state: &mut StorageState,
        directory: &str,
    ) -> Result<Vec<String>, StorageError> {
        validate_path(directory)?;
        let config = self.config()?;
        let response = self
            .request(
                state,
                Method::GET,
                &format!(
                    "{}/contents/{directory}?ref={}",
                    self.base_url()?,
                    encode_path_segment(&config.branch)
                ),
                None,
            )
            .await?;
        if response.status == StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if !response.status.is_success() {
            return Err(status_error(response.status));
        }
        let items: Vec<DirectoryItem> = serde_json::from_slice(&response.body)
            .map_err(|error| StorageError::detail("invalid-directory", error))?;
        if items.len() > MAX_DIRECTORY_FILES
            || items.iter().any(|item| {
                item.entry_type != "file" || !item.path.starts_with(&format!("{directory}/"))
            })
        {
            return Err(StorageError::new("invalid-directory"));
        }
        Ok(items.into_iter().map(|item| item.path).collect())
    }

    async fn write_file(
        &self,
        state: &mut StorageState,
        path: &str,
        content: &str,
        expected_sha: Option<&str>,
    ) -> Result<(), StorageError> {
        validate_path(path)?;
        if content.len() > MAX_DOCUMENT_BYTES {
            return Err(StorageError::new("document-too-large"));
        }
        if let Some(last_write_at) = state.last_write_at {
            let elapsed = Instant::now().duration_since(last_write_at);
            if elapsed < WRITE_INTERVAL {
                sleep(WRITE_INTERVAL - elapsed).await;
            }
        }
        state.last_write_at = Some(Instant::now());
        let config = self.config()?;
        let mut body = json!({
            "message": format!("Update {path}"),
            "branch": config.branch,
            "content": BASE64.encode(content.as_bytes()),
        });
        if let Some(sha) = expected_sha {
            body["sha"] = json!(sha);
        }
        let encoded = serde_json::to_vec(&body)
            .map_err(|error| StorageError::detail("json-encode", error))?;
        let response = self
            .request(
                state,
                Method::PUT,
                &format!("{}/contents/{path}", self.base_url()?),
                Some(encoded),
            )
            .await;
        match response {
            Ok(response) if response.status.is_success() => Ok(()),
            Ok(response)
                if response.status == StatusCode::CONFLICT
                    || response.status == StatusCode::UNPROCESSABLE_ENTITY =>
            {
                log_response_message("GitHub storage write conflict", &response.body);
                Err(StorageError::new("conflict"))
            }
            Ok(response) => Err(status_error(response.status)),
            Err(error) => {
                let confirmed = self.read_file(state, path).await?;
                if confirmed
                    .as_ref()
                    .is_some_and(|file| file.content == content)
                {
                    Ok(())
                } else {
                    Err(error)
                }
            }
        }
    }

    async fn request_json<T: for<'de> Deserialize<'de>>(
        &self,
        state: &mut StorageState,
        method: Method,
        url: &str,
        body: Option<Vec<u8>>,
    ) -> Result<T, StorageError> {
        let response = self.request(state, method, url, body).await?;
        if !response.status.is_success() {
            return Err(status_error(response.status));
        }
        serde_json::from_slice(&response.body)
            .map_err(|error| StorageError::detail("invalid-response", error))
    }

    async fn request(
        &self,
        state: &mut StorageState,
        method: Method,
        url: &str,
        body: Option<Vec<u8>>,
    ) -> Result<crate::services::HttpResponse, StorageError> {
        if state
            .cooldown_until
            .is_some_and(|until| until > Instant::now())
        {
            return Err(StorageError::new("rate-limited"));
        }
        let config = self.config()?;
        let headers = vec![
            (
                "authorization".to_owned(),
                format!("Bearer {}", config.token),
            ),
            (
                "accept".to_owned(),
                "application/vnd.github+json".to_owned(),
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
            ("x-github-api-version".to_owned(), "2022-11-28".to_owned()),
        ];
        let response = self
            .http
            .request(method, url, &headers, body)
            .await
            .map_err(|error| StorageError::detail("network-unavailable", error))?;
        if response.status == StatusCode::TOO_MANY_REQUESTS
            || (response.status == StatusCode::FORBIDDEN
                && (response
                    .headers
                    .get("x-ratelimit-remaining")
                    .is_some_and(|value| value == "0")
                    || response.headers.contains_key("retry-after")))
        {
            state.cooldown_until = Some(Instant::now() + Duration::from_secs(60));
            return Err(StorageError::new("rate-limited"));
        }
        Ok(response)
    }

    fn config(&self) -> Result<&StorageConfig, StorageError> {
        self.config
            .as_ref()
            .ok_or_else(|| StorageError::new("not-configured"))
    }

    fn base_url(&self) -> Result<String, StorageError> {
        let config = self.config()?;
        if !valid_repository_name(&config.owner) || !valid_repository_name(&config.repository) {
            return Err(StorageError::new("invalid-configuration"));
        }
        Ok(format!(
            "https://api.github.com/repos/{}/{}",
            config.owner, config.repository
        ))
    }
}

fn parse_settings(content: &str) -> Result<GuildSettings, StorageError> {
    let settings: GuildSettings = serde_json::from_str(content.trim_start_matches('\u{feff}'))
        .map_err(|error| StorageError::detail("invalid-guild-settings", error))?;
    validate_settings(&settings)?;
    Ok(settings)
}

fn new_guild_settings(guild_id: &str) -> GuildSettings {
    GuildSettings {
        schema_version: 1,
        guild_id: guild_id.to_owned(),
        health_maintainer_role_id: None,
        subscriptions: Vec::new(),
        notification_roles: Vec::new(),
        notification_role_panel: None,
        skd_related_urls: Vec::new(),
    }
}

fn parse_maintainers(content: &str) -> Result<MaintainerSettings, StorageError> {
    let settings: MaintainerSettings = serde_json::from_str(content.trim_start_matches('\u{feff}'))
        .map_err(|error| StorageError::detail("invalid-maintainer-settings", error))?;
    validate_maintainers(&settings)?;
    Ok(settings)
}

fn parse_store_versions(content: &str) -> Result<StoreVersionState, StorageError> {
    let versions: StoreVersionState = serde_json::from_str(content.trim_start_matches('\u{feff}'))
        .map_err(|error| StorageError::detail("invalid-store-versions", error))?;
    validate_store_versions(&versions)?;
    Ok(versions)
}

fn validate_store_versions(versions: &StoreVersionState) -> Result<(), StorageError> {
    if versions.schema_version != 1
        || versions
            .google_play
            .as_deref()
            .is_some_and(|value| !valid_version(value))
        || versions
            .app_store
            .as_deref()
            .is_some_and(|value| !valid_version(value))
    {
        return Err(StorageError::new("invalid-store-versions"));
    }
    Ok(())
}

fn validate_maintainers(settings: &MaintainerSettings) -> Result<(), StorageError> {
    if settings.schema_version != 1
        || settings.subject_ids.len() > MAX_MAINTAINER_SUBJECTS
        || settings
            .subject_ids
            .iter()
            .any(|subject| !valid_maintainer_subject_id(subject))
    {
        return Err(StorageError::new("invalid-maintainer-settings"));
    }
    let unique = settings.subject_ids.iter().collect::<HashSet<_>>();
    if unique.len() != settings.subject_ids.len() {
        return Err(StorageError::new("duplicate-maintainer-subject"));
    }
    Ok(())
}

fn validate_settings(settings: &GuildSettings) -> Result<(), StorageError> {
    if settings.schema_version != 1
        || !valid_snowflake(&settings.guild_id)
        || settings
            .health_maintainer_role_id
            .as_ref()
            .is_some_and(|role| !valid_snowflake(role))
        || settings
            .subscriptions
            .iter()
            .any(|subscription| subscription.guild_id != settings.guild_id)
        || settings.notification_roles.len() > MAX_NOTIFICATION_ROLES
        || settings
            .notification_roles
            .iter()
            .any(|role| validate_notification_role(role).is_err())
        || settings
            .notification_role_panel
            .as_ref()
            .is_some_and(|panel| validate_notification_role_panel(panel).is_err())
        || settings.skd_related_urls.len() > MAX_SKD_RELATED_URLS
        || settings
            .skd_related_urls
            .iter()
            .any(|url| normalize_web_url(url).as_deref() != Some(url.as_str()))
        || settings
            .skd_related_urls
            .iter()
            .map(|url| url.chars().count())
            .sum::<usize>()
            > MAX_SKD_RELATED_URL_CHARS
    {
        return Err(StorageError::new("invalid-guild-settings"));
    }
    let configured_roles = settings
        .notification_roles
        .iter()
        .map(|role| role.role_id.as_str())
        .collect::<HashSet<_>>();
    if configured_roles.len() != settings.notification_roles.len() {
        return Err(StorageError::new("duplicate-notification-role"));
    }
    if settings
        .skd_related_urls
        .iter()
        .collect::<HashSet<_>>()
        .len()
        != settings.skd_related_urls.len()
    {
        return Err(StorageError::new("duplicate-related-url"));
    }
    let mut unique = HashSet::new();
    for subscription in &settings.subscriptions {
        validate_subscription(subscription)?;
        if subscription.role_ids.len() > MAX_NOTIFICATION_ROLES
            || subscription
                .role_ids
                .iter()
                .any(|role_id| !configured_roles.contains(role_id.as_str()))
            || subscription.role_ids.iter().collect::<HashSet<_>>().len()
                != subscription.role_ids.len()
        {
            return Err(StorageError::new("invalid-subscription-role"));
        }
        if !unique.insert((subscription.channel_id.clone(), subscription.category)) {
            return Err(StorageError::new("duplicate-subscription"));
        }
    }
    Ok(())
}

fn validate_subscription(subscription: &Subscription) -> Result<(), StorageError> {
    if !valid_snowflake(&subscription.guild_id)
        || !valid_snowflake(&subscription.channel_id)
        || subscription
            .role_ids
            .iter()
            .any(|role_id| !valid_snowflake(role_id))
    {
        return Err(StorageError::new("invalid-subscription"));
    }
    Ok(())
}

fn validate_notification_role(role: &NotificationRole) -> Result<(), StorageError> {
    if !valid_snowflake(&role.role_id)
        || role.name.is_empty()
        || role.name.chars().count() > 100
        || role.name.chars().any(char::is_control)
    {
        return Err(StorageError::new("invalid-notification-role"));
    }
    Ok(())
}

fn validate_notification_role_panel(panel: &NotificationRolePanel) -> Result<(), StorageError> {
    if !valid_snowflake(&panel.channel_id) || !valid_snowflake(&panel.message_id) {
        return Err(StorageError::new("invalid-notification-role-panel"));
    }
    Ok(())
}

fn normalize_web_url(value: &str) -> Option<String> {
    let url = reqwest::Url::parse(value).ok()?;
    (["http", "https"].contains(&url.scheme())
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none())
    .then(|| url.to_string())
}

fn guild_path(guild_id: &str) -> Result<String, StorageError> {
    if !valid_snowflake(guild_id) {
        return Err(StorageError::new("invalid-guild-id"));
    }
    Ok(format!("config/guilds/{guild_id}.json"))
}

fn notification_path(event_id: &str) -> Result<String, StorageError> {
    if event_id.is_empty()
        || event_id.len() > 160
        || !event_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ":_-".contains(character))
    {
        return Err(StorageError::new("invalid-event-id"));
    }
    let digest = Sha256::digest(event_id.as_bytes());
    Ok(format!("notifications/events/{}.json", hex_encode(&digest)))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn valid_snowflake(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
}

fn valid_maintainer_subject_id(value: &str) -> bool {
    (17..=20).contains(&value.len())
        && value.chars().all(|character| character.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|number| number > 0)
}

fn validate_path(path: &str) -> Result<(), StorageError> {
    if path.is_empty()
        || path.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || !part
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "_.-".contains(character))
        })
    {
        return Err(StorageError::new("invalid-path"));
    }
    Ok(())
}

fn valid_repository_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "_.-".contains(character))
}

fn encode_path_segment(value: &str) -> String {
    value
        .bytes()
        .flat_map(|byte| {
            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                vec![byte as char]
            } else {
                format!("%{byte:02X}").chars().collect()
            }
        })
        .collect()
}

fn status_error(status: StatusCode) -> StorageError {
    StorageError::new(format!("http-{}", status.as_u16()))
}

fn log_response_message(context: &str, body: &[u8]) {
    let message = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("message")?.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown response".to_owned());
    eprintln!(
        "{context}: {}",
        message.chars().take(240).collect::<String>()
    );
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    schema_version: u8,
    application: String,
}

#[derive(Deserialize)]
struct RepositoryResponse {
    private: Option<bool>,
    archived: Option<bool>,
    disabled: Option<bool>,
}

#[derive(Deserialize)]
struct ContentResponse {
    #[serde(rename = "type")]
    entry_type: String,
    encoding: String,
    content: String,
    sha: String,
}

#[derive(Deserialize)]
struct DirectoryItem {
    #[serde(rename = "type")]
    entry_type: String,
    path: String,
}

#[derive(Debug)]
pub(crate) struct StorageError {
    code: String,
}

impl StorageError {
    pub(crate) fn new(code: impl Into<String>) -> Self {
        Self { code: code.into() }
    }

    fn detail(code: &str, error: impl Display) -> Self {
        eprintln!("Storage operation failed ({code}): {error}");
        Self::new(code)
    }

    pub(crate) fn code(&self) -> &str {
        &self.code
    }
}

impl Display for StorageError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "storage error: {}", self.code)
    }
}
