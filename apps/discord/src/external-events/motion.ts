import { timingSafeEqual } from "node:crypto";
import type { IncomingMessage, ServerResponse } from "node:http";
import type { NativeCore } from "../protocol/native";

// 生成の状態はRustが所有し、ここでは認証・有限なHTTP入出力だけを扱う。
export function createMotionHandler(secret: string | undefined, core: NativeCore, isReady: () => boolean) {
  const expected = Buffer.from(secret ?? "");
  let activeRequests = 0;
  return async (request: IncomingMessage, response: ServerResponse): Promise<boolean> => {
    if (!request.url?.startsWith("/motion-jobs")) return false;
    const reply = (status: number, value: unknown) => {
      response.writeHead(status, { "Content-Type": "application/json", "Cache-Control": "no-store" });
      response.end(JSON.stringify({ protocolVersion: 1, ...(typeof value === "string" ? { status: value } : value as object) }));
    };
    if (!secret) { reply(404, "not-found"); return true; }
    const header = request.headers.authorization;
    const supplied = Buffer.from(header?.startsWith("Bearer ") ? header.slice(7) : "");
    if (supplied.length !== expected.length || !timingSafeEqual(supplied, expected)) {
      reply(401, "unauthorized"); return true;
    }
    if (!isReady()) { reply(503, "unavailable"); return true; }
    if (activeRequests >= 4) { reply(429, "busy"); return true; }
    activeRequests++;
    const deadline = setTimeout(() => response.destroy(), 15_000);
    try {
      if (request.url === "/motion-jobs" && request.method === "POST") {
        if (!request.headers["content-type"]?.startsWith("application/json")) {
          reply(415, "unsupported-media-type"); return true;
        }
        const chunks: Buffer[] = [];
        let size = 0;
        for await (const chunk of request) {
          const bytes = Buffer.from(chunk);
          size += bytes.length;
          if (size > 16 * 1024) { reply(413, "body-too-large"); return true; }
          chunks.push(bytes);
        }
        await core.submitMotion(Buffer.concat(chunks).toString("utf8"));
        reply(202, "accepted");
        return true;
      }
      const match = /^\/motion-jobs\/([a-fA-F0-9]{40})(\/artifact)?$/.exec(request.url);
      if (!match) { reply(404, "not-found"); return true; }
      const id = match[1]!;
      if (request.method === "DELETE" && !match[2]) {
        await core.removeMotion(id);
        reply(200, "removed");
      } else if (request.method === "GET" && !match[2]) {
        const status = await core.motionStatus(id);
        reply(status ? 200 : 404, status ?? "not-found");
      } else if (request.method === "GET" && match[2]) {
        const artifact = await core.readMotionArtifact(id);
        response.writeHead(200, {
          "Content-Type": artifact.contentType, "Content-Length": artifact.data.length,
          "Cache-Control": "no-store", "X-Motion-Protocol-Version": "1", "X-Motion-File-Name": artifact.fileName,
          ...(artifact.durationMs === undefined || artifact.durationMs === null ? {} : { "X-Motion-Duration-Ms": String(artifact.durationMs) }),
        });
        // 書込完了まで枠を保持し、遅いダウンロードでBufferを無制限に増やさない。
        await new Promise<void>((resolve) => {
          response.once("close", resolve);
          response.end(artifact.data, resolve);
        });
      } else { reply(405, "method-not-allowed"); }
    } catch (error) {
      if (!response.headersSent && !response.destroyed) {
        const message = error instanceof Error ? error.message : "";
        const code = ["invalid-request", "request-conflict", "busy", "not-found", "not-ready", "artifact-unavailable"]
          .find((candidate) => message.includes(candidate)) ?? "unavailable";
        const status = code === "invalid-request" ? 400 : code === "request-conflict" || code === "not-ready" ? 409
          : code === "busy" ? 429 : code === "not-found" ? 404 : 503;
        reply(status, code);
      } else { response.destroy(); }
    } finally { clearTimeout(deadline); activeRequests--; }
    return true;
  };
}
