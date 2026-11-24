import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ResourceCache, SessionStreams } from "./data";
vi.mock("../../../api", () => ({
  request: vi.fn(),
  refreshFactorySession: vi.fn(async () => "authenticated"),
}));
class Source {
  static all: Source[] = [];
  listeners = new Map<string, EventListener>();
  close = vi.fn();
  constructor(readonly url: string) {
    Source.all.push(this);
  }
  addEventListener(type: string, listener: EventListener) {
    this.listeners.set(type, listener);
  }
  session(id: number, stdout: string, status = "running") {
    this.listeners.get("session")?.(
      new MessageEvent("session", {
        data: JSON.stringify({ id, stdout, agent: "codex", status }),
      })
    );
  }
  error() {
    this.listeners.get("error")?.(new Event("error"));
  }
}
const a = `p_${"a".repeat(32)}`,
  b = `p_${"b".repeat(32)}`;
beforeEach(() => {
  Source.all = [];
  vi.stubGlobal("EventSource", Source);
  vi.useFakeTimers();
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
it("a committed acknowledgement fences an older in-flight read and releases its poll on unsubscribe", async () => {
  let finish: (value: unknown) => void = () => {};
  const read = vi.fn(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      })
  );
  const cache = new ResourceCache(read);
  const listener = vi.fn();
  const stop = cache.subscribe("/application/preferences", listener);
  expect(read).toHaveBeenCalledTimes(1);
  cache.acknowledge("/application/preferences", { revision: 4 });
  finish({ revision: 3 });
  await Promise.resolve();
  await Promise.resolve();
  expect(cache.snapshot("/application/preferences").value).toEqual({ revision: 4 });
  stop();
  await vi.advanceTimersByTimeAsync(20000);
  expect(read).toHaveBeenCalledTimes(1);
  cache.dispose();
});
it("keeps identical session IDs isolated by project with one transport per resource", async () => {
  const streams = new SessionStreams();
  const one = streams.subscribe(a, 1, () => {}),
    two = streams.subscribe(a, 1, () => {}),
    other = streams.subscribe(b, 1, () => {});
  await Promise.resolve();
  await Promise.resolve();
  expect(Source.all).toHaveLength(2);
  const sa = Source.all.find((s) => s.url.includes(a))!,
    sb = Source.all.find((s) => s.url.includes(b))!;
  sa.session(1, "A only");
  sb.session(1, "B only");
  expect(streams.snapshot(a, 1).value?.stdout).toBe("A only");
  expect(streams.snapshot(b, 1).value?.stdout).toBe("B only");
  one();
  await vi.advanceTimersByTimeAsync(150);
  expect(sa.close).not.toHaveBeenCalled();
  two();
  other();
  await vi.advanceTimersByTimeAsync(150);
  expect(sa.close).toHaveBeenCalledTimes(1);
  expect(sb.close).toHaveBeenCalledTimes(1);
});
it("retains a subscription across transient view remounts and ignores stale sources", async () => {
  const streams = new SessionStreams();
  const first = streams.subscribe(a, 1, () => {});
  await Promise.resolve();
  const source = Source.all[0];
  first();
  await vi.advanceTimersByTimeAsync(50);
  const next = streams.subscribe(a, 1, () => {});
  await vi.advanceTimersByTimeAsync(100);
  expect(Source.all).toHaveLength(1);
  expect(source.close).not.toHaveBeenCalled();
  await streams.connect(a, 1);
  expect(Source.all).toHaveLength(2);
  Source.all[1].session(1, "current");
  source.session(1, "stale");
  expect(streams.snapshot(a, 1).value?.stdout).toBe("current");
  next();
  await vi.advanceTimersByTimeAsync(150);
});
it("reports transport and identity errors without stopping other project subscriptions", async () => {
  const streams = new SessionStreams();
  const one = streams.subscribe(a, 1, () => {}),
    two = streams.subscribe(b, 1, () => {});
  await Promise.resolve();
  Source.all[0].session(2, "wrong resource");
  expect(streams.snapshot(a, 1).error).toMatch(/Invalid session event/);
  expect(streams.snapshot(a, 1).value).toBeUndefined();
  Source.all[0].error();
  expect(streams.snapshot(a, 1).error).toMatch(/disconnected/);
  Source.all[1].session(1, "healthy B");
  expect(streams.snapshot(b, 1).value?.stdout).toBe("healthy B");
  expect(Source.all[1].close).not.toHaveBeenCalled();
  one();
  two();
  await vi.advanceTimersByTimeAsync(150);
});
it.each(["success", "failed", "interrupted", "cancelled"])(
  "retains a terminal %s snapshot after normal SSE completion without a false connection failure",
  async (status) => {
    const streams = new SessionStreams();
    const unsubscribe = streams.subscribe(a, 3, () => {});
    await Promise.resolve();
    const source = Source.all[0];
    source.session(3, "Durable reviewed output", status);
    expect(source.close).toHaveBeenCalledTimes(1);
    source.error();
    expect(streams.snapshot(a, 3).error).toBeUndefined();
    expect(streams.snapshot(a, 3).value?.status).toBe(status);
    expect(streams.snapshot(a, 3).value?.stdout).toBe("Durable reviewed output");
    const second = streams.subscribe(a, 3, () => {});
    await Promise.resolve();
    expect(Source.all).toHaveLength(1);
    unsubscribe();
    second();
    await vi.advanceTimersByTimeAsync(150);
  }
);
