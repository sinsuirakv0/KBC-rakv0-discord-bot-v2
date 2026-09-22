//! TypeScript Discord AdapterとRust Coreの間で共有するProtocol定義を置くcrate。

use std::error::Error;
use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const PROTOCOL_VERSION: u32 = 2;

macro_rules! protocol_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize, TS)]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

protocol_id!(EventId);
protocol_id!(RequestId);
protocol_id!(ActionId);

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CoreEvent {
    pub protocol_version: u32,
    pub event_id: EventId,
    pub request_id: RequestId,
    pub event: CoreEventData,
}

impl CoreEvent {
    pub fn validate_version(&self) -> Result<(), ProtocolVersionMismatch> {
        validate_protocol_version(self.protocol_version)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CoreEventData {
    MessageCreate {
        guild_id: Option<String>,
        channel_id: String,
        message_id: String,
        user_id: String,
        member_role_ids: Vec<String>,
        content: String,
    },
    ReactionAdd {
        guild_id: Option<String>,
        channel_id: String,
        message_id: String,
        user_id: String,
        emoji: String,
    },
    ActionResult {
        action_id: ActionId,
        outcome: ActionOutcome,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(
    tag = "status",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ActionOutcome {
    Success { message_id: Option<String> },
    Failure { code: String, retryable: bool },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CoreAction {
    pub protocol_version: u32,
    pub action_id: ActionId,
    pub request_id: RequestId,
    pub action: CoreActionData,
}

impl CoreAction {
    pub fn validate_version(&self) -> Result<(), ProtocolVersionMismatch> {
        validate_protocol_version(self.protocol_version)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CoreActionData {
    SendMessage {
        channel_id: String,
        content: String,
    },
    SendNotification {
        channel_id: String,
        content: String,
        nonce: String,
    },
    EditMessage {
        channel_id: String,
        message_id: String,
        content: String,
    },
    SendAttachment {
        channel_id: String,
        file_name: String,
        content_type: Option<String>,
        message: Option<String>,
        #[ts(type = "Uint8Array")]
        data: Vec<u8>,
    },
    AddReaction {
        channel_id: String,
        message_id: String,
        emoji: String,
    },
    ClearReactions {
        channel_id: String,
        message_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeInfo {
    pub protocol_version: u32,
    pub core_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtocolVersionMismatch {
    pub expected: u32,
    pub actual: u32,
}

impl Display for ProtocolVersionMismatch {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "protocol version mismatch: expected {}, received {}",
            self.expected, self.actual
        )
    }
}

impl Error for ProtocolVersionMismatch {}

pub fn validate_protocol_version(actual: u32) -> Result<(), ProtocolVersionMismatch> {
    if actual == PROTOCOL_VERSION {
        return Ok(());
    }

    Err(ProtocolVersionMismatch {
        expected: PROTOCOL_VERSION,
        actual,
    })
}
