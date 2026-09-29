import { randomUUID } from "node:crypto";

import type {
  Message,
  MessageComponentInteraction,
  MessageReaction,
  ModalSubmitInteraction,
  PartialMessageReaction,
  PartialUser,
  User,
} from "discord.js";
import { ComponentType } from "discord.js";

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
  return createReactionEvent("reactionAdd", reaction, user);
}

export function createReactionRemoveEvent(
  reaction: MessageReaction | PartialMessageReaction,
  user: User | PartialUser,
): CoreEvent {
  return createReactionEvent("reactionRemove", reaction, user);
}

export function createComponentInteractionEvent(
  interaction: MessageComponentInteraction,
): CoreEvent {
  return createEvent({
    type: "componentInteraction",
    interactionId: interaction.id,
    guildId: interaction.guildId,
    channelId: interaction.channelId,
    userId: interaction.user.id,
    memberRoleIds: getMemberRoleIds(interaction.member),
    customId: interaction.customId,
    values: interaction.isAnySelectMenu() ? [...interaction.values] : [],
  });
}

export function createModalSubmitEvent(
  interaction: ModalSubmitInteraction,
): CoreEvent {
  if (!interaction.channelId) {
    throw new Error("Settings modal submission has no channel");
  }
  const fields = [...interaction.fields.fields.values()].flatMap((field) => {
    if (field.type === ComponentType.TextInput) {
      return [{ customId: field.customId, values: [field.value] }];
    }
    if (field.type === ComponentType.StringSelect) {
      return [{ customId: field.customId, values: [...field.values] }];
    }
    return [];
  });
  return createEvent({
    type: "modalSubmit",
    interactionId: interaction.id,
    guildId: interaction.guildId,
    channelId: interaction.channelId,
    userId: interaction.user.id,
    memberRoleIds: getMemberRoleIds(interaction.member),
    customId: interaction.customId,
    fields,
  });
}

function createReactionEvent(
  type: "reactionAdd" | "reactionRemove",
  reaction: MessageReaction | PartialMessageReaction,
  user: User | PartialUser,
): CoreEvent {
  return createEvent({
    type,
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

function getMemberRoleIds(
  member:
    | MessageComponentInteraction["member"]
    | ModalSubmitInteraction["member"],
): string[] {
  if (!member) {
    return [];
  }
  return "cache" in member.roles
    ? [...member.roles.cache.keys()]
    : [...member.roles];
}
