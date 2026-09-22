import { resolve } from "node:path";

import { PROTOCOL_VERSION } from "./index";
import type { CoreAction, CoreEvent, RuntimeInfo } from "./index";

export interface RuntimeConfig {
  eventQueueCapacity?: number;
  actionQueueCapacity?: number;
  contentDirectory?: string;
  httpMaxConcurrency?: number;
  httpRequestTimeoutMs?: number;
  httpMaxResponseBytes?: number;
  githubDataOwner?: string;
  githubDataRepository?: string;
  githubDataBranch?: string;
  githubDataToken?: string;
}

export interface NativeCore {
  submitEvent(event: CoreEvent): Promise<void>;
  prepareNotifications(): Promise<void>;
  submitDetection(value: unknown): Promise<void>;
  nextAction(): Promise<CoreAction | null>;
  shutdown(): Promise<void>;
}

interface NativeModule {
  createCore(config?: RuntimeConfig): Promise<NativeCore>;
  getRuntimeInfo(): RuntimeInfo;
}

export async function createCore(
  config: RuntimeConfig = {},
  nativePath = resolve(__dirname, "../../../../native/kbc_node.node"),
): Promise<NativeCore> {
  const nativeModule = require(nativePath) as NativeModule;
  const runtimeInfo = nativeModule.getRuntimeInfo();

  if (runtimeInfo.protocolVersion !== PROTOCOL_VERSION) {
    throw new Error(
      `Protocol version mismatch: adapter=${PROTOCOL_VERSION}, core=${runtimeInfo.protocolVersion}`,
    );
  }

  return nativeModule.createCore({
    ...config,
    contentDirectory:
      config.contentDirectory ?? resolve(__dirname, "../../../../content"),
  });
}
