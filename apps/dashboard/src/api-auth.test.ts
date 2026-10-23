import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

beforeEach(() => {
  vi.resetModules();
  vi.unstubAllGlobals();
});

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("Factory browser authentication", () => {
  it("bootstraps a session and retries a 401 response once", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(json({ error: "unauthorized" }, 401))
      .mockResolvedValueOnce(json({ csrfToken: "session-csrf" }))
      .mockResolvedValueOnce(json([]));
    vi.stubGlobal("fetch", fetchMock);
    const { fetchRuns, ensureFactorySession } = await import("./api");

    await expect(fetchRuns()).resolves.toEqual([]);
    expect(fetchMock).toHaveBeenCalledTimes(3);

    const [bootstrapUrl, bootstrapInit] = fetchMock.mock.calls[1] as [string, RequestInit];
    expect(bootstrapUrl).toBe("/api/auth/session");
    expect(bootstrapInit).toMatchObject({
      method: "POST",
      credentials: "same-origin",
      body: "{}",
    });
    expect(new Headers(bootstrapInit.headers).get("Content-Type")).toBe("application/json");

    await expect(ensureFactorySession()).resolves.toBe("session-csrf");
    expect(fetchMock).toHaveBeenCalledTimes(3);
  });

  it("rotates the session before opening a long-lived transport", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(json({ csrfToken: "first-session" }))
      .mockResolvedValueOnce(json({ csrfToken: "second-session" }));
    vi.stubGlobal("fetch", fetchMock);
    const { ensureFactorySession, refreshFactorySession } = await import("./api");

    await expect(ensureFactorySession()).resolves.toBe("first-session");
    await expect(refreshFactorySession()).resolves.toBe("second-session");
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock.mock.calls[1][0]).toBe("/api/auth/session");
  });

  it("sends the CSRF header on authenticated mutations", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(json({ csrfToken: "session-csrf" }))
      .mockResolvedValueOnce(json({ id: 1, objective: "secure" }));
    vi.stubGlobal("fetch", fetchMock);
    const { createWorkflow } = await import("./api");

    await createWorkflow("secure");

    expect(fetchMock).toHaveBeenCalledTimes(2);
    const [url, init] = fetchMock.mock.calls[1] as [string, RequestInit];
    expect(url).toBe("/api/runs");
    expect(init).toMatchObject({ method: "POST", credentials: "same-origin" });
    expect(new Headers(init.headers).get("X-Factory-CSRF")).toBe("session-csrf");
  });

  it("refreshes a stale CSRF token after a 403 and retries once", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(json({ csrfToken: "stale-csrf" }))
      .mockResolvedValueOnce(json({ error: "CSRF validation failed" }, 403))
      .mockResolvedValueOnce(json({ csrfToken: "current-csrf" }))
      .mockResolvedValueOnce(json({ id: 1, objective: "secure" }));
    vi.stubGlobal("fetch", fetchMock);
    const { createWorkflow, ensureFactorySession } = await import("./api");

    await ensureFactorySession();
    await expect(createWorkflow("secure")).resolves.toMatchObject({ id: 1 });
    expect(fetchMock).toHaveBeenCalledTimes(4);
    const [retryUrl, retryInit] = fetchMock.mock.calls[3] as [string, RequestInit];
    expect(retryUrl).toBe("/api/runs");
    expect(new Headers(retryInit.headers).get("X-Factory-CSRF")).toBe("current-csrf");
  });

  it("does not loop when authentication still fails after bootstrap", async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(json({ error: "unauthorized" }, 401))
      .mockResolvedValueOnce(json({ csrfToken: "session-csrf" }))
      .mockResolvedValueOnce(json({ error: "unauthorized" }, 401));
    vi.stubGlobal("fetch", fetchMock);
    const { fetchRuns } = await import("./api");

    await expect(fetchRuns()).rejects.toThrow("HTTP 401");
    expect(fetchMock).toHaveBeenCalledTimes(3);
  });

  it("reports a clear error when the browser session is rejected", async () => {
    vi.stubGlobal("fetch", vi.fn().mockResolvedValue(json({ error: "forbidden" }, 403)));
    const { createWorkflow } = await import("./api");

    await expect(createWorkflow("secure")).rejects.toThrow(
      "Factory API session could not be established (HTTP 403)"
    );
  });
});
