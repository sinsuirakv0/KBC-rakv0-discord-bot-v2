export interface DiscordAdapterConfig {
  discordToken: string;
  outgoingMessagePrefix: string;
  ffmpegPath?: string;
  githubData?: {
    owner: string;
    repository: string;
    branch: string;
    token: string;
  };
  eventUpdate?: {
    secret: string;
    port: number;
    host: string;
  };
  motionRender?: {
    secret: string;
    port: number;
    host: string;
  };
  northflank?: {
    token: string;
    projectId: string;
    serviceId: string;
    containerName?: string;
    uptimeStartedAt?: string;
  };
}

import ffmpegStaticPath from "ffmpeg-static";

export function loadDiscordAdapterConfig(
  environment: NodeJS.ProcessEnv = process.env,
): DiscordAdapterConfig {
  const discordToken = environment.DISCORD_TOKEN?.trim();
  if (!discordToken) {
    throw new Error("DISCORD_TOKEN is required.");
  }
  const githubDataOwner = environment.GITHUB_DATA_OWNER?.trim();
  const githubDataRepository = environment.GITHUB_DATA_REPO?.trim();
  const githubDataToken = environment.GITHUB_DATA_TOKEN?.trim();
  const hasGitHubData = Boolean(
    githubDataOwner || githubDataRepository || githubDataToken,
  );
  if (
    hasGitHubData &&
    (!githubDataOwner || !githubDataRepository || !githubDataToken)
  ) {
    throw new Error(
      "GITHUB_DATA_OWNER, GITHUB_DATA_REPO and GITHUB_DATA_TOKEN are required together.",
    );
  }
  if (
    githubDataOwner &&
    (!/^[A-Za-z0-9-]+$/.test(githubDataOwner) ||
      !/^[A-Za-z0-9_.-]+$/.test(githubDataRepository!))
  ) {
    throw new Error("Invalid GitHub data repository.");
  }
  const eventUpdateSecret = environment.EVENT_UPDATE_SECRET?.trim();
  const motionRenderSecret = environment.MOTION_RENDER_SECRET?.trim();
  if (motionRenderSecret && !/^[\x21-\x7e]{32,256}$/.test(motionRenderSecret)) {
    throw new Error("MOTION_RENDER_SECRET must contain 32-256 printable ASCII characters.");
  }
  const eventUpdatePort = Number(
    environment.EVENT_UPDATE_PORT || environment.PORT || 3000,
  );
  if (
    !Number.isInteger(eventUpdatePort) ||
    eventUpdatePort < 1 ||
    eventUpdatePort > 65535
  ) {
    throw new Error("Invalid EVENT_UPDATE_PORT.");
  }
  if (eventUpdateSecret && !githubDataOwner) {
    throw new Error("GitHub data storage is required for event updates.");
  }
  const northflankToken = environment.NORTHFLANK_API_TOKEN?.trim();
  const explicitNorthflankProjectId = environment.NORTHFLANK_PROJECT_ID?.trim();
  const explicitNorthflankServiceId = environment.NORTHFLANK_SERVICE_ID?.trim();
  const northflankProjectId = (
    explicitNorthflankProjectId || environment.NF_PROJECT_ID
  )?.trim();
  const northflankServiceId = (
    explicitNorthflankServiceId || environment.NF_OBJECT_ID
  )?.trim();
  const hasNorthflank = Boolean(
    northflankToken
      || explicitNorthflankProjectId
      || explicitNorthflankServiceId,
  );
  if (
    hasNorthflank
    && (!northflankToken || !northflankProjectId || !northflankServiceId)
  ) {
    throw new Error(
      "NORTHFLANK_API_TOKEN, project ID and service ID are required together.",
    );
  }
  if (
    hasNorthflank
    && northflankProjectId
    && (!validNorthflankId(northflankProjectId)
      || !validNorthflankId(northflankServiceId!))
  ) {
    throw new Error("Invalid Northflank project or service ID.");
  }
  const uptimeStartedAt = environment.BOT_UPTIME_STARTED_AT?.trim();
  if (uptimeStartedAt && Number.isNaN(Date.parse(uptimeStartedAt))) {
    throw new Error("BOT_UPTIME_STARTED_AT must be an ISO 8601 timestamp.");
  }

  return {
    discordToken,
    outgoingMessagePrefix: environment.NODE_ENV === "production" ? "" : "[local] ",
    ffmpegPath: environment.FFMPEG_PATH?.trim() || ffmpegStaticPath || undefined,
    githubData: githubDataOwner
      ? {
          owner: githubDataOwner,
          repository: githubDataRepository!,
          branch: environment.GITHUB_DATA_BRANCH?.trim() || "main",
          token: githubDataToken!,
      }
      : undefined,
    eventUpdate: eventUpdateSecret
      ? {
          secret: eventUpdateSecret,
          port: eventUpdatePort,
          host: environment.EVENT_UPDATE_HOST?.trim() || "0.0.0.0",
        }
      : undefined,
    motionRender: motionRenderSecret ? {
      secret: motionRenderSecret, port: eventUpdatePort,
      host: environment.EVENT_UPDATE_HOST?.trim() || "0.0.0.0",
    } : undefined,
    northflank: northflankToken
      ? {
          token: northflankToken,
          projectId: northflankProjectId!,
          serviceId: northflankServiceId!,
          containerName: environment.HOSTNAME?.trim() || undefined,
          uptimeStartedAt,
        }
      : undefined,
  };
}

function validNorthflankId(value: string): boolean {
  return /^[A-Za-z][A-Za-z0-9-]{2,53}$/.test(value);
}
