import { request } from "../../api";

export interface SessionBinding {
  projectId: string;
  sessionId: string;
}
export interface ClientLease {
  leaseId: string;
  clientInstanceId: string;
  generation: number;
  sequence: number;
  expiresAt: string;
  heartbeatMillis: number;
  graceMillis: number;
  bindings: SessionBinding[];
}
interface PresenceSnapshot {
  connected: boolean;
  error: string;
}
export class BrowserPresence {
  private readonly clientInstanceId = `bc_${crypto.randomUUID().replaceAll("-", "")}`;
  private bindings: SessionBinding[] = [];
  private lease?: ClientLease;
  private timer?: ReturnType<typeof setTimeout>;
  private pending?: Promise<void>;
  private active = false;
  private listeners = new Set<() => void>();
  private snapshot: PresenceSnapshot = { connected: false, error: "" };
  constructor(private api: typeof request = request) {}
  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  readonly get = () => this.snapshot;
  private publish(connected: boolean, error = "") {
    this.snapshot = { connected, error };
    for (const listener of this.listeners) listener();
  }
  start() {
    this.active = true;
    void this.renew();
  }
  stop() {
    this.active = false;
    clearTimeout(this.timer);
  }
  update(bindings: SessionBinding[]) {
    if (JSON.stringify(bindings) === JSON.stringify(this.bindings)) return;
    this.bindings = bindings.slice(0, 256);
    if (this.active) void this.renew();
  }
  headers(): Record<string, string> {
    return this.lease && this.snapshot.connected ? { "X-ASF-Lease": this.lease.leaseId } : {};
  }
  async ensureBinding(projectId: string, sessionId: string) {
    if (!this.active) return;
    await this.renew();
    if (!this.lease?.bindings.some((b) => b.projectId === projectId && b.sessionId === sessionId))
      await this.renew();
    if (
      !this.snapshot.connected ||
      !this.lease?.bindings.some((b) => b.projectId === projectId && b.sessionId === sessionId)
    )
      throw new Error(
        "Browser presence is not established for this workspace session. Reconnect before submitting work."
      );
  }
  renew(): Promise<void> {
    if (this.pending) return this.pending;
    clearTimeout(this.timer);
    const bindings = this.bindings;
    this.pending = (async () => {
      try {
        const lease = this.lease;
        this.lease = await this.api<ClientLease>(
          lease ? `/browser-clients/${lease.leaseId}/heartbeat` : "/browser-clients/register",
          {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(
              lease
                ? {
                    clientInstanceId: this.clientInstanceId,
                    generation: lease.generation,
                    sequence: lease.sequence + 1,
                    bindings,
                  }
                : { clientInstanceId: this.clientInstanceId, bindings }
            ),
          }
        );
        this.publish(true);
      } catch (error) {
        this.lease = undefined;
        this.publish(
          false,
          error instanceof Error ? error.message : "Browser presence unavailable."
        );
      } finally {
        this.pending = undefined;
        if (this.active)
          this.timer = setTimeout(
            () => void this.renew(),
            JSON.stringify(bindings) === JSON.stringify(this.bindings)
              ? (this.lease?.heartbeatMillis ?? 5000)
              : 0
          );
      }
    })();
    return this.pending;
  }
}
export const browserPresence = new BrowserPresence();
