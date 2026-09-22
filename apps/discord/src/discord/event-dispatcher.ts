import type { NativeCore } from "../protocol/native";
import type { CoreEvent } from "../protocol";

interface PendingEvent {
  event: CoreEvent;
  resolve: () => void;
  reject: (reason: unknown) => void;
}

export class CoreEventDispatcher {
  private readonly queue: PendingEvent[] = [];
  private outstandingCount = 0;
  private isDraining = false;
  private isClosed = false;

  public constructor(
    private readonly core: NativeCore,
    private readonly capacity: number,
  ) {
    if (!Number.isInteger(capacity) || capacity < 1) {
      throw new RangeError("Event dispatcher capacity must be a positive integer.");
    }
  }

  public submit(event: CoreEvent): Promise<void> {
    if (this.isClosed) {
      return Promise.reject(new Error("Event dispatcher is closed."));
    }
    if (this.outstandingCount >= this.capacity) {
      return Promise.reject(new Error("Event dispatcher capacity exceeded."));
    }

    this.outstandingCount += 1;
    return new Promise<void>((resolve, reject) => {
      this.queue.push({ event, resolve, reject });
      void this.drain();
    });
  }

  public close(reason: unknown = new Error("Event dispatcher is closed.")): void {
    if (this.isClosed) {
      return;
    }

    this.isClosed = true;
    for (const pending of this.queue.splice(0)) {
      this.outstandingCount -= 1;
      pending.reject(reason);
    }
  }

  private async drain(): Promise<void> {
    if (this.isDraining) {
      return;
    }

    this.isDraining = true;
    try {
      while (!this.isClosed) {
        const pending = this.queue.shift();
        if (!pending) {
          return;
        }

        try {
          await this.core.submitEvent(pending.event);
          pending.resolve();
        } catch (error) {
          pending.reject(error);
          this.outstandingCount -= 1;
          this.close(error);
          return;
        }
        this.outstandingCount -= 1;
      }
    } finally {
      this.isDraining = false;
    }
  }
}
