import {
  Client,
  Events,
  GatewayIntentBits,
  Partials,
  type Message,
  type MessageReaction,
  type PartialMessageReaction,
  type PartialUser,
  type User,
} from "discord.js";
import type { Server } from "node:http";

import type { DiscordAdapterConfig } from "../config";
import { createEventUpdateServer } from "../external-events/server";
import { createCore, type NativeCore } from "../protocol/native";
import type { ActionOutcome, CoreEvent } from "../protocol";
import { createActionFailure, executeCoreAction } from "./actions";
import { CoreEventDispatcher } from "./event-dispatcher";
import {
  createActionResultEvent,
  createMessageCreateEvent,
  createReactionAddEvent,
} from "./events";

const EVENT_BUFFER_CAPACITY = 64;

export interface DiscordAdapterCallbacks {
  onReady(userTag: string): void;
  onActionError(actionId: string, error: unknown): void;
  onNotificationError(error: unknown): void;
  onFatalError(error: unknown): void;
}

export class DiscordAdapter {
  private readonly eventDispatcher: CoreEventDispatcher;
  private actionLoop: Promise<void> | undefined;
  private shutdownPromise: Promise<void> | undefined;
  private notificationServer: Server | undefined;
  private notificationRetry: NodeJS.Timeout | undefined;
  private notificationsReady = false;
  private isStarted = false;
  private isStopping = false;

  private constructor(
    private readonly config: DiscordAdapterConfig,
    private readonly core: NativeCore,
    private readonly client: Client,
    private readonly callbacks: DiscordAdapterCallbacks,
  ) {
    this.eventDispatcher = new CoreEventDispatcher(core, EVENT_BUFFER_CAPACITY);
  }

  public static async create(
    config: DiscordAdapterConfig,
    callbacks: DiscordAdapterCallbacks,
  ): Promise<DiscordAdapter> {
    const core = await createCore({
      ffmpegPath: config.ffmpegPath,
      ...(config.githubData
        ? {
            githubDataOwner: config.githubData.owner,
            githubDataRepository: config.githubData.repository,
            githubDataBranch: config.githubData.branch,
            githubDataToken: config.githubData.token,
          }
        : {}),
    });
    const client = new Client({
      intents: [
        GatewayIntentBits.Guilds,
        GatewayIntentBits.GuildMembers,
        GatewayIntentBits.GuildMessages,
        GatewayIntentBits.MessageContent,
        GatewayIntentBits.GuildMessageReactions,
      ],
      partials: [Partials.Channel, Partials.Message, Partials.Reaction],
    });

    return new DiscordAdapter(config, core, client, callbacks);
  }

  public async start(): Promise<void> {
    if (this.isStarted) {
      throw new Error("Discord adapter is already started.");
    }

    this.isStarted = true;
    this.client.once(Events.ClientReady, this.handleReady);
    this.client.on(Events.MessageCreate, this.handleMessageCreate);
    this.client.on(Events.MessageReactionAdd, this.handleReactionAdd);
    this.actionLoop = this.consumeActions().catch((error: unknown) => {
      this.fail(error);
    });

    try {
      await this.client.login(this.config.discordToken);
    } catch (error) {
      await this.shutdown();
      throw error;
    }
  }

  public shutdown(): Promise<void> {
    this.shutdownPromise ??= this.performShutdown();
    return this.shutdownPromise;
  }

  private readonly handleReady = (): void => {
    this.callbacks.onReady(this.client.user?.tag ?? "unknown");
    this.startEventUpdates();
  };

  private startEventUpdates(): void {
    const config = this.config.eventUpdate;
    if (!config || this.notificationServer) {
      return;
    }
    this.notificationServer = createEventUpdateServer({
      secret: config.secret,
      core: this.core,
      isReady: () => this.client.isReady() && this.notificationsReady,
    });
    this.notificationServer.on("error", (error: unknown) => this.fail(error));
    this.notificationServer.listen(config.port, config.host);
    void this.prepareNotifications();
  }

  private async prepareNotifications(): Promise<void> {
    try {
      await this.core.prepareNotifications();
      this.notificationsReady = true;
    } catch (error) {
      this.notificationsReady = false;
      this.callbacks.onNotificationError(error);
      if (!this.isStopping) {
        this.notificationRetry = setTimeout(
          () => void this.prepareNotifications(),
          30_000,
        );
        this.notificationRetry.unref();
      }
    }
  }

  private readonly handleMessageCreate = (message: Message): void => {
    if (message.author.bot || this.isStopping) {
      return;
    }
    this.dispatchEvent(createMessageCreateEvent(message));
  };

  private readonly handleReactionAdd = (
    reaction: MessageReaction | PartialMessageReaction,
    user: User | PartialUser,
  ): void => {
    if (user.bot || this.isStopping) {
      return;
    }
    this.dispatchEvent(createReactionAddEvent(reaction, user));
  };

  private dispatchEvent(event: CoreEvent): void {
    void this.eventDispatcher.submit(event).catch((error: unknown) => {
      this.fail(error);
    });
  }

  private async consumeActions(): Promise<void> {
    while (!this.isStopping) {
      const action = await this.core.nextAction();
      if (!action || this.isStopping) {
        return;
      }

      let outcome: ActionOutcome;
      try {
        outcome = await executeCoreAction(
          this.client,
          action.action,
          this.config.outgoingMessagePrefix,
        );
      } catch (error) {
        this.callbacks.onActionError(action.actionId, error);
        outcome = createActionFailure(error);
      }

      if (this.isStopping) {
        return;
      }
      await this.eventDispatcher.submit(createActionResultEvent(action, outcome));
    }
  }

  private fail(error: unknown): void {
    if (this.isStopping) {
      return;
    }

    this.callbacks.onFatalError(error);
    void this.shutdown().catch((shutdownError: unknown) => {
      this.callbacks.onFatalError(shutdownError);
    });
  }

  private async performShutdown(): Promise<void> {
    this.isStopping = true;
    this.notificationsReady = false;
    if (this.notificationRetry) {
      clearTimeout(this.notificationRetry);
      this.notificationRetry = undefined;
    }
    const serverClose = this.notificationServer
      ? closeServer(this.notificationServer)
      : Promise.resolve();
    this.client.off(Events.ClientReady, this.handleReady);
    this.client.off(Events.MessageCreate, this.handleMessageCreate);
    this.client.off(Events.MessageReactionAdd, this.handleReactionAdd);
    this.client.destroy();
    this.eventDispatcher.close();

    let shutdownError: unknown;
    try {
      await this.core.shutdown();
    } catch (error) {
      shutdownError = error;
    }
    await serverClose;
    await this.actionLoop;

    if (shutdownError !== undefined) {
      throw shutdownError;
    }
  }
}

function closeServer(server: Server): Promise<void> {
  return new Promise((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
}
