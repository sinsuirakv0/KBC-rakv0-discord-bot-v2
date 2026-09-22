import { deepStrictEqual } from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

import { PROTOCOL_VERSION } from "./index";
import type { CoreEvent } from "./index";
import { createCore } from "./native";

const contentDirectory = resolve(__dirname, "../../../../content");

function loadContent(directory: "help" | "responses", name: string): string {
  return readFileSync(resolve(contentDirectory, directory, `${name}.txt`), "utf8")
    .replace(/^\uFEFF/, "")
    .replace(/\r\n?/g, "\n")
    .replace(/\n$/, "");
}

function createMessageEvent(id: string, content: string): CoreEvent {
  return {
    protocolVersion: PROTOCOL_VERSION,
    eventId: `event:${id}`,
    requestId: `request:${id}`,
    event: {
      type: "messageCreate",
      guildId: "guild:runtime-smoke",
      channelId: "channel:runtime-smoke",
      messageId: `message:${id}`,
      userId: "user:runtime-smoke",
      memberRoleIds: ["role:runtime-smoke"],
      content,
    },
  };
}

async function main(): Promise<void> {
  const core = await createCore({
    eventQueueCapacity: 1,
    actionQueueCapacity: 1,
  });

  const cases = [
    ["static-response", "o.HOME ignored", loadContent("responses", "home")],
    ["static-help", "o.PING HELP", loadContent("help", "ping")],
    ["help-index", "o.help ignored", loadContent("help", "index")],
  ] as const;
  for (const [id, input, expectedContent] of cases) {
    await core.submitEvent(createMessageEvent(id, input));
    deepStrictEqual(await core.nextAction(), {
      protocolVersion: PROTOCOL_VERSION,
      actionId: `action:event:${id}`,
      requestId: `request:${id}`,
      action: {
        type: "sendMessage",
        channelId: "channel:runtime-smoke",
        content: expectedContent,
      },
    });
  }

  const pendingAction = core.nextAction();
  await core.shutdown();
  deepStrictEqual(await pendingAction, null);
  process.stdout.write("Runtime smoke passed.\n");
}

void main().catch((error: unknown) => {
  process.stderr.write(`${error instanceof Error ? error.stack : String(error)}\n`);
  process.exitCode = 1;
});
