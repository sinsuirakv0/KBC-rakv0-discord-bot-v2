//! TypeScript Discord AdapterとRust Coreの間で共有するProtocol定義を置くcrate。

use std::error::Error;
use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const PROTOCOL_VERSION: u32 = 7;

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct InteractionField {
    pub custom_id: String,
    pub values: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ButtonStyle {
    Primary,
    Secondary,
    Success,
    Danger,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SelectOption {
    pub label: String,
    pub value: String,
    pub description: Option<String>,
    pub default: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum MessageComponentData {
    Button {
        custom_id: String,
        label: String,
        style: ButtonStyle,
        disabled: bool,
    },
    StringSelect {
        custom_id: String,
        placeholder: String,
        options: Vec<SelectOption>,
        min_values: u8,
        max_values: u8,
    },
    ChannelSelect {
        custom_id: String,
        placeholder: String,
        min_values: u8,
        max_values: u8,
    },
    RoleSelect {
        custom_id: String,
        placeholder: String,
        min_values: u8,
        max_values: u8,
    },
    UserSelect {
        custom_id: String,
        placeholder: String,
        min_values: u8,
        max_values: u8,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MessageComponentRow {
    pub components: Vec<MessageComponentData>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MessageAttachment {
    pub file_name: String,
    pub content_type: Option<String>,
    #[ts(type = "Uint8Array")]
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MessageEmbedField {
    pub name: String,
    pub value: String,
    pub inline: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct MessageEmbed {
    pub title: Option<String>,
    pub description: Option<String>,
    pub color: Option<u32>,
    pub fields: Vec<MessageEmbedField>,
    pub footer: Option<String>,
    pub timestamp: Option<String>,
    pub image_attachment: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct RichMessage {
    pub content: Option<String>,
    pub embeds: Vec<MessageEmbed>,
    pub rows: Vec<MessageComponentRow>,
    pub attachments: Vec<MessageAttachment>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub enum ModalTextInputStyle {
    Short,
    Paragraph,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ModalComponentData {
    TextDisplay {
        content: String,
    },
    TextInput {
        custom_id: String,
        label: String,
        description: Option<String>,
        style: ModalTextInputStyle,
        required: bool,
        value: Option<String>,
        placeholder: Option<String>,
        max_length: Option<u16>,
    },
    StringSelect {
        custom_id: String,
        label: String,
        description: Option<String>,
        options: Vec<SelectOption>,
        min_values: u8,
        max_values: u8,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ModalDefinition {
    pub custom_id: String,
    pub title: String,
    pub components: Vec<ModalComponentData>,
}

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
    ReactionRemove {
        guild_id: Option<String>,
        channel_id: String,
        message_id: String,
        user_id: String,
        emoji: String,
    },
    ComponentInteraction {
        interaction_id: String,
        guild_id: Option<String>,
        channel_id: String,
        message_id: String,
        user_id: String,
        member_role_ids: Vec<String>,
        custom_id: String,
        values: Vec<String>,
    },
    ModalSubmit {
        interaction_id: String,
        guild_id: Option<String>,
        channel_id: String,
        user_id: String,
        member_role_ids: Vec<String>,
        custom_id: String,
        fields: Vec<InteractionField>,
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
    Success {
        message_id: Option<String>,
    },
    MembersResolved {
        members: Vec<ResolvedGuildMember>,
        truncated: bool,
    },
    RoleResolved {
        role_id: String,
        name: String,
    },
    ChannelResolved {
        channel_id: String,
        name: String,
    },
    Failure {
        code: String,
        retryable: bool,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedGuildMember {
    pub user_id: String,
    pub display_name: String,
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
    SendInteractiveMessage {
        channel_id: String,
        content: String,
        rows: Vec<MessageComponentRow>,
    },
    SendRichMessage {
        channel_id: String,
        message: RichMessage,
    },
    ReplyInteraction {
        interaction_id: String,
        content: String,
        ephemeral: bool,
        rows: Vec<MessageComponentRow>,
    },
    UpdateInteraction {
        interaction_id: String,
        content: String,
        rows: Vec<MessageComponentRow>,
    },
    EditInteractionReply {
        interaction_id: String,
        content: String,
        rows: Vec<MessageComponentRow>,
    },
    EditRichInteractionReply {
        interaction_id: String,
        message: RichMessage,
    },
    ShowModal {
        interaction_id: String,
        modal: ModalDefinition,
    },
    SendNotification {
        channel_id: String,
        content: String,
        nonce: String,
        allowed_role_ids: Vec<String>,
    },
    EditMessage {
        channel_id: String,
        message_id: String,
        content: String,
    },
    EditRichMessage {
        channel_id: String,
        message_id: String,
        message: RichMessage,
    },
    SendAttachment {
        channel_id: String,
        file_name: String,
        content_type: Option<String>,
        message: Option<String>,
        #[ts(type = "Uint8Array")]
        data: Vec<u8>,
    },
    SendAttachmentFile {
        channel_id: String,
        file_name: String,
        content_type: Option<String>,
        message: Option<String>,
        path: String,
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
    ResolveGuildMembers {
        guild_id: String,
        subject_ids: Vec<String>,
        max_members: u16,
    },
    CreateGuildRole {
        guild_id: String,
        name: String,
    },
    ResolveAssignableRole {
        guild_id: String,
        role_id: String,
    },
    ResolveSendableChannel {
        guild_id: String,
        channel_id: String,
    },
    AddGuildMemberRole {
        guild_id: String,
        user_id: String,
        role_id: String,
    },
    RemoveGuildMemberRole {
        guild_id: String,
        user_id: String,
        role_id: String,
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
