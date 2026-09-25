import { Buffer } from "node:buffer";

import type { Client, Guild, Role, SendableChannels } from "discord.js";

import type { ActionOutcome, CoreActionData } from "../protocol";

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
