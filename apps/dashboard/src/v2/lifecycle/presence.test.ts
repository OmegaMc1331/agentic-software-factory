import { afterEach, describe, expect, it, vi } from "vitest";
import { BrowserPresence, type ClientLease } from "./presence";
import type { request } from "../../api";

afterEach(() => vi.useRealTimers());
function fixture() {
  const calls: Array<{ path: string; body: Record<string, unknown> }> = [];
  const api = vi.fn(async (path: string, init?: RequestInit) => {
    const body = JSON.parse(String(init?.body)) as Record<string, unknown>;
    calls.push({ path, body });
    return {
      leaseId: "bl_test",
      clientInstanceId: body.clientInstanceId,
      sequence: body.sequence ?? 0,
      generation: 1,
      expiresAt: "2026-11-02T00:00:00Z",
      heartbeatMillis: 1000,
      graceMillis: 5000,
      bindings: body.bindings,
    } as ClientLease;
  });
  return { presence: new BrowserPresence(api as typeof request), api, calls };
}
describe("browser presence ownership", () => {
  it("registers once and renews using ordered server lease identity", async () => {
    vi.useFakeTimers();
    const { presence, calls } = fixture();
    presence.start();
    await presence.renew();
    expect(calls).toHaveLength(1);
    expect(presence.headers()).toEqual({ "X-ASF-Lease": "bl_test" });
    await vi.advanceTimersByTimeAsync(1000);
    expect(calls[1].body.sequence).toBe(1);
    expect(calls[1].path).toBe("/browser-clients/bl_test/heartbeat");
    presence.stop();
  });
  it("keeps all session bindings across project selection and ignores visibility events", async () => {
    vi.useFakeTimers();
    const { presence, calls } = fixture();
    const bindings = [
      { projectId: "A", sessionId: "X" },
      { projectId: "B", sessionId: "Y" },
    ];
    presence.update(bindings);
    presence.start();
    await presence.renew();
    document.dispatchEvent(new Event("visibilitychange"));
    window.dispatchEvent(new Event("blur"));
    expect(calls).toHaveLength(1);
    await presence.ensureBinding("A", "X");
    expect(calls.at(-1)?.body.bindings).toEqual(bindings);
    presence.stop();
    expect(calls.every((c) => !c.path.endsWith("release"))).toBe(true);
  });
  it("re-registers after an expired lease without claiming mission suspension", async () => {
    vi.useFakeTimers();
    const { presence, api, calls } = fixture();
    presence.start();
    await presence.renew();
    api.mockRejectedValueOnce(new Error("browser_lease_expired"));
    await presence.renew();
    expect(presence.get().connected).toBe(false);
    expect(presence.headers()).toEqual({});
    await presence.renew();
    expect(calls.at(-1)?.path).toBe("/browser-clients/register");
    expect(presence.get().connected).toBe(true);
    presence.stop();
  });
  it("does not treat component unmount as explicit session closure", async () => {
    vi.useFakeTimers();
    const { presence, calls } = fixture();
    presence.start();
    await presence.renew();
    presence.stop();
    await vi.advanceTimersByTimeAsync(60_000);
    expect(calls).toHaveLength(1);
  });
  it("refuses a mission binding that the server did not acknowledge", async () => {
    vi.useFakeTimers();
    const { presence } = fixture();
    presence.start();
    await presence.renew();
    await expect(presence.ensureBinding("B", "unowned")).rejects.toThrow("not established");
    presence.stop();
  });
});
