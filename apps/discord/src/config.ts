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
  };
}
