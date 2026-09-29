import { Buffer } from "node:buffer";

import {
  ButtonStyle,
  ChannelType,
  MessageFlags,
  PermissionFlagsBits,
  TextInputStyle,
  type Client,
  type Guild,
  type Role,
  type SendableChannels,
} from "discord.js";

import {
  ActionRowBuilder,
  ButtonBuilder,
  ChannelSelectMenuBuilder,
  LabelBuilder,
  ModalBuilder,
  RoleSelectMenuBuilder,
  StringSelectMenuBuilder,
  TextDisplayBuilder,
  TextInputBuilder,
  UserSelectMenuBuilder,
  type MessageActionRowComponentBuilder,
} from "@discordjs/builders";

import type { ActionOutcome, CoreActionData } from "../protocol";
import type { InteractionRegistry } from "./interaction-registry";

const MAX_MEMBER_RESOLUTION_GUILD_SIZE = 5_000;
const MAX_MEMBER_RESOLUTION_SUBJECTS = 64;
const MAX_MEMBER_RESOLUTION_RESULTS = 25;

class DiscordActionError extends Error {
  public constructor(
    public readonly code: string,
    public readonly retryable: boolean,
  ) {
    super(code);
  }
}

async function getSendableChannel(
  client: Client,
  channelId: string,
): Promise<SendableChannels> {
  const channel = await client.channels.fetch(channelId);
  if (!channel?.isTextBased() || !("send" in channel)) {
    throw new DiscordActionError("channel_unavailable", false);
  }
  return channel;
}

async function getMessage(
  client: Client,
  channelId: string,
  messageId: string,
) {
  const channel = await getSendableChannel(client, channelId);
  return channel.messages.fetch(messageId);
}

export async function executeCoreAction(
  client: Client,
  action: CoreActionData,
  outgoingMessagePrefix: string,
  interactionRegistry: InteractionRegistry,
): Promise<ActionOutcome> {
  switch (action.type) {
    case "sendMessage": {
      const channel = await getSendableChannel(client, action.channelId);
      const message = await channel.send({
        content: `${outgoingMessagePrefix}${action.content}`,
        allowedMentions: { parse: [] },
      });
      return success(message.id);
    }
    case "sendInteractiveMessage": {
      const channel = await getSendableChannel(client, action.channelId);
      const message = await channel.send({
        content: `${outgoingMessagePrefix}${action.content}`,
        allowedMentions: { parse: [] },
        components: createMessageRows(action.rows),
      });
      return success(message.id);
    }
    case "replyInteraction": {
      const interaction = takeInteraction(
        interactionRegistry,
        action.interactionId,
      );
      if (interaction.deferred || interaction.replied) {
        const message = await interaction.editReply({
          content: `${outgoingMessagePrefix}${action.content}`,
          allowedMentions: { parse: [] },
          components: createMessageRows(action.rows),
        });
        return success(message.id);
      }
      const response = await interaction.reply({
        content: `${outgoingMessagePrefix}${action.content}`,
        allowedMentions: { parse: [] },
        components: createMessageRows(action.rows),
        ...(action.ephemeral ? { flags: MessageFlags.Ephemeral } : {}),
        withResponse: true,
      });
      return success(response.resource?.message?.id ?? null);
    }
    case "updateInteraction": {
      const interaction = takeInteraction(
        interactionRegistry,
        action.interactionId,
      );
      if (!interaction.isMessageComponent()) {
        throw new DiscordActionError("interaction_not_updatable", false);
      }
      if (interaction.deferred || interaction.replied) {
        const message = await interaction.editReply({
          content: `${outgoingMessagePrefix}${action.content}`,
          allowedMentions: { parse: [] },
          components: createMessageRows(action.rows),
        });
        return success(message.id);
      }
      const response = await interaction.update({
        content: `${outgoingMessagePrefix}${action.content}`,
        allowedMentions: { parse: [] },
        components: createMessageRows(action.rows),
        withResponse: true,
      });
      return success(response.resource?.message?.id ?? null);
    }
    case "editInteractionReply": {
      const interaction = takeInteraction(
        interactionRegistry,
        action.interactionId,
      );
      const message = await interaction.editReply({
        content: `${outgoingMessagePrefix}${action.content}`,
        allowedMentions: { parse: [] },
        components: createMessageRows(action.rows),
      });
      return success(message.id);
    }
    case "showModal": {
      const interaction = takeInteraction(
        interactionRegistry,
        action.interactionId,
      );
      if (!interaction.isMessageComponent()) {
        throw new DiscordActionError("interaction_cannot_show_modal", false);
      }
      await interaction.showModal(createModal(action.modal));
      return success(null);
    }
    case "sendNotification": {
      const channel = await getSendableChannel(client, action.channelId);
      const message = await channel.send({
        content: `${outgoingMessagePrefix}${action.content}`,
        allowedMentions: { parse: [], roles: action.allowedRoleIds },
        nonce: action.nonce,
        enforceNonce: true,
      });
      return success(message.id);
    }
    case "editMessage": {
      const channel = await getSendableChannel(client, action.channelId);
      await channel.messages.edit(action.messageId, {
        content: `${outgoingMessagePrefix}${action.content}`,
        allowedMentions: { parse: [] },
      });
      return success(null);
    }
    case "sendAttachment": {
      const channel = await getSendableChannel(client, action.channelId);
      const message = await channel.send({
        content: createAttachmentMessage(outgoingMessagePrefix, action.message),
        allowedMentions: { parse: [] },
        files: [{
          attachment: Buffer.from(action.data),
          name: action.fileName,
        }],
      });
      return success(message.id);
    }
    case "sendAttachmentFile": {
      const channel = await getSendableChannel(client, action.channelId);
      const message = await channel.send({
        content: createAttachmentMessage(outgoingMessagePrefix, action.message),
        allowedMentions: { parse: [] },
        files: [{ attachment: action.path, name: action.fileName }],
      });
      return success(message.id);
    }
    case "addReaction": {
      const message = await getMessage(client, action.channelId, action.messageId);
      await message.react(action.emoji);
      return success(null);
    }
    case "clearReactions": {
      const message = await getMessage(client, action.channelId, action.messageId);
      await message.reactions.removeAll();
      return success(null);
    }
    case "resolveGuildMembers": {
      if (
        action.subjectIds.length > MAX_MEMBER_RESOLUTION_SUBJECTS
        || action.maxMembers < 1
        || action.maxMembers > MAX_MEMBER_RESOLUTION_RESULTS
      ) {
        throw new DiscordActionError("invalid_member_resolution", false);
      }
      const guild = await client.guilds.fetch(action.guildId);
      if (guild.memberCount > MAX_MEMBER_RESOLUTION_GUILD_SIZE) {
        throw new DiscordActionError("guild_too_large", false);
      }
      const subjects = new Set(action.subjectIds);
      const matches = [...(await guild.members.fetch()).values()]
        .filter((member) =>
          !member.user.bot
          && (
            subjects.has(member.id)
            || member.roles.cache.some((role) => subjects.has(role.id))
          ))
        .sort((left, right) =>
          left.displayName.localeCompare(right.displayName, "ja")
          || left.id.localeCompare(right.id))
        .map((member) => ({
          userId: member.id,
          displayName: member.displayName,
        }));
      return {
        status: "membersResolved",
        members: matches.slice(0, action.maxMembers),
        truncated: matches.length > action.maxMembers,
      };
    }
    case "createGuildRole": {
      if (action.name.length < 1 || action.name.length > 100) {
        throw new DiscordActionError("invalid_role_name", false);
      }
      const guild = await client.guilds.fetch(action.guildId);
      const role = await guild.roles.create({
        name: action.name,
        permissions: [],
        mentionable: false,
        hoist: false,
        reason: "KBC通知用ロール",
      });
      return roleResolved(role);
    }
    case "resolveAssignableRole": {
      const guild = await client.guilds.fetch(action.guildId);
      const role = await getManageableRole(guild, action.roleId, true);
      return roleResolved(role);
    }
    case "resolveSendableChannel": {
      const channel = await getGuildSendableChannel(
        client,
        action.guildId,
        action.channelId,
      );
      return {
        status: "channelResolved",
        channelId: channel.id,
        name: "name" in channel ? channel.name : channel.id,
      };
    }
    case "addGuildMemberRole": {
      const guild = await client.guilds.fetch(action.guildId);
      const role = await getManageableRole(guild, action.roleId, true);
      const member = await guild.members.fetch(action.userId);
      await member.roles.add(role, "KBC通知設定");
      return success(null);
    }
    case "removeGuildMemberRole": {
      const guild = await client.guilds.fetch(action.guildId);
      const role = await getManageableRole(guild, action.roleId, false);
      const member = await guild.members.fetch(action.userId);
      await member.roles.remove(role, "KBC通知設定");
      return success(null);
    }
    default:
      return assertNever(action);
  }
}

function takeInteraction(
  registry: InteractionRegistry,
  interactionId: string,
) {
  const interaction = registry.take(interactionId);
  if (!interaction) {
    throw new DiscordActionError("interaction_expired", false);
  }
  return interaction;
}

function createMessageRows(
  rows: Extract<CoreActionData, {
    type: "sendInteractiveMessage";
  }>["rows"],
): ActionRowBuilder<MessageActionRowComponentBuilder>[] {
  return rows.map((row) => {
    const builder = new ActionRowBuilder<MessageActionRowComponentBuilder>();
    builder.addComponents(...row.components.map((component) => {
      switch (component.type) {
        case "button":
          return new ButtonBuilder()
            .setCustomId(component.customId)
            .setLabel(component.label)
            .setStyle(toButtonStyle(component.style))
            .setDisabled(component.disabled);
        case "stringSelect": {
          const select = new StringSelectMenuBuilder()
            .setCustomId(component.customId)
            .setPlaceholder(component.placeholder)
            .setMinValues(component.minValues)
            .setMaxValues(component.maxValues);
          select.addOptions(component.options.map((option) => ({
            label: option.label,
            value: option.value,
            ...(option.description ? { description: option.description } : {}),
            default: option.default,
          })));
          return select;
        }
        case "channelSelect":
          return new ChannelSelectMenuBuilder()
            .setCustomId(component.customId)
            .setPlaceholder(component.placeholder)
            .setChannelTypes(
              ChannelType.GuildText,
              ChannelType.GuildAnnouncement,
              ChannelType.PublicThread,
              ChannelType.PrivateThread,
              ChannelType.AnnouncementThread,
            )
            .setMinValues(component.minValues)
            .setMaxValues(component.maxValues);
        case "roleSelect":
          return new RoleSelectMenuBuilder()
            .setCustomId(component.customId)
            .setPlaceholder(component.placeholder)
            .setMinValues(component.minValues)
            .setMaxValues(component.maxValues);
        case "userSelect":
          return new UserSelectMenuBuilder()
            .setCustomId(component.customId)
            .setPlaceholder(component.placeholder)
            .setMinValues(component.minValues)
            .setMaxValues(component.maxValues);
        default:
          return assertNever(component);
      }
    }));
    return builder;
  });
}

function createModal(
  definition: Extract<CoreActionData, { type: "showModal" }>["modal"],
): ModalBuilder {
  const modal = new ModalBuilder()
    .setCustomId(definition.customId)
    .setTitle(definition.title);
  for (const component of definition.components) {
    switch (component.type) {
      case "textDisplay":
        modal.addTextDisplayComponents(
          new TextDisplayBuilder().setContent(component.content),
        );
        break;
      case "textInput": {
        const input = new TextInputBuilder()
          .setCustomId(component.customId)
          .setStyle(component.style === "short"
            ? TextInputStyle.Short
            : TextInputStyle.Paragraph)
          .setRequired(component.required);
        if (component.value !== null) {
          input.setValue(component.value);
        }
        if (component.placeholder !== null) {
          input.setPlaceholder(component.placeholder);
        }
        if (component.maxLength !== null) {
          input.setMaxLength(component.maxLength);
        }
        const label = new LabelBuilder()
          .setLabel(component.label)
          .setTextInputComponent(input);
        if (component.description !== null) {
          label.setDescription(component.description);
        }
        modal.addLabelComponents(label);
        break;
      }
      case "stringSelect": {
        const select = new StringSelectMenuBuilder()
          .setCustomId(component.customId)
          .setMinValues(component.minValues)
          .setMaxValues(component.maxValues)
          .addOptions(component.options.map((option) => ({
            label: option.label,
            value: option.value,
            ...(option.description ? { description: option.description } : {}),
            default: option.default,
          })));
        const label = new LabelBuilder()
          .setLabel(component.label)
          .setStringSelectMenuComponent(select);
        if (component.description !== null) {
          label.setDescription(component.description);
        }
        modal.addLabelComponents(label);
        break;
      }
      default:
        assertNever(component);
    }
  }
  return modal;
}

function toButtonStyle(
  style: "primary" | "secondary" | "success" | "danger",
): ButtonStyle {
  switch (style) {
    case "primary":
      return ButtonStyle.Primary;
    case "secondary":
      return ButtonStyle.Secondary;
    case "success":
      return ButtonStyle.Success;
    case "danger":
      return ButtonStyle.Danger;
    default:
      return assertNever(style);
  }
}

async function getGuildSendableChannel(
  client: Client,
  guildId: string,
  channelId: string,
): Promise<SendableChannels> {
  const guild = await client.guilds.fetch(guildId);
  const channel = await getSendableChannel(client, channelId);
  if (!("guildId" in channel) || channel.guildId !== guild.id) {
    throw new DiscordActionError("channel_unavailable", false);
  }
  const member = guild.members.me ?? await guild.members.fetchMe();
  const permissions = channel.permissionsFor(member);
  const canSend = channel.isThread()
    ? permissions?.has([
      PermissionFlagsBits.ViewChannel,
      PermissionFlagsBits.SendMessagesInThreads,
    ])
    : permissions?.has([
      PermissionFlagsBits.ViewChannel,
      PermissionFlagsBits.SendMessages,
    ]);
  if (!canSend) {
    throw new DiscordActionError("channel_not_sendable", false);
  }
  return channel;
}

async function getManageableRole(
  guild: Guild,
  roleId: string,
  requireNoPermissions: boolean,
): Promise<Role> {
  const role = await guild.roles.fetch(roleId);
  if (!role || role.id === guild.id || role.managed) {
    throw new DiscordActionError("role_unavailable", false);
  }
  if (!role.editable) {
    throw new DiscordActionError("role_not_manageable", false);
  }
  if (requireNoPermissions && role.permissions.bitfield !== 0n) {
    throw new DiscordActionError("role_has_permissions", false);
  }
  return role;
}

function roleResolved(role: Role): ActionOutcome {
  return { status: "roleResolved", roleId: role.id, name: role.name };
}

function success(messageId: string | null): ActionOutcome {
  return { status: "success", messageId };
}

function createAttachmentMessage(
  outgoingMessagePrefix: string,
  message: string | null,
): string | undefined {
  if (message !== null) {
    return `${outgoingMessagePrefix}${message}`;
  }

  return outgoingMessagePrefix.trimEnd() || undefined;
}

export function createActionFailure(error: unknown): ActionOutcome {
  if (error instanceof DiscordActionError) {
    return {
      status: "failure",
      code: error.code,
      retryable: error.retryable,
    };
  }

  const status = getHttpStatus(error);
  return {
    status: "failure",
    code: "discord_request_failed",
    retryable: status === undefined || status === 429 || status >= 500,
  };
}

function getHttpStatus(error: unknown): number | undefined {
  if (typeof error !== "object" || error === null || !("status" in error)) {
    return undefined;
  }
  return typeof error.status === "number" ? error.status : undefined;
}

function assertNever(value: never): never {
  throw new Error(`Unsupported CoreAction: ${JSON.stringify(value)}`);
}
