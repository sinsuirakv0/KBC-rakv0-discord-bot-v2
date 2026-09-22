import { timingSafeEqual } from "node:crypto";
import { createServer, type Server } from "node:http";

import type { NativeCore } from "../protocol/native";

const BODY_LIMIT_BYTES = 16 * 1024;
const REQUEST_TIMEOUT_MS = 15_000;

export function createEventUpdateServer(options: {
  secret: string;
  core: NativeCore;
  isReady(): boolean;
}): Server {
  const expectedSecret = Buffer.from(options.secret);
  const server = createServer(async (request, response) => {
    const reply = (status: number, result: string): void => {
      response.writeHead(status, { "Content-Type": "application/json" });
      response.end(JSON.stringify({ status: result }));
    };

    if (request.url === "/health/live" && request.method === "GET") {
      reply(200, "alive");
      return;
    }
    if (request.url === "/health" && request.method === "GET") {
      const ready = options.isReady();
      reply(ready ? 200 : 503, ready ? "ready" : "unavailable");
      return;
    }
    if (request.url !== "/event-update") {
      reply(404, "not-found");
      return;
    }
    if (request.method !== "POST") {
      reply(405, "method-not-allowed");
      return;
    }
    const suppliedHeader = request.headers["x-event-update-secret"];
    const suppliedSecret = Buffer.from(
      typeof suppliedHeader === "string" ? suppliedHeader : "",
    );
    if (
      suppliedSecret.length !== expectedSecret.length ||
      !timingSafeEqual(suppliedSecret, expectedSecret)
    ) {
      reply(401, "unauthorized");
      return;
    }
    if (!options.isReady()) {
      reply(503, "unavailable");
      return;
    }
    if (!request.headers["content-type"]?.startsWith("application/json")) {
      reply(415, "unsupported-media-type");
      return;
    }

    let value: unknown;
    try {
      const chunks: Buffer[] = [];
      let size = 0;
      for await (const chunk of request) {
        const bytes = Buffer.from(chunk);
        size += bytes.length;
        if (size > BODY_LIMIT_BYTES) {
          reply(413, "body-too-large");
          return;
        }
        chunks.push(bytes);
      }
      value = JSON.parse(Buffer.concat(chunks).toString("utf8"));
    } catch {
      reply(400, "invalid-event");
      return;
    }

    try {
      await options.core.submitDetection(value);
      reply(200, "accepted");
    } catch (error) {
      const code = notificationErrorCode(error);
      if (code === "invalid-event") {
        reply(400, code);
      } else if (code === "reconciliation-required") {
        reply(409, code);
      } else if (code === "busy") {
        reply(429, code);
      } else {
        reply(503, "delivery-failed");
      }
    }
  });
  server.requestTimeout = REQUEST_TIMEOUT_MS;
  server.headersTimeout = REQUEST_TIMEOUT_MS;
  return server;
}

function notificationErrorCode(error: unknown): string {
  if (!(error instanceof Error)) {
    return "unknown";
  }
  const knownCodes = [
    "invalid-event",
    "reconciliation-required",
    "busy",
    "unavailable",
    "delivery-failed",
  ];
  return knownCodes.find((code) => error.message.includes(code)) ?? "unknown";
}
