import { loadDiscordAdapterConfig } from "./config";
import { DiscordAdapter } from "./discord/adapter";

export { loadDiscordAdapterConfig } from "./config";
export type { DiscordAdapterConfig } from "./config";
export { DiscordAdapter } from "./discord/adapter";
export { createCore } from "./protocol/native";
export type { NativeCore, RuntimeConfig } from "./protocol/native";
export * from "./protocol";

async function main(): Promise<void> {
  const config = loadDiscordAdapterConfig();
  const adapter = await DiscordAdapter.create(config, {
    onReady(userTag) {
      process.stdout.write(`Bot started: ${userTag}\n`);
    },
    onActionError(actionId, error) {
      console.error(`Discord action failed (${actionId}).`, error);
    },
    onNotificationError(error) {
      console.error("Notification storage unavailable; retrying.", error);
    },
    onFatalError(error) {
      console.error("Discord adapter failed.", error);
      process.exitCode = 1;
    },
    onEventDispatcherMetrics(metrics) {
      process.stdout.write(
        `Event dispatcher metrics: outstanding=${metrics.outstanding} high_water=${metrics.highWaterMark} overflow_count=${metrics.overflowCount}\n`,
      );
    },
  });

  const requestShutdown = (signal: NodeJS.Signals): void => {
    process.stdout.write(`Received ${signal}; shutting down.\n`);
    void adapter.shutdown()
      .then(() => {
        process.stdout.write("Discord adapter stopped.\n");
      })
      .catch((error: unknown) => {
        console.error("Discord adapter shutdown failed.", error);
        process.exitCode = 1;
      });
  };

  process.once("SIGINT", () => requestShutdown("SIGINT"));
  process.once("SIGTERM", () => requestShutdown("SIGTERM"));
  await adapter.start();
}

if (require.main === module) {
  void main().catch((error: unknown) => {
    console.error("Discord adapter startup failed.", error);
    process.exitCode = 1;
  });
}
