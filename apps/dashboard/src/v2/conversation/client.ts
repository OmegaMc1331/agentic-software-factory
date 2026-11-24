import { useCallback, useSyncExternalStore } from "react";
import { request, refreshFactorySession } from "../../api";
import { projectPath } from "../projects/projectApi";
import type { Command, ConversationEvent, Message, Mission, Submission } from "./types";
import { browserPresence } from "../lifecycle/presence";
export const conversationPath = (project: string, id: string, suffix = "") =>
  projectPath(project, `conversations/${encodeURIComponent(id)}${suffix}`);
export class ConversationApi {
  constructor(
    readonly project: string,
    readonly id: string,
    private api: typeof request = request
  ) {}
  messages(before?: number) {
    return this.api<Message[]>(
      conversationPath(
        this.project,
        this.id,
        `/messages?limit=50${before ? `&before=${before}` : ""}`
      )
    );
  }
  missions() {
    return this.api<Mission[]>(conversationPath(this.project, this.id, "/missions"));
  }
  async send(body: Submission) {
    await browserPresence.ensureBinding(this.project, body.workspace_session_id);
    return this.api<Command>(conversationPath(this.project, this.id, "/messages"), {
      method: "POST",
      headers: { "Content-Type": "application/json", ...browserPresence.headers() },
      body: JSON.stringify(body),
    });
  }
  cancel(id: number) {
    return this.api(conversationPath(this.project, this.id, `/commands/${id}/cancel`), {
      method: "POST",
      headers: { "Content-Type": "application/json" },
    });
  }
}
interface Snapshot {
  messages: Message[];
  missions: Mission[];
  cursor: number;
  loading: boolean;
  error: string;
  connected: boolean;
  older: boolean;
}
export class ConversationStore {
  snapshot: Snapshot = {
    messages: [],
    missions: [],
    cursor: 0,
    loading: true,
    error: "",
    connected: false,
    older: false,
  };
  private listeners = new Set<() => void>();
  private source?: EventSource;
  private reconnect?: ReturnType<typeof setTimeout>;
  private stopTimer?: ReturnType<typeof setTimeout>;
  private pending?: Promise<void>;
  private refreshAgain = false;
  private generation = 0;
  readonly api: ConversationApi;
  constructor(
    readonly project: string,
    readonly id: string,
    api?: typeof request
  ) {
    this.api = new ConversationApi(project, id, api);
  }
  private publish(patch: Partial<Snapshot>) {
    this.snapshot = { ...this.snapshot, ...patch };
    for (const listener of this.listeners) listener();
  }
  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    clearTimeout(this.stopTimer);
    if (this.listeners.size === 1) {
      void this.refresh();
      if (!this.source && !this.reconnect) void this.connect();
    }
    return () => {
      this.listeners.delete(listener);
      if (!this.listeners.size) this.stopTimer = setTimeout(() => this.stop(), 100);
    };
  };
  readonly get = () => this.snapshot;
  refresh(): Promise<void> {
    if (this.pending) {
      this.refreshAgain = true;
      return this.pending;
    }
    this.pending = Promise.all([this.api.messages(), this.api.missions()])
      .then(([messages, missions]) => {
        if (messages.some((m) => m.conversation_id !== this.id))
          throw new Error("Conversation identity mismatch.");
        const byId = new Map(this.snapshot.messages.map((m) => [m.id, m]));
        for (const message of messages) byId.set(message.id, message);
        this.publish({
          messages: [...byId.values()].sort((a, b) => a.id - b.id).slice(-200),
          missions,
          loading: false,
          older: messages.length === 50,
          error: "",
        });
      })
      .catch((error) => this.publish({ loading: false, error: String(error) }))
      .finally(() => {
        this.pending = undefined;
        if (this.refreshAgain) {
          this.refreshAgain = false;
          void this.refresh();
        }
      });
    return this.pending;
  }
  async older() {
    const first = this.snapshot.messages[0]?.id;
    if (!first) return;
    try {
      const messages = await this.api.messages(first);
      const byId = new Map([...messages, ...this.snapshot.messages].map((m) => [m.id, m]));
      this.publish({
        messages: [...byId.values()].sort((a, b) => a.id - b.id).slice(0, 200),
        older: messages.length === 50,
      });
    } catch (error) {
      this.publish({ error: String(error) });
    }
  }
  accept(event: ConversationEvent) {
    if (
      event.schema_version !== 1 ||
      event.conversation_id !== this.id ||
      !Number.isSafeInteger(event.id) ||
      event.id <= this.snapshot.cursor
    )
      return;
    this.publish({ cursor: event.id });
    void this.refresh();
  }
  private async connect() {
    const generation = ++this.generation;
    try {
      await refreshFactorySession();
      if (generation !== this.generation || !this.listeners.size) return;
      const source = new EventSource(
        `/api${conversationPath(this.project, this.id, `/events?after=${this.snapshot.cursor}`)}`
      );
      this.source = source;
      source.onopen = () => {
        if (this.source === source) {
          this.publish({ connected: true, error: "" });
          void this.refresh();
        }
      };
      source.addEventListener("conversation", (event) => {
        if (this.source !== source) return;
        try {
          this.accept(JSON.parse((event as MessageEvent).data) as ConversationEvent);
        } catch {
          this.publish({ error: "Invalid conversation event. Reconnect to durable history." });
        }
      });
      source.addEventListener("resync", () => {
        void this.refresh();
      });
      source.onerror = () => {
        if (this.source === source) {
          source.close();
          this.source = undefined;
          this.publish({
            connected: false,
            error: "Conversation stream disconnected; history is retained. Reconnecting…",
          });
          this.schedule();
        }
      };
    } catch (error) {
      if (generation === this.generation) {
        this.publish({ connected: false, error: String(error) });
        this.schedule();
      }
    }
  }
  private schedule() {
    clearTimeout(this.reconnect);
    this.reconnect = setTimeout(() => {
      this.reconnect = undefined;
      if (this.listeners.size) void this.connect();
    }, 2000);
  }
  stop() {
    this.generation++;
    clearTimeout(this.reconnect);
    this.reconnect = undefined;
    this.source?.close();
    this.source = undefined;
  }
  get idle() {
    return this.listeners.size === 0;
  }
}
const stores = new Map<string, ConversationStore>();
export function conversationStore(project: string, id: string) {
  const key = `${project}:${id}`;
  let store = stores.get(key);
  if (!store) {
    if (stores.size >= 128) {
      const oldest = [...stores.entries()].find(([, store]) => store.idle);
      if (!oldest)
        throw new Error(
          "128 conversation views are subscribed. Close a view before opening another."
        );
      if (oldest) {
        oldest[1].stop();
        stores.delete(oldest[0]);
      }
    }
    store = new ConversationStore(project, id);
    stores.set(key, store);
  }
  return store;
}
export function useConversation(project: string, id: string) {
  const store = conversationStore(project, id);
  return {
    store,
    state: useSyncExternalStore(
      useCallback((listener) => store.subscribe(listener), [store]),
      store.get
    ),
  };
}
