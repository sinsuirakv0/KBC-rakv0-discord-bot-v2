//! 外部更新を永続化し、Discord Actionの結果まで追跡する通知Runtime。

use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use chrono::{DateTime, Datelike, FixedOffset, SecondsFormat, Timelike, Utc};
use kbc_protocol::{
    ActionId, ActionOutcome, CoreAction, CoreActionData, PROTOCOL_VERSION, RequestId,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot, watch};
use tokio::time::timeout;

use crate::commands::skd::data_source::{ScheduleType, SkdDataSource};
use crate::services::HttpService;
use crate::storage::{NotificationCategory, StorageError, StorageService, Subscription};

const MAX_CONCURRENT_REQUESTS: usize = 4;
const MAX_DETAIL_MESSAGES: usize = 128;
const ACTION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum DetectionPhase {
    Detected,
    Types,
    Ready,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScheduleFile {
    path: String,
    hash: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScheduleSource {
    before_ref: String,
    after_ref: String,
    files: HashMap<ScheduleType, ScheduleFile>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DetectionEvent {
    version: u8,
    pub(crate) event_id: String,
    pub(crate) category: NotificationCategory,
    phase: DetectionPhase,
    detected_at: String,
    types: Vec<ScheduleType>,
    source: Option<ScheduleSource>,
}

impl DetectionEvent {
    fn parse(value: Value) -> Result<Self, NotificationError> {
        let mut event: Self =
            serde_json::from_value(value).map_err(|_| NotificationError::new("invalid-event"))?;
        event
            .validate()
            .map_err(|_| NotificationError::new("invalid-event"))?;
        let timestamp = parse_timestamp(&event.detected_at)
            .map_err(|_| NotificationError::new("invalid-event"))?;
        event.detected_at = timestamp.to_rfc3339_opts(SecondsFormat::Millis, true);
        event.types = ScheduleType::ALL
            .into_iter()
            .filter(|data_type| event.types.contains(data_type))
            .collect();
        Ok(event)
    }

    fn validate(&self) -> Result<(), StorageError> {
        if self.version != 1 || !valid_event_id(&self.event_id) {
            return Err(StorageError::new("invalid-event"));
        }
        parse_timestamp(&self.detected_at)
            .map_err(|_| StorageError::new("invalid-event-timestamp"))?;
        if self.types.len() > ScheduleType::ALL.len()
            || self.types.iter().collect::<HashSet<_>>().len() != self.types.len()
        {
            return Err(StorageError::new("invalid-event-types"));
        }
        if self.category != NotificationCategory::Skd {
            if self.phase != DetectionPhase::Detected
                || !self.types.is_empty()
                || self.source.is_some()
            {
                return Err(StorageError::new("invalid-control-event"));
            }
            return Ok(());
        }
        if self.phase == DetectionPhase::Types && self.types.is_empty() {
            return Err(StorageError::new("missing-schedule-types"));
        }
        match (&self.phase, &self.source) {
            (DetectionPhase::Ready, Some(source)) => validate_source(source, &self.types),
            (DetectionPhase::Ready, None) => Err(StorageError::new("missing-schedule-source")),
            (_, None) => Ok(()),
            (_, Some(_)) => Err(StorageError::new("unexpected-schedule-source")),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum DeliveryStatus {
    Pending,
    Attempting,
    Sent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeliveryPart {
    status: DeliveryStatus,
    attempt_id: Option<String>,
    message_id: Option<String>,
    content: Option<String>,
}

impl DeliveryPart {
    fn pending() -> Self {
        Self {
            status: DeliveryStatus::Pending,
            attempt_id: None,
            message_id: None,
            content: None,
        }
    }

    fn validate(&self) -> bool {
        self.attempt_id
            .as_ref()
            .is_none_or(|value| valid_uuid(value))
            && self
                .message_id
                .as_ref()
                .is_none_or(|value| valid_snowflake(value))
            && (self.status != DeliveryStatus::Sent || self.message_id.is_some())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeliveryRecord {
    channel_id: String,
    status: DeliveryStatus,
    attempt_id: Option<String>,
    message_id: Option<String>,
    content: Option<String>,
    follow_ups: Option<Vec<DeliveryPart>>,
}

impl DeliveryRecord {
    fn pending(channel_id: String) -> Self {
        Self {
            channel_id,
            status: DeliveryStatus::Pending,
            attempt_id: None,
            message_id: None,
            content: None,
            follow_ups: None,
        }
    }

    fn part(&self) -> DeliveryPart {
        DeliveryPart {
            status: self.status,
            attempt_id: self.attempt_id.clone(),
            message_id: self.message_id.clone(),
            content: self.content.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventRecord {
    schema_version: u8,
    pub(crate) event: DetectionEvent,
    deliveries: Vec<DeliveryRecord>,
    detail_contents: Option<Vec<String>>,
}

impl EventRecord {
    pub(crate) fn validate(&self) -> Result<(), StorageError> {
        if self.schema_version != 1 {
            return Err(StorageError::new("invalid-event-record"));
        }
        self.event.validate()?;
        let mut channels = HashSet::new();
        for delivery in &self.deliveries {
            if !valid_snowflake(&delivery.channel_id)
                || !channels.insert(delivery.channel_id.as_str())
                || !delivery.part().validate()
            {
                return Err(StorageError::new("invalid-delivery"));
            }
            if let Some(parts) = &delivery.follow_ups
                && (parts.len() != self.detail_contents.as_ref().map_or(0, Vec::len)
                    || parts.iter().any(|part| !part.validate()))
            {
                return Err(StorageError::new("invalid-delivery"));
            }
        }
        if let Some(contents) = &self.detail_contents
            && (contents.is_empty()
                || contents.len() > MAX_DETAIL_MESSAGES
                || contents
                    .iter()
                    .any(|content| content.encode_utf16().count() > 2_000))
        {
            return Err(StorageError::new("invalid-details"));
        }
        Ok(())
    }
}

pub(crate) struct NotificationService {
    storage: Arc<StorageService>,
    skd: SkdDataSource,
    action_sender: mpsc::Sender<CoreAction>,
    waiters: Mutex<HashMap<ActionId, oneshot::Sender<ActionOutcome>>>,
    request_permits: Arc<Semaphore>,
    delivery_lock: Mutex<()>,
    action_sequence: AtomicU64,
    shutdown_receiver: watch::Receiver<bool>,
}

impl NotificationService {
    pub(crate) fn new(
        storage: Arc<StorageService>,
        http: Arc<HttpService>,
        action_sender: mpsc::Sender<CoreAction>,
        shutdown_receiver: watch::Receiver<bool>,
    ) -> Self {
        Self {
            storage,
            skd: SkdDataSource::new(http),
            action_sender,
            waiters: Mutex::new(HashMap::new()),
            request_permits: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
            delivery_lock: Mutex::new(()),
            action_sequence: AtomicU64::new(1),
            shutdown_receiver,
        }
    }

    pub(crate) async fn prepare(&self) -> Result<(), NotificationError> {
        self.storage
            .initialize()
            .await
            .map_err(|error| NotificationError::detail("unavailable", error))
    }

    pub(crate) async fn submit(&self, value: Value) -> Result<(), NotificationError> {
        let event = DetectionEvent::parse(value)?;
        let _permit = Arc::clone(&self.request_permits)
            .try_acquire_owned()
            .map_err(|_| NotificationError::new("busy"))?;
        let _delivery = self.delivery_lock.lock().await;
        if *self.shutdown_receiver.borrow() {
            return Err(NotificationError::new("unavailable"));
        }
        self.deliver(event).await
    }

    pub(crate) async fn handle_action_result(
        &self,
        action_id: &ActionId,
        outcome: &ActionOutcome,
    ) -> bool {
        let waiter = self.waiters.lock().await.remove(action_id);
        if let Some(waiter) = waiter {
            let _ = waiter.send(outcome.clone());
            true
        } else {
            false
        }
    }

    async fn deliver(&self, event: DetectionEvent) -> Result<(), NotificationError> {
        let subscriptions = self
            .storage
            .subscriptions()
            .await
            .map_err(storage_failure)?;
        let event_id = event.event_id.clone();
        let record = self
            .storage
            .update_notification(&event_id, |current| {
                merge_event(current, event.clone(), &subscriptions)
            })
            .await
            .map_err(storage_failure)?;
        let active_channels = subscriptions
            .iter()
            .filter(|subscription| subscription.category == record.event.category)
            .map(|subscription| subscription.channel_id.clone())
            .collect::<HashSet<_>>();
        let channels = record
            .deliveries
            .iter()
            .filter(|delivery| active_channels.contains(&delivery.channel_id))
            .map(|delivery| delivery.channel_id.clone())
            .collect::<Vec<_>>();
        let content = format_detection(&record.event, true)?;
        let initial_content = format_detection(&record.event, false)?;
        let mut failed = false;
        let mut reconciliation = false;
        for channel_id in &channels {
            match self
                .deliver_primary(&event_id, channel_id, &initial_content, &content)
                .await
            {
                Ok(()) => {}
                Err(DeliveryFailure::Failed) => failed = true,
                Err(DeliveryFailure::Reconciliation) => reconciliation = true,
            }
        }
        if record.event.source.is_some() && !channels.is_empty() {
            let details = match self.details(&event_id).await {
                Ok(details) => details,
                Err(error) => {
                    eprintln!("Schedule notification details failed: {error}");
                    failed = true;
                    Vec::new()
                }
            };
            for channel_id in &channels {
                let can_send_details = self
                    .load_record(&event_id)
                    .await
                    .ok()
                    .and_then(|record| find_delivery(&record, channel_id).ok().cloned())
                    .is_some_and(|delivery| delivery.message_id.is_some());
                if !can_send_details {
                    continue;
                }
                for (index, detail) in details.iter().enumerate() {
                    match self
                        .deliver_follow_up(&event_id, channel_id, index, detail)
                        .await
                    {
                        Ok(()) => {}
                        Err(DeliveryFailure::Failed) => failed = true,
                        Err(DeliveryFailure::Reconciliation) => reconciliation = true,
                    }
                }
            }
        }
        if failed {
            Err(NotificationError::new("delivery-failed"))
        } else if reconciliation {
            Err(NotificationError::new("reconciliation-required"))
        } else {
            Ok(())
        }
    }

    async fn deliver_primary(
        &self,
        event_id: &str,
        channel_id: &str,
        initial_content: &str,
        final_content: &str,
    ) -> Result<(), DeliveryFailure> {
        let record = self.load_record(event_id).await?;
        let mut delivery = find_delivery(&record, channel_id)?.clone();
        if delivery.message_id.is_none() {
            if delivery.status == DeliveryStatus::Attempting {
                return Err(DeliveryFailure::Reconciliation);
            }
            let attempt_id = new_attempt_id().map_err(|_| DeliveryFailure::Failed)?;
            self.storage
                .update_notification(event_id, |current| {
                    let mut record = required_record(current)?;
                    let delivery = find_delivery_mut(&mut record, channel_id)?;
                    if delivery.status != DeliveryStatus::Pending {
                        return Err(StorageError::new("delivery-already-updated"));
                    }
                    delivery.status = DeliveryStatus::Attempting;
                    delivery.attempt_id = Some(attempt_id);
                    Ok(record)
                })
                .await
                .map_err(|_| DeliveryFailure::Failed)?;
            let nonce = notification_nonce(&format!("{event_id}:{channel_id}"));
            let outcome = self
                .perform_action(
                    event_id,
                    CoreActionData::SendNotification {
                        channel_id: channel_id.to_owned(),
                        content: initial_content.to_owned(),
                        nonce,
                    },
                )
                .await
                .map_err(|_| DeliveryFailure::Failed)?;
            let Some(message_id) = successful_message_id(outcome) else {
                return Err(DeliveryFailure::Failed);
            };
            let updated = self
                .storage
                .update_notification(event_id, |current| {
                    let mut record = required_record(current)?;
                    let delivery = find_delivery_mut(&mut record, channel_id)?;
                    delivery.status = DeliveryStatus::Sent;
                    delivery.message_id = Some(message_id);
                    delivery.content = Some(initial_content.to_owned());
                    Ok(record)
                })
                .await
                .map_err(|_| DeliveryFailure::Failed)?;
            delivery = find_delivery(&updated, channel_id)?.clone();
        }
        if delivery.content.as_deref() != Some(final_content) {
            let message_id = delivery
                .message_id
                .clone()
                .ok_or(DeliveryFailure::Reconciliation)?;
            let outcome = self
                .perform_action(
                    event_id,
                    CoreActionData::EditMessage {
                        channel_id: channel_id.to_owned(),
                        message_id,
                        content: final_content.to_owned(),
                    },
                )
                .await
                .map_err(|_| DeliveryFailure::Failed)?;
            if !matches!(outcome, ActionOutcome::Success { .. }) {
                return Err(DeliveryFailure::Failed);
            }
            self.storage
                .update_notification(event_id, |current| {
                    let mut record = required_record(current)?;
                    find_delivery_mut(&mut record, channel_id)?.content =
                        Some(final_content.to_owned());
                    Ok(record)
                })
                .await
                .map_err(|_| DeliveryFailure::Failed)?;
        }
        Ok(())
    }

    async fn details(&self, event_id: &str) -> Result<Vec<String>, NotificationError> {
        let record = self.load_record(event_id).await.map_err(delivery_error)?;
        if let Some(contents) = record.detail_contents {
            return Ok(contents);
        }
        let source = record
            .event
            .source
            .as_ref()
            .ok_or_else(|| NotificationError::new("missing-schedule-source"))?;
        let detected_at = parse_timestamp(&record.event.detected_at)
            .map_err(|_| NotificationError::new("invalid-event"))?;
        let files = record
            .event
            .types
            .iter()
            .map(|data_type| {
                let file = source
                    .files
                    .get(data_type)
                    .ok_or_else(|| NotificationError::new("invalid-event"))?;
                Ok((*data_type, file.path.clone(), file.hash.clone()))
            })
            .collect::<Result<Vec<_>, NotificationError>>()?;
        let contents = self
            .skd
            .notification_details(&source.before_ref, &source.after_ref, &files, detected_at)
            .await
            .map_err(|error| NotificationError::detail("details-unavailable", error))?;
        let saved = self
            .storage
            .update_notification(event_id, |current| {
                let mut record = required_record(current)?;
                if record.detail_contents.is_none() {
                    record.detail_contents = Some(contents.clone());
                }
                Ok(record)
            })
            .await
            .map_err(storage_failure)?;
        Ok(saved.detail_contents.unwrap_or(contents))
    }

    async fn deliver_follow_up(
        &self,
        event_id: &str,
        channel_id: &str,
        index: usize,
        content: &str,
    ) -> Result<(), DeliveryFailure> {
        let record = self
            .storage
            .update_notification(event_id, |current| {
                let mut record = required_record(current)?;
                let length = record
                    .detail_contents
                    .as_ref()
                    .ok_or_else(|| StorageError::new("missing-details"))?
                    .len();
                let delivery = find_delivery_mut(&mut record, channel_id)?;
                delivery
                    .follow_ups
                    .get_or_insert_with(|| vec![DeliveryPart::pending(); length]);
                Ok(record)
            })
            .await
            .map_err(|_| DeliveryFailure::Failed)?;
        let part = find_delivery(&record, channel_id)?
            .follow_ups
            .as_ref()
            .and_then(|parts| parts.get(index))
            .ok_or(DeliveryFailure::Failed)?
            .clone();
        if part.status == DeliveryStatus::Sent {
            return Ok(());
        }
        if part.status == DeliveryStatus::Attempting {
            return Err(DeliveryFailure::Reconciliation);
        }
        let attempt_id = new_attempt_id().map_err(|_| DeliveryFailure::Failed)?;
        self.storage
            .update_notification(event_id, |current| {
                let mut record = required_record(current)?;
                let part = find_follow_up_mut(&mut record, channel_id, index)?;
                if part.status != DeliveryStatus::Pending {
                    return Err(StorageError::new("delivery-already-updated"));
                }
                part.status = DeliveryStatus::Attempting;
                part.attempt_id = Some(attempt_id);
                Ok(record)
            })
            .await
            .map_err(|_| DeliveryFailure::Failed)?;
        let nonce = notification_nonce(&format!("{event_id}:{channel_id}:detail:{index}"));
        let outcome = self
            .perform_action(
                event_id,
                CoreActionData::SendNotification {
                    channel_id: channel_id.to_owned(),
                    content: content.to_owned(),
                    nonce,
                },
            )
            .await
            .map_err(|_| DeliveryFailure::Failed)?;
        let Some(message_id) = successful_message_id(outcome) else {
            return Err(DeliveryFailure::Failed);
        };
        self.storage
            .update_notification(event_id, |current| {
                let mut record = required_record(current)?;
                let part = find_follow_up_mut(&mut record, channel_id, index)?;
                part.status = DeliveryStatus::Sent;
                part.message_id = Some(message_id);
                part.content = Some(content.to_owned());
                Ok(record)
            })
            .await
            .map_err(|_| DeliveryFailure::Failed)?;
        Ok(())
    }

    async fn load_record(&self, event_id: &str) -> Result<EventRecord, DeliveryFailure> {
        self.storage
            .update_notification(event_id, required_record)
            .await
            .map_err(|_| DeliveryFailure::Failed)
    }

    async fn perform_action(
        &self,
        event_id: &str,
        action: CoreActionData,
    ) -> Result<ActionOutcome, NotificationError> {
        let sequence = self.action_sequence.fetch_add(1, Ordering::Relaxed);
        let action_id = ActionId::new(format!("notification:{sequence}"));
        let request_id = RequestId::new(format!("notification:{event_id}"));
        let (sender, receiver) = oneshot::channel();
        self.waiters.lock().await.insert(action_id.clone(), sender);
        if self
            .action_sender
            .send(CoreAction {
                protocol_version: PROTOCOL_VERSION,
                action_id: action_id.clone(),
                request_id,
                action,
            })
            .await
            .is_err()
        {
            self.waiters.lock().await.remove(&action_id);
            return Err(NotificationError::new("unavailable"));
        }
        let mut shutdown = self.shutdown_receiver.clone();
        let outcome = tokio::select! {
            result = timeout(ACTION_TIMEOUT, receiver) => result.ok().and_then(Result::ok),
            _ = wait_for_shutdown(&mut shutdown) => None,
        };
        self.waiters.lock().await.remove(&action_id);
        outcome.ok_or_else(|| NotificationError::new("action-result-unavailable"))
    }
}

fn merge_event(
    current: Option<EventRecord>,
    event: DetectionEvent,
    subscriptions: &[Subscription],
) -> Result<EventRecord, StorageError> {
    let Some(mut record) = current else {
        let mut channels = HashSet::new();
        let deliveries = subscriptions
            .iter()
            .filter(|subscription| subscription.category == event.category)
            .filter(|subscription| channels.insert(subscription.channel_id.as_str()))
            .map(|subscription| DeliveryRecord::pending(subscription.channel_id.clone()))
            .collect();
        return Ok(EventRecord {
            schema_version: 1,
            event,
            deliveries,
            detail_contents: None,
        });
    };
    if record.event.category != event.category {
        return Err(StorageError::new("event-category-mismatch"));
    }
    if let Some(source) = event.source {
        if record
            .event
            .source
            .as_ref()
            .is_some_and(|current| current != &source)
        {
            return Err(StorageError::new("schedule-source-mismatch"));
        }
        record.event.source = Some(source);
        record.event.phase = DetectionPhase::Ready;
        record.event.types = event.types;
    } else {
        record.event.types = ScheduleType::ALL
            .into_iter()
            .filter(|data_type| {
                record.event.types.contains(data_type) || event.types.contains(data_type)
            })
            .collect();
    }
    Ok(record)
}

fn validate_source(source: &ScheduleSource, types: &[ScheduleType]) -> Result<(), StorageError> {
    if !valid_revision(&source.before_ref)
        || !valid_revision(&source.after_ref)
        || types.is_empty()
        || source.files.len() != types.len()
    {
        return Err(StorageError::new("invalid-schedule-source"));
    }
    for data_type in types {
        let Some(file) = source.files.get(data_type) else {
            return Err(StorageError::new("invalid-schedule-source"));
        };
        let prefix = format!("raw/{}_", data_type.label());
        let timestamp = file
            .path
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(".tsv"));
        if timestamp.is_none_or(|value| {
            value.is_empty() || !value.chars().all(|character| character.is_ascii_digit())
        }) || file.hash.len() != 32
            || !file
                .hash
                .chars()
                .all(|character| character.is_ascii_hexdigit())
        {
            return Err(StorageError::new("invalid-schedule-source"));
        }
    }
    Ok(())
}

fn required_record(current: Option<EventRecord>) -> Result<EventRecord, StorageError> {
    current.ok_or_else(|| StorageError::new("missing-event-record"))
}

fn find_delivery<'a>(
    record: &'a EventRecord,
    channel_id: &str,
) -> Result<&'a DeliveryRecord, DeliveryFailure> {
    record
        .deliveries
        .iter()
        .find(|delivery| delivery.channel_id == channel_id)
        .ok_or(DeliveryFailure::Failed)
}

fn find_delivery_mut<'a>(
    record: &'a mut EventRecord,
    channel_id: &str,
) -> Result<&'a mut DeliveryRecord, StorageError> {
    record
        .deliveries
        .iter_mut()
        .find(|delivery| delivery.channel_id == channel_id)
        .ok_or_else(|| StorageError::new("missing-delivery"))
}

fn find_follow_up_mut<'a>(
    record: &'a mut EventRecord,
    channel_id: &str,
    index: usize,
) -> Result<&'a mut DeliveryPart, StorageError> {
    find_delivery_mut(record, channel_id)?
        .follow_ups
        .as_mut()
        .and_then(|parts| parts.get_mut(index))
        .ok_or_else(|| StorageError::new("missing-follow-up"))
}

fn format_detection(
    event: &DetectionEvent,
    include_types: bool,
) -> Result<String, NotificationError> {
    let timezone = FixedOffset::east_opt(9 * 60 * 60).expect("JST offset is valid");
    let date = parse_timestamp(&event.detected_at)
        .map_err(|_| NotificationError::new("invalid-event"))?
        .with_timezone(&timezone);
    let weekday =
        ["日", "月", "火", "水", "木", "金", "土"][date.weekday().num_days_from_sunday() as usize];
    let title = match event.category {
        NotificationCategory::Skd => "**スケジュール更新**",
        NotificationCategory::Ad => "adの更新を検知",
        NotificationCategory::Notice => "popup_noticeの更新を検知",
    };
    let mut lines = vec![
        title.to_owned(),
        format!(
            "検知時刻: {:04}/{:02}/{:02}({weekday}) {:02}:{:02}:{:02}",
            date.year(),
            date.month(),
            date.day(),
            date.hour(),
            date.minute(),
            date.second()
        ),
    ];
    if event.category == NotificationCategory::Skd && include_types && !event.types.is_empty() {
        lines.push(format!(
            "種類: {}",
            event
                .types
                .iter()
                .map(|data_type| data_type.label())
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    Ok(lines.join("\n"))
}

fn successful_message_id(outcome: ActionOutcome) -> Option<String> {
    match outcome {
        ActionOutcome::Success {
            message_id: Some(message_id),
        } if valid_snowflake(&message_id) => Some(message_id),
        _ => None,
    }
}

fn notification_nonce(value: &str) -> String {
    hex_encode(&Sha256::digest(value.as_bytes()))[..24].to_owned()
}

fn new_attempt_id() -> Result<String, getrandom::Error> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes)?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let value = hex_encode(&bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &value[0..8],
        &value[8..12],
        &value[12..16],
        &value[16..20],
        &value[20..32]
    ))
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.chars().enumerate().all(|(index, character)| {
            if [8, 13, 18, 23].contains(&index) {
                character == '-'
            } else {
                character.is_ascii_hexdigit()
            }
        })
}

fn valid_snowflake(value: &str) -> bool {
    !value.is_empty() && value.chars().all(|character| character.is_ascii_digit())
}

fn valid_event_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 160
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ":_-".contains(character))
}

fn valid_revision(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn parse_timestamp(value: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(value).map(|date| date.with_timezone(&Utc))
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn storage_failure(error: StorageError) -> NotificationError {
    NotificationError::detail("delivery-failed", error)
}

fn delivery_error(error: DeliveryFailure) -> NotificationError {
    match error {
        DeliveryFailure::Failed => NotificationError::new("delivery-failed"),
        DeliveryFailure::Reconciliation => NotificationError::new("reconciliation-required"),
    }
}

#[derive(Clone, Copy)]
enum DeliveryFailure {
    Failed,
    Reconciliation,
}

#[derive(Debug)]
pub struct NotificationError {
    code: &'static str,
}

impl NotificationError {
    fn new(code: &'static str) -> Self {
        Self { code }
    }

    fn detail(code: &'static str, error: impl Display) -> Self {
        eprintln!("Notification operation failed ({code}): {error}");
        Self::new(code)
    }

    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl Display for NotificationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code)
    }
}

async fn wait_for_shutdown(receiver: &mut watch::Receiver<bool>) {
    if *receiver.borrow() {
        return;
    }
    while receiver.changed().await.is_ok() {
        if *receiver.borrow() {
            return;
        }
    }
}
