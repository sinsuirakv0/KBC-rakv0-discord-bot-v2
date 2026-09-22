import { randomUUID } from "node:crypto";

import type {
  Message,
  MessageReaction,
  PartialMessageReaction,
  PartialUser,
  User,
} from "discord.js";

import {
  PROTOCOL_VERSION,
  type ActionOutcome,
  type CoreAction,
  type CoreEvent,
  type CoreEventData,
  type RequestId,
} from "../protocol";

function createEvent(event: CoreEventData, requestId?: RequestId): CoreEvent {
  return {
    protocolVersion: PROTOCOL_VERSION,
    eventId: `event:${randomUUID()}`,
    requestId: requestId ?? `request:${randomUUID()}`,
    event,
  };
}

export function createMessageCreateEvent(message: Message): CoreEvent {
  return createEvent({
    type: "messageCreate",
    guildId: message.guildId,
    channelId: message.channelId,
    messageId: message.id,
    userId: message.author.id,
    memberRoleIds: message.member ? [...message.member.roles.cache.keys()] : [],
    content: message.content,
  });
}

export function createReactionAddEvent(
  reaction: MessageReaction | PartialMessageReaction,
  user: User | PartialUser,
): CoreEvent {
  return createEvent({
    type: "reactionAdd",
    guildId: reaction.message.guildId,
    channelId: reaction.message.channelId,
    messageId: reaction.message.id,
    userId: user.id,
    emoji: reaction.emoji.id
      ? reaction.emoji.identifier
      : (reaction.emoji.name ?? reaction.emoji.identifier),
  });
}

export function createActionResultEvent(
  action: CoreAction,
  outcome: ActionOutcome,
): CoreEvent {
  return createEvent(
    {
      type: "actionResult",
      actionId: action.actionId,
      outcome,
    },
    action.requestId,
  );
}
