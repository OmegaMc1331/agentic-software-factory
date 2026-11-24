import { useCallback, useSyncExternalStore } from "react";
import { request, refreshFactorySession } from "../../../api";
import { projectPath } from "../../projects/projectApi";
import type { AgentSession } from "../../../types";

export interface Resource<T> {
  value?: T;
  error?: string;
  loading: boolean;
  updated: number;
}
type Entry = {
  snapshot: Resource<unknown>;
  listeners: Set<() => void>;
  timer?: ReturnType<typeof setTimeout>;
  abort?: AbortController;
};
export class ResourceCache {
  private entries = new Map<string, Entry>();
  constructor(
    private read: (path: string, signal: AbortSignal) => Promise<unknown> = (path, signal) =>
      request(path, undefined, { signal })
  ) {}
  private entry(key: string) {
    let entry = this.entries.get(key);
    if (!entry) {
      entry = { snapshot: { loading: true, updated: 0 }, listeners: new Set() };
      this.entries.set(key, entry);
    }
    return entry;
  }
  snapshot<T>(key: string): Resource<T> {
    return this.entry(key).snapshot as Resource<T>;
  }
  subscribe(key: string, listener: () => void) {
    const e = this.entry(key);
    e.listeners.add(listener);
    if (!e.abort && !e.timer) void this.refresh(key);
    return () => {
      e.listeners.delete(listener);
      if (!e.listeners.size) {
        clearTimeout(e.timer);
        e.timer = undefined;
        e.abort?.abort();
        e.abort = undefined;
        if (this.entries.size > 256) this.entries.delete(key);
      }
    };
  }
  async refresh(key: string) {
    const e = this.entry(key);
    if (e.abort) return;
    clearTimeout(e.timer);
    e.timer = undefined;
    const abort = new AbortController();
    e.abort = abort;
    try {
      const value = await this.read(key, abort.signal);
      if (!abort.signal.aborted) e.snapshot = { value, loading: false, updated: Date.now() };
    } catch (error) {
      if (!abort.signal.aborted)
        e.snapshot = {
          ...e.snapshot,
          loading: false,
          error: error instanceof Error ? error.message : "Request failed.",
        };
    } finally {
      if (e.abort === abort) e.abort = undefined;
      if (!abort.signal.aborted) {
        for (const listener of e.listeners) listener();
        if (e.listeners.size)
          e.timer = setTimeout(() => void this.refresh(key), /runs/.test(key) ? 2500 : 10000);
      }
    }
  }
  acknowledge<T>(key: string, value: T) {
    const entry = this.entry(key);
    entry.abort?.abort();
    entry.abort = undefined;
    clearTimeout(entry.timer);
    entry.timer = undefined;
    entry.snapshot = { value, loading: false, updated: Date.now() };
    for (const listener of entry.listeners) listener();
    if (entry.listeners.size)
      entry.timer = setTimeout(() => void this.refresh(key), /runs/.test(key) ? 2500 : 10000);
  }
  dispose() {
    for (const e of this.entries.values()) {
      clearTimeout(e.timer);
      e.abort?.abort();
    }
    this.entries.clear();
  }
}
export const resources = new ResourceCache();
export function useData<T>(projectId: string, path: string): Resource<T> {
  const key = projectId ? projectPath(projectId, path) : path;
  return useSyncExternalStore(
    useCallback((listener) => resources.subscribe(key, listener), [key]),
    useCallback(() => resources.snapshot<T>(key), [key])
  );
}
export const refreshData = (projectId: string, path: string) =>
  resources.refresh(projectId ? projectPath(projectId, path) : path);
export const acknowledgeData = <T>(projectId: string, path: string, value: T) =>
  resources.acknowledge(projectId ? projectPath(projectId, path) : path, value);

type SessionStream = {
  snapshot: Resource<AgentSession>;
  listeners: Set<() => void>;
  source?: EventSource;
  closing?: ReturnType<typeof setTimeout>;
  generation: number;
};
const sessionFinished = (session?: AgentSession) =>
  session !== undefined && session.status !== "running" && session.status !== "active";
export class SessionStreams {
  private streams = new Map<string, SessionStream>();
  private entry(projectId: string, id: number) {
    const key = projectPath(projectId, `sessions/${id}/stream`);
    let e = this.streams.get(key);
    if (!e) {
      e = { snapshot: { loading: true, updated: 0 }, listeners: new Set(), generation: 0 };
      this.streams.set(key, e);
    }
    return { key, e };
  }
  snapshot(projectId: string, id: number) {
    return this.entry(projectId, id).e.snapshot;
  }
  subscribe(projectId: string, id: number, listener: () => void) {
    const { key, e } = this.entry(projectId, id);
    clearTimeout(e.closing);
    e.listeners.add(listener);
    if (!e.source && !sessionFinished(e.snapshot.value)) void this.connect(projectId, id);
    return () => {
      e.listeners.delete(listener);
      if (!e.listeners.size)
        e.closing = setTimeout(() => {
          if (!e.listeners.size) {
            e.generation++;
            e.source?.close();
            e.source = undefined;
            this.streams.delete(key);
          }
        }, 100);
    };
  }
  async connect(projectId: string, id: number) {
    const { key, e } = this.entry(projectId, id);
    const generation = ++e.generation;
    try {
      await refreshFactorySession();
      if (generation !== e.generation || !e.listeners.size) return;
      e.source?.close();
      const source = new EventSource(`/api${key}`);
      e.source = source;
      source.addEventListener("session", (event) => {
        if (e.source !== source) return;
        try {
          const value = JSON.parse((event as MessageEvent).data) as AgentSession;
          if (value.id !== id) throw new Error("Session identity mismatch.");
          e.snapshot = { value, loading: false, updated: Date.now() };
          if (sessionFinished(value)) {
            source.close();
            e.source = undefined;
          }
        } catch {
          e.snapshot = {
            loading: false,
            error: "Invalid session event. Reconnect this view.",
            updated: 0,
          };
        }
        for (const l of e.listeners) l();
      });
      source.addEventListener("error", () => {
        if (e.source !== source) return;
        e.snapshot = {
          ...e.snapshot,
          loading: false,
          error: "Session stream disconnected. Reconnect after checking the project runtime.",
        };
        source.close();
        e.source = undefined;
        for (const l of e.listeners) l();
      });
    } catch (error) {
      if (generation === e.generation) {
        e.snapshot = {
          loading: false,
          error: error instanceof Error ? error.message : "Authentication failed.",
          updated: 0,
        };
        for (const l of e.listeners) l();
      }
    }
  }
}
export const sessionStreams = new SessionStreams();
export function useSessionStream(projectId: string, id: number) {
  return useSyncExternalStore(
    useCallback((listener) => sessionStreams.subscribe(projectId, id, listener), [projectId, id]),
    useCallback(() => sessionStreams.snapshot(projectId, id), [projectId, id])
  );
}
