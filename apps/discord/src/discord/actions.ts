import { Buffer } from "node:buffer";

import {
  ButtonStyle,
  ChannelType,
  DiscordAPIError,
  MessageFlags,
  PermissionFlagsBits,
  ReactionType,
  RESTJSONErrorCodes,
  Routes,
  TextInputStyle,
  type APIEmbed,
  type APIMessage,
  type Client,
  type Guild,
  type GuildMember,
  type RESTGetAPIChannelMessageReactionUsersResult,
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
    case "sendRichMessage": {
      const channel = await getSendableChannel(client, action.channelId);
      const message = await channel.send(
        createRichMessageOptions(action.message, outgoingMessagePrefix, false),
      );
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
    case "editRichInteractionReply": {
      const interaction = takeInteraction(
        interactionRegistry,
        action.interactionId,
      );
      const message = await interaction.editReply(
        createRichMessageOptions(action.message, outgoingMessagePrefix, true),
      );
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
    case "editRichMessage": {
      const channel = await getSendableChannel(client, action.channelId);
      await channel.messages.edit(
        action.messageId,
        createRichMessageOptions(action.message, outgoingMessagePrefix, true),
      );
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
      const role = await getManageableRole(guild, action.roleId);
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
    case "updatePanelReaction": {
      const panel = await getPanelSnapshot(client, action);
      const reactions = panelReactions(panel, action.emoji);
      if (action.enabled) {
        // 未使用の番号に残っている選択を新しいロールへ引き継がない。
        if (reactions.some(({ reaction }) =>
          reaction.count > Number(reaction.me) + Number(reaction.me_burst ?? false),
        )) {
          throw new DiscordActionError("panel_reaction_slot_in_use", false);
        }
        if (!reactions.some(({ reaction }) => reaction.me)) {
          await client.rest.put(Routes.channelMessageOwnReaction(
            action.channelId, action.messageId, encodeURIComponent(action.emoji),
          ));
        }
      } else {
        for (const { name } of reactions) {
          await client.rest.delete(Routes.channelMessageReaction(
            action.channelId, action.messageId, encodeURIComponent(name),
          ));
        }
      }
      return success(null);
    }
    case "syncReactionRoles": {
      await syncReactionRoles(client, action);
      return success(null);
    }
    default:
      return assertNever(action);
  }
}

type ReactionRoleSync = Extract<CoreActionData, { type: "syncReactionRoles" }>;

async function getPanelSnapshot(
  client: Client,
  action: Pick<ReactionRoleSync, "guildId" | "channelId" | "messageId">,
): Promise<APIMessage> {
  try {
    const channel = await getSendableChannel(client, action.channelId);
    if (!("guildId" in channel) || channel.guildId !== action.guildId) {
      throw new DiscordActionError("invalid_role_panel", false);
    }
    const message = await client.rest.get(Routes.channelMessage(channel.id, action.messageId)) as APIMessage;
    if (message.author.id !== client.user?.id) {
      throw new DiscordActionError("invalid_role_panel", false);
    }
    return message;
  } catch (error) {
    if (error instanceof DiscordAPIError) {
      if (error.code === RESTJSONErrorCodes.UnknownMessage) {
        throw new DiscordActionError("role_panel_message_missing", false);
      }
      if (error.code === RESTJSONErrorCodes.UnknownChannel) {
        throw new DiscordActionError("role_panel_channel_missing", false);
      }
      if (error.code === RESTJSONErrorCodes.MissingAccess || error.code === RESTJSONErrorCodes.MissingPermissions) {
        throw new DiscordActionError("role_panel_access_denied", false);
      }
    }
    throw error;
  }
}

function panelReactions(panel: APIMessage, emoji: string) {
  // 表示指定の有無を揃えて比較し、APIにはDiscordが返した絵文字名を使う。
  const key = emoji.replaceAll("\uFE0F", "");
  return (panel.reactions ?? []).flatMap((reaction) => {
    const { id, name } = reaction.emoji;
    return !id && name?.replaceAll("\uFE0F", "") === key ? [{ reaction, name }] : [];
  });
}

async function syncReactionRoles(client: Client, action: ReactionRoleSync): Promise<void> {
  const guild = await client.guilds.fetch(action.guildId);
  const panel = await getPanelSnapshot(client, action);
  const members: GuildMember[] = [];
  if (action.userId) {
    try {
      members.push(await guild.members.fetch({ user: action.userId, force: true, cache: false }));
    } catch (error) {
      if (error instanceof DiscordAPIError && error.code === RESTJSONErrorCodes.UnknownMember) return;
      throw error;
    }
  } else {
    if (guild.memberCount > MAX_MEMBER_RESOLUTION_GUILD_SIZE) {
      throw new DiscordActionError("guild_too_large", false);
    }
    let after: string | undefined;
    const limit = 1000;
    while (true) {
      const page = await guild.members.list({ after, limit, cache: false });
      if (members.length + page.size > MAX_MEMBER_RESOLUTION_GUILD_SIZE) {
        throw new DiscordActionError("guild_too_large", false);
      }
      members.push(...page.values());
      if (page.size < limit) break;
      after = page.lastKey();
    }
  }

  let firstError: unknown;
  for (const { emoji, roleId } of action.bindings) {
    try {
      const role = await getManageableRole(guild, roleId);
      const selected = await fetchPanelReactors(client, panel, action, emoji);
      for (const member of members) {
        const hasRole = member.roles.cache.has(role.id);
        const shouldHaveRole = !member.user.bot && selected.has(member.id);
        if (hasRole === shouldHaveRole) continue;
        try {
          if (shouldHaveRole) {
            await member.roles.add(role, "KBC通知設定");
          } else {
            await member.roles.remove(role, "KBC通知設定");
          }
        } catch (error) {
          firstError ??= error;
        }
      }
    } catch (error) {
      firstError ??= error;
    }
  }
  if (firstError !== undefined) throw firstError;
}

async function fetchPanelReactors(
  client: Client,
  panel: APIMessage,
  action: ReactionRoleSync,
  emoji: string,
): Promise<Set<string>> {
  const selected = new Set<string>();
  for (const { reaction, name } of panelReactions(panel, emoji)) {
    const route = Routes.channelMessageReaction(action.channelId, action.messageId, encodeURIComponent(name));
    await collectReactionUsers(client, route, ReactionType.Normal, action.userId, selected);
    if (reaction.count_details?.burst) {
      await collectReactionUsers(client, route, ReactionType.Burst, action.userId, selected);
    }
  }
  return selected;
}

async function collectReactionUsers(
  client: Client,
  route: ReturnType<typeof Routes.channelMessageReaction>,
  type: ReactionType,
  userId: string | null,
  selected: Set<string>,
): Promise<void> {
  const limit = userId ? 1 : 100;
  let after = userId ? (BigInt(userId) - 1n).toString() : undefined;
  // 全ページを読めなければ、そのロールは一括変更しない。
  for (let offset = 0; ; offset += limit) {
    const query = new URLSearchParams({ limit: String(limit), type: String(type) });
    if (after) query.set("after", after);
    const page = await client.rest.get(route, { query }) as RESTGetAPIChannelMessageReactionUsersResult;
    if (offset + page.length > MAX_MEMBER_RESOLUTION_GUILD_SIZE) {
      throw new DiscordActionError("too_many_panel_reactors", false);
    }
    for (const user of page) {
      if (!user.bot && (!userId || user.id === userId)) selected.add(user.id);
    }
    if (userId || page.length < limit) return;
    after = page[page.length - 1]?.id;
  }
}

type RichMessage = Extract<CoreActionData, {
  type: "sendRichMessage";
}>["message"];

function createRichMessageOptions(
  message: RichMessage,
  outgoingMessagePrefix: string,
  replaceAttachments: boolean,
) {
  const files = message.attachments.map((attachment) => ({
    attachment: Buffer.from(attachment.data),
    name: attachment.fileName,
    contentType: attachment.contentType ?? undefined,
  }));
  return {
    ...(message.content === null
      ? {}
      : { content: `${outgoingMessagePrefix}${message.content}` }),
    embeds: message.embeds.map(createEmbed),
    components: createMessageRows(message.rows),
    allowedMentions: { parse: [] as never[] },
    ...(files.length === 0 ? {} : { files }),
    ...(replaceAttachments && files.length > 0 ? { attachments: [] } : {}),
  };
}

function createEmbed(embed: RichMessage["embeds"][number]): APIEmbed {
  return {
    ...(embed.title === null ? {} : { title: embed.title }),
    ...(embed.description === null ? {} : { description: embed.description }),
    ...(embed.color === null ? {} : { color: embed.color }),
    fields: embed.fields,
    ...(embed.footer === null ? {} : { footer: { text: embed.footer } }),
    ...(embed.timestamp === null ? {} : { timestamp: embed.timestamp }),
    ...(embed.imageAttachment === null
      ? {}
      : { image: { url: `attachment://${embed.imageAttachment}` } }),
  };
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
): Promise<Role> {
  const role = await guild.roles.fetch(roleId, { force: true });
  if (!role || role.id === guild.id || role.managed) {
    throw new DiscordActionError("role_unavailable", false);
  }
  if (!role.editable) {
    throw new DiscordActionError("role_not_manageable", false);
  }
  if (role.permissions.bitfield !== 0n) {
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
