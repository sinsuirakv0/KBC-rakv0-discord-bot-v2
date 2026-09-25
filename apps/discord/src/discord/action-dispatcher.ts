import type { CoreAction, CoreActionData } from "../protocol";

interface PendingAction {
  action: CoreAction;
  orderingKey: string;
}

type ActionExecutor = (action: CoreAction) => Promise<void>;
type ExecutionErrorHandler = (error: unknown) => void;

export class CoreActionDispatcher {
  private readonly queue: PendingAction[] = [];
  private readonly activeKeys = new Set<string>();
  private readonly capacityWaiters: Array<(available: boolean) => void> = [];
  private readonly idleWaiters: Array<() => void> = [];
  private activeCount = 0;
  private outstandingCount = 0;
  private isClosed = false;

  public constructor(
    private readonly executor: ActionExecutor,
    private readonly onExecutionError: ExecutionErrorHandler,
    private readonly concurrency: number,
    private readonly capacity: number,
  ) {
    if (!Number.isInteger(concurrency) || concurrency < 1) {
      throw new RangeError("Action concurrency must be a positive integer.");
    }
    if (!Number.isInteger(capacity) || capacity < concurrency) {
      throw new RangeError(
        "Action dispatcher capacity must be an integer at least equal to concurrency.",
      );
    }
  }

  public async waitForCapacity(): Promise<boolean> {
    while (!this.isClosed && this.outstandingCount >= this.capacity) {
      const available = await new Promise<boolean>((resolve) => {
        this.capacityWaiters.push(resolve);
      });
      if (!available) {
        return false;
      }
    }
    if (this.isClosed) {
      return false;
    }
    return true;
  }

  public dispatch(action: CoreAction): boolean {
    if (this.isClosed) {
      return false;
    }
    if (this.outstandingCount >= this.capacity) {
      throw new Error("Action dispatcher capacity was not reserved.");
    }

    this.outstandingCount += 1;
    this.queue.push({
      action,
      orderingKey: actionOrderingKey(action.action),
    });
    this.schedule();
    return true;
  }

  public close(): void {
    if (this.isClosed) {
      return;
    }

    this.isClosed = true;
    this.outstandingCount -= this.queue.length;
    this.queue.length = 0;
    for (const waiter of this.capacityWaiters.splice(0)) {
      waiter(false);
    }
    this.resolveIdleWaiters();
  }

  public waitForIdle(): Promise<void> {
    if (this.activeCount === 0 && this.queue.length === 0) {
      return Promise.resolve();
    }
    return new Promise<void>((resolve) => {
      this.idleWaiters.push(resolve);
    });
  }

  private schedule(): void {
    while (!this.isClosed && this.activeCount < this.concurrency) {
      const index = this.queue.findIndex(
        (pending) => !this.activeKeys.has(pending.orderingKey),
      );
      if (index < 0) {
        return;
      }

      const [pending] = this.queue.splice(index, 1);
      if (!pending) {
        return;
      }
      this.activeCount += 1;
      this.activeKeys.add(pending.orderingKey);
      void this.executor(pending.action)
        .catch((error: unknown) => {
          this.onExecutionError(error);
        })
        .finally(() => {
          this.complete(pending.orderingKey);
        });
    }
  }

  private complete(orderingKey: string): void {
    this.activeKeys.delete(orderingKey);
    this.activeCount -= 1;
    this.outstandingCount -= 1;
    this.capacityWaiters.shift()?.(true);
    this.schedule();
    this.resolveIdleWaiters();
  }

  private resolveIdleWaiters(): void {
    if (this.activeCount !== 0 || this.queue.length !== 0) {
      return;
    }
    for (const waiter of this.idleWaiters.splice(0)) {
      waiter();
    }
  }
}

function actionOrderingKey(action: CoreActionData): string {
  if (
    action.type === "resolveGuildMembers"
    || action.type === "createGuildRole"
    || action.type === "resolveAssignableRole"
    || action.type === "addGuildMemberRole"
    || action.type === "removeGuildMemberRole"
  ) {
    return `guild:${action.guildId}`;
  }
  return `channel:${action.channelId}`;
}
