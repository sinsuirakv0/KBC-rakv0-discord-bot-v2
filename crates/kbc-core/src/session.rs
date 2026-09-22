//! 複数Eventを跨ぐ短時間の対話状態を有限に管理する。

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use kbc_protocol::{
    ActionId, ActionOutcome, CoreAction, CoreActionData, PROTOCOL_VERSION, RequestId,
};
use tokio::time::Instant;

use crate::command::CommandExecutionError;

pub(crate) const DEFAULT_MAX_SESSIONS: usize = 64;
const MAX_SESSION_REACTIONS: usize = 11;
const MAX_SESSION_TTL: Duration = Duration::from_secs(5 * 60);
const REACTION_SETUP_TIMEOUT: Duration = Duration::from_secs(2 * 60);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SessionId(u64);

pub(crate) type SessionFuture =
    Pin<Box<dyn Future<Output = Result<SessionResume, CommandExecutionError>> + Send + 'static>>;

pub(crate) struct SessionContext {
    channel_id: String,
    message_id: String,
}

impl SessionContext {
    pub(crate) fn channel_id(&self) -> &str {
        &self.channel_id
    }

    pub(crate) fn message_id(&self) -> &str {
        &self.message_id
    }
}

pub(crate) struct SessionResume {
    actions: Vec<CoreActionData>,
    next: Option<SessionNext>,
}

enum SessionNext {
    SameMessage(Box<dyn SessionContinuation>),
    NewMessage(Box<dyn SessionContinuation>),
}

impl SessionResume {
    pub(crate) fn complete(actions: Vec<CoreActionData>) -> Self {
        Self {
            actions,
            next: None,
        }
    }

    pub(crate) fn continue_with(
        actions: Vec<CoreActionData>,
        next: Box<dyn SessionContinuation>,
    ) -> Self {
        Self {
            actions,
            next: Some(SessionNext::SameMessage(next)),
        }
    }

    pub(crate) fn start_new(
        actions: Vec<CoreActionData>,
        next: Box<dyn SessionContinuation>,
    ) -> Self {
        Self {
            actions,
            next: Some(SessionNext::NewMessage(next)),
        }
    }
}

pub(crate) trait SessionContinuation: Send {
    fn reactions(&self) -> &[String];

    fn resume(self: Box<Self>, context: SessionContext, reaction: String) -> SessionFuture;

    fn timeout_content(&self) -> Option<String> {
        None
    }
}

pub(crate) struct SessionRequest {
    owner_id: String,
    channel_id: String,
    ttl: Duration,
    clear_reactions_on_completion: bool,
    continuation: Box<dyn SessionContinuation>,
}

impl SessionRequest {
    pub(crate) fn new(
        owner_id: String,
        channel_id: String,
        ttl: Duration,
        continuation: Box<dyn SessionContinuation>,
    ) -> Result<Self, CommandExecutionError> {
        if owner_id.is_empty()
            || channel_id.is_empty()
            || ttl.is_zero()
            || ttl > MAX_SESSION_TTL
            || !valid_reactions(continuation.reactions())
        {
            return Err(CommandExecutionError::InvalidSession);
        }

        Ok(Self {
            owner_id,
            channel_id,
            ttl,
            clear_reactions_on_completion: false,
            continuation,
        })
    }

    pub(crate) fn clear_reactions_on_completion(mut self) -> Self {
        self.clear_reactions_on_completion = true;
        self
    }
}

pub(crate) struct SessionRegistration {
    action_id: ActionId,
    request_id: RequestId,
    request: SessionRequest,
}

impl SessionRegistration {
    pub(crate) fn new(action_id: ActionId, request_id: RequestId, request: SessionRequest) -> Self {
        Self {
            action_id,
            request_id,
            request,
        }
    }

    pub(crate) fn into_unavailable_action(self) -> CoreAction {
        CoreAction {
            protocol_version: PROTOCOL_VERSION,
            action_id: self.action_id,
            request_id: self.request_id,
            action: CoreActionData::SendMessage {
                channel_id: self.request.channel_id,
                content: "❌ 現在ほかの選択操作を処理中です。少し待ってから再実行してください"
                    .to_owned(),
            },
        }
    }
}

struct PendingSession {
    id: SessionId,
    request_id: RequestId,
    expires_at: Instant,
    request: SessionRequest,
}

pub(crate) struct SessionActivation {
    id: SessionId,
    request_id: RequestId,
    owner_id: String,
    channel_id: String,
    message_id: String,
    ttl: Duration,
    clear_reactions_on_completion: bool,
    continuation: Box<dyn SessionContinuation>,
}

struct ArmingSession {
    expires_at: Instant,
    activation: SessionActivation,
}

struct ActiveSession {
    id: SessionId,
    request_id: RequestId,
    owner_id: String,
    channel_id: String,
    message_id: String,
    ttl: Duration,
    expires_at: Instant,
    clear_reactions_on_completion: bool,
    continuation: Box<dyn SessionContinuation>,
}

pub(crate) struct SessionOutput {
    pub(crate) request_id: RequestId,
    pub(crate) actions: Vec<CoreActionData>,
    pub(crate) activation: Option<SessionActivation>,
    pub(crate) session: Option<SessionRequest>,
}

pub(crate) struct SessionExpiration {
    pub(crate) session_id: u64,
    pub(crate) request_id: RequestId,
    pub(crate) actions: Vec<CoreActionData>,
}

pub(crate) struct SessionManager {
    maximum_sessions: usize,
    next_session_id: u64,
    pending_by_action_id: HashMap<ActionId, PendingSession>,
    arming_by_action_id: HashMap<ActionId, ArmingSession>,
    active_by_message_id: HashMap<String, ActiveSession>,
}

impl SessionManager {
    pub(crate) fn new(maximum_sessions: usize) -> Self {
        Self {
            maximum_sessions,
            next_session_id: 1,
            pending_by_action_id: HashMap::new(),
            arming_by_action_id: HashMap::new(),
            active_by_message_id: HashMap::new(),
        }
    }

    pub(crate) fn register(
        &mut self,
        registration: SessionRegistration,
        now: Instant,
    ) -> Result<(), Box<SessionRegistration>> {
        if self.session_count() >= self.maximum_sessions {
            return Err(Box::new(registration));
        }

        let id = self.issue_session_id();
        let expires_at = now + registration.request.ttl;
        self.pending_by_action_id.insert(
            registration.action_id,
            PendingSession {
                id,
                request_id: registration.request_id,
                expires_at,
                request: registration.request,
            },
        );
        Ok(())
    }

    pub(crate) fn handle_action_result(
        &mut self,
        action_id: &ActionId,
        outcome: ActionOutcome,
        now: Instant,
    ) -> Option<SessionOutput> {
        if let Some(arming) = self.arming_by_action_id.remove(action_id) {
            if matches!(outcome, ActionOutcome::Success { .. }) {
                let SessionActivation {
                    id,
                    request_id,
                    owner_id,
                    channel_id,
                    message_id,
                    ttl,
                    clear_reactions_on_completion,
                    continuation,
                } = arming.activation;
                self.active_by_message_id.insert(
                    message_id.clone(),
                    ActiveSession {
                        id,
                        request_id,
                        owner_id,
                        channel_id,
                        message_id,
                        ttl,
                        expires_at: now + ttl,
                        clear_reactions_on_completion,
                        continuation,
                    },
                );
            }
            return None;
        }

        let pending = self.pending_by_action_id.remove(action_id)?;
        let ActionOutcome::Success {
            message_id: Some(message_id),
        } = outcome
        else {
            return None;
        };

        let SessionRequest {
            owner_id,
            channel_id,
            ttl,
            clear_reactions_on_completion,
            continuation,
        } = pending.request;
        let reactions = continuation.reactions().to_vec();
        let actions = reactions
            .into_iter()
            .map(|emoji| CoreActionData::AddReaction {
                channel_id: channel_id.clone(),
                message_id: message_id.clone(),
                emoji,
            })
            .collect();
        Some(SessionOutput {
            request_id: pending.request_id.clone(),
            actions,
            activation: Some(SessionActivation {
                id: pending.id,
                request_id: pending.request_id,
                owner_id,
                channel_id,
                message_id,
                ttl,
                clear_reactions_on_completion,
                continuation,
            }),
            session: None,
        })
    }

    pub(crate) fn register_activation(
        &mut self,
        action_id: ActionId,
        activation: SessionActivation,
        now: Instant,
    ) {
        self.arming_by_action_id.insert(
            action_id,
            ArmingSession {
                expires_at: now + REACTION_SETUP_TIMEOUT,
                activation,
            },
        );
    }

    pub(crate) async fn handle_reaction(
        &mut self,
        channel_id: &str,
        message_id: &str,
        user_id: &str,
        emoji: &str,
        _now: Instant,
    ) -> Option<SessionOutput> {
        let session = self.active_by_message_id.get(message_id)?;
        if session.channel_id != channel_id
            || session.owner_id != user_id
            || !session
                .continuation
                .reactions()
                .iter()
                .any(|reaction| reaction == emoji)
        {
            return None;
        }

        let session = self.active_by_message_id.remove(message_id)?;
        let mut actions = if session.clear_reactions_on_completion {
            vec![CoreActionData::ClearReactions {
                channel_id: session.channel_id.clone(),
                message_id: session.message_id.clone(),
            }]
        } else {
            Vec::new()
        };
        let context = SessionContext {
            channel_id: session.channel_id.clone(),
            message_id: session.message_id.clone(),
        };
        let resume = match session.continuation.resume(context, emoji.to_owned()).await {
            Ok(resume) => resume,
            Err(error) => {
                eprintln!("Session continuation failed: {error}");
                SessionResume::complete(vec![CoreActionData::SendMessage {
                    channel_id: session.channel_id.clone(),
                    content: "❌ 選択結果の処理に失敗しました".to_owned(),
                }])
            }
        };
        actions.extend(resume.actions);

        let mut next_session = None;
        let activation = match resume.next {
            Some(SessionNext::SameMessage(continuation))
                if valid_reactions(continuation.reactions()) =>
            {
                actions.extend(continuation.reactions().iter().cloned().map(|emoji| {
                    CoreActionData::AddReaction {
                        channel_id: session.channel_id.clone(),
                        message_id: session.message_id.clone(),
                        emoji,
                    }
                }));
                Some(SessionActivation {
                    id: session.id,
                    request_id: session.request_id.clone(),
                    owner_id: session.owner_id.clone(),
                    channel_id: session.channel_id.clone(),
                    message_id: session.message_id.clone(),
                    ttl: session.ttl,
                    clear_reactions_on_completion: session.clear_reactions_on_completion,
                    continuation,
                })
            }
            Some(SessionNext::NewMessage(continuation))
                if valid_reactions(continuation.reactions()) =>
            {
                next_session = Some(SessionRequest {
                    owner_id: session.owner_id.clone(),
                    channel_id: session.channel_id.clone(),
                    ttl: session.ttl,
                    clear_reactions_on_completion: session.clear_reactions_on_completion,
                    continuation,
                });
                None
            }
            Some(_) => {
                eprintln!("Session continuation produced invalid reactions");
                None
            }
            None => None,
        };

        Some(SessionOutput {
            request_id: session.request_id,
            actions,
            activation,
            session: next_session,
        })
    }

    pub(crate) fn next_expiration(&self) -> Option<Instant> {
        self.pending_by_action_id
            .values()
            .map(|session| (session.expires_at, session.id))
            .chain(
                self.arming_by_action_id
                    .values()
                    .map(|session| (session.expires_at, session.activation.id)),
            )
            .chain(
                self.active_by_message_id
                    .values()
                    .map(|session| (session.expires_at, session.id)),
            )
            .min()
            .map(|(expires_at, _)| expires_at)
    }

    pub(crate) fn expire(&mut self, now: Instant) -> Vec<SessionExpiration> {
        self.pending_by_action_id
            .retain(|_, session| session.expires_at > now);
        self.arming_by_action_id
            .retain(|_, session| session.expires_at > now);

        let mut expired_message_ids = self
            .active_by_message_id
            .iter()
            .filter(|(_, session)| session.expires_at <= now)
            .map(|(message_id, session)| (session.id, message_id.clone()))
            .collect::<Vec<_>>();
        expired_message_ids.sort_by_key(|(id, _)| *id);
        expired_message_ids
            .into_iter()
            .filter_map(|(_, message_id)| self.active_by_message_id.remove(&message_id))
            .map(|session| {
                let mut actions = Vec::new();
                if session.clear_reactions_on_completion {
                    actions.push(CoreActionData::ClearReactions {
                        channel_id: session.channel_id.clone(),
                        message_id: session.message_id.clone(),
                    });
                }
                if let Some(content) = session.continuation.timeout_content() {
                    actions.push(CoreActionData::EditMessage {
                        channel_id: session.channel_id,
                        message_id: session.message_id,
                        content,
                    });
                }
                SessionExpiration {
                    session_id: session.id.0,
                    request_id: session.request_id,
                    actions,
                }
            })
            .collect()
    }

    fn session_count(&self) -> usize {
        self.pending_by_action_id.len()
            + self.arming_by_action_id.len()
            + self.active_by_message_id.len()
    }

    fn issue_session_id(&mut self) -> SessionId {
        let id = SessionId(self.next_session_id);
        self.next_session_id = self.next_session_id.wrapping_add(1).max(1);
        id
    }
}

fn valid_reactions(reactions: &[String]) -> bool {
    let unique_reactions = reactions.iter().collect::<HashSet<_>>();
    !reactions.is_empty()
        && reactions.len() <= MAX_SESSION_REACTIONS
        && reactions.iter().all(|reaction| !reaction.is_empty())
        && unique_reactions.len() == reactions.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestContinuation {
        reactions: Vec<String>,
    }

    struct TimeoutContinuation {
        reactions: Vec<String>,
    }

    impl SessionContinuation for TestContinuation {
        fn reactions(&self) -> &[String] {
            &self.reactions
        }

        fn resume(self: Box<Self>, _context: SessionContext, reaction: String) -> SessionFuture {
            Box::pin(async move {
                Ok(SessionResume::complete(vec![CoreActionData::SendMessage {
                    channel_id: "channel".to_owned(),
                    content: reaction,
                }]))
            })
        }
    }

    impl SessionContinuation for TimeoutContinuation {
        fn reactions(&self) -> &[String] {
            &self.reactions
        }

        fn resume(self: Box<Self>, _context: SessionContext, _reaction: String) -> SessionFuture {
            Box::pin(async move { Ok(SessionResume::complete(Vec::new())) })
        }

        fn timeout_content(&self) -> Option<String> {
            Some("受付終了".to_owned())
        }
    }

    #[tokio::test]
    async fn activates_and_consumes_owned_reaction_session() {
        let now = Instant::now();
        let request = SessionRequest::new(
            "owner".to_owned(),
            "channel".to_owned(),
            Duration::from_secs(30),
            Box::new(TestContinuation {
                reactions: vec!["1️⃣".to_owned()],
            }),
        )
        .unwrap()
        .clear_reactions_on_completion();
        let registration = SessionRegistration::new(
            ActionId::new("action:prompt"),
            RequestId::new("request:command"),
            request,
        );
        let mut manager = SessionManager::new(1);
        assert!(manager.register(registration, now).is_ok());

        let reactions = manager
            .handle_action_result(
                &ActionId::new("action:prompt"),
                ActionOutcome::Success {
                    message_id: Some("message".to_owned()),
                },
                now,
            )
            .unwrap();
        assert!(matches!(
            reactions.actions.as_slice(),
            [CoreActionData::AddReaction { emoji, .. }] if emoji == "1️⃣"
        ));
        manager.register_activation(
            ActionId::new("action:reaction"),
            reactions.activation.unwrap(),
            now,
        );
        assert!(
            manager
                .handle_reaction("channel", "message", "owner", "1️⃣", now)
                .await
                .is_none()
        );
        assert!(
            manager
                .handle_action_result(
                    &ActionId::new("action:reaction"),
                    ActionOutcome::Success { message_id: None },
                    now + Duration::from_secs(40),
                )
                .is_none()
        );
        assert!(
            manager
                .handle_reaction(
                    "channel",
                    "message",
                    "other",
                    "1️⃣",
                    now + Duration::from_secs(65),
                )
                .await
                .is_none()
        );

        let selection = manager
            .handle_reaction(
                "channel",
                "message",
                "owner",
                "1️⃣",
                now + Duration::from_secs(65),
            )
            .await
            .unwrap();
        assert!(matches!(
            selection.actions.as_slice(),
            [
                CoreActionData::ClearReactions { message_id, .. },
                CoreActionData::SendMessage { content, .. }
            ] if message_id == "message" && content == "1️⃣"
        ));
        assert!(
            manager
                .handle_reaction("channel", "message", "owner", "1️⃣", now)
                .await
                .is_none()
        );
    }

    #[test]
    fn expires_active_session_with_cleanup_actions() {
        let now = Instant::now();
        let request = SessionRequest::new(
            "owner".to_owned(),
            "channel".to_owned(),
            Duration::from_secs(30),
            Box::new(TimeoutContinuation {
                reactions: vec!["▶️".to_owned()],
            }),
        )
        .unwrap()
        .clear_reactions_on_completion();
        let mut manager = SessionManager::new(1);
        assert!(
            manager
                .register(
                    SessionRegistration::new(
                        ActionId::new("action:prompt"),
                        RequestId::new("request:command"),
                        request,
                    ),
                    now,
                )
                .is_ok()
        );
        let setup = manager
            .handle_action_result(
                &ActionId::new("action:prompt"),
                ActionOutcome::Success {
                    message_id: Some("message".to_owned()),
                },
                now,
            )
            .unwrap();
        manager.register_activation(
            ActionId::new("action:reaction"),
            setup.activation.unwrap(),
            now,
        );
        manager.handle_action_result(
            &ActionId::new("action:reaction"),
            ActionOutcome::Success { message_id: None },
            now,
        );

        let expired = manager.expire(now + Duration::from_secs(31));
        assert!(matches!(
            expired[0].actions.as_slice(),
            [
                CoreActionData::ClearReactions { message_id, .. },
                CoreActionData::EditMessage { content, .. }
            ] if message_id == "message" && content == "受付終了"
        ));
    }
}
