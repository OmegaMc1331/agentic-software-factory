import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type ApiModule = typeof import("./api");

let api: ApiModule;

beforeEach(async () => {
  vi.resetModules();
  api = await import("./api");
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

function sessionResponse(token = "csrf-1"): Response {
  return jsonResponse({ csrfToken: token });
}

function abortError(): DOMException {
  return new DOMException("The operation was aborted.", "AbortError");
}

function pendingResponse(init?: RequestInit): Promise<Response> {
  return new Promise<Response>((_resolve, reject) => {
    if (init?.signal?.aborted) {
      reject(abortError());
      return;
    }
    init?.signal?.addEventListener("abort", () => reject(abortError()), { once: true });
  });
}

function controlledResponse(init?: RequestInit): {
  response: Promise<Response>;
  resolve: (value: Response) => void;
} {
  let resolve!: (value: Response) => void;
  const response = new Promise<Response>((res, reject) => {
    resolve = res;
    if (init?.signal?.aborted) {
      reject(abortError());
      return;
    }
    init?.signal?.addEventListener("abort", () => reject(abortError()), { once: true });
  });
  return { response, resolve };
}

function stubFetch(
  handler: (url: string, init: RequestInit) => Promise<Response>
): ReturnType<typeof vi.fn> {
  const fetchMock = vi.fn((input: RequestInfo | URL, init?: RequestInit) =>
    handler(String(input), init ?? {})
  );
  vi.stubGlobal("fetch", fetchMock);
  return fetchMock;
}

describe("Factory API client", () => {
  it("returns JSON from a successful read", async () => {
    const fetchMock = stubFetch(async () => jsonResponse([{ id: 1, objective: "test" }]));

    await expect(api.fetchRuns()).resolves.toEqual([{ id: 1, objective: "test" }]);
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/runs",
      expect.objectContaining({ method: "GET", credentials: "same-origin" })
    );
  });

  it("reports HTTP errors with their status and detail", async () => {
    stubFetch(async () => jsonResponse({ error: "service unavailable" }, 503));

    const error = await api.fetchRuns().catch((reason: unknown) => reason);

    expect(error).toBeInstanceOf(api.FactoryApiError);
    expect(error).toMatchObject({
      kind: "http",
      status: 503,
      uncertain: false,
      message: "Factory API request failed (HTTP 503): service unavailable",
    });
  });

  it("replaces browser network noise with a known connection error", async () => {
    stubFetch(async () => {
      throw new TypeError("Failed to fetch");
    });

    const error = await api.fetchRuns().catch((reason: unknown) => reason);

    expect(error).toMatchObject({
      kind: "network",
      uncertain: false,
      message: "Could not connect to the Factory API. Check that `factory start` is running.",
    });
    expect(api.isUncertainApiError(error)).toBe(false);
  });

  it("times out a default read after five seconds and cleans up its timer", async () => {
    vi.useFakeTimers();
    const fetchMock = stubFetch((_url, init) => pendingResponse(init));

    const request = api.fetchRuns();
    const captured = request.catch((reason: unknown) => reason);
    await vi.advanceTimersByTimeAsync(api.API_TIMEOUT_POLICIES.default);
    const error = await captured;

    expect(error).toMatchObject({
      kind: "timeout",
      uncertain: false,
      message:
        "Factory API did not respond before the request deadline. Check that `factory start` is running, then retry.",
    });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(vi.getTimerCount()).toBe(0);
  });

  it("allows extended external reads to outlive the default deadline", async () => {
    vi.useFakeTimers();
    let pending!: ReturnType<typeof controlledResponse>;
    stubFetch((_url, init) => {
      pending = controlledResponse(init);
      return pending.response;
    });

    const request = api.fetchGithubStatus();
    let settled = false;
    void request.then(
      () => {
        settled = true;
      },
      () => {
        settled = true;
      }
    );

    await vi.advanceTimersByTimeAsync(5_001);
    expect(settled).toBe(false);

    pending.resolve(jsonResponse({ available: true }));
    await expect(request).resolves.toEqual({ available: true });
    expect(vi.getTimerCount()).toBe(0);
  });

  it("keeps GitHub issue imports alive beyond the default deadline", async () => {
    vi.useFakeTimers();
    let pending!: ReturnType<typeof controlledResponse>;
    const fetchMock = stubFetch((url, init) => {
      if (url === "/api/auth/session") return Promise.resolve(sessionResponse());
      pending = controlledResponse(init);
      return pending.response;
    });

    const request = api.createWorkflowFromIssue("owner/repo#42");
    await vi.advanceTimersByTimeAsync(5_001);
    expect(fetchMock.mock.calls).toContainEqual([
      "/api/runs/from-issue",
      expect.objectContaining({ method: "POST" }),
    ]);

    pending.resolve(jsonResponse({ id: 7 }, 202));
    await expect(request).resolves.toEqual({ id: 7 });
    expect(vi.getTimerCount()).toBe(0);
  });

  it("times out an external write as uncertain without retrying it", async () => {
    vi.useFakeTimers();
    let pullRequestCalls = 0;
    stubFetch((url, init) => {
      if (url === "/api/auth/session") return Promise.resolve(sessionResponse());
      if (url === "/api/runs/12/pull-request") {
        pullRequestCalls += 1;
        return pendingResponse(init);
      }
      return Promise.resolve(jsonResponse({ error: "unexpected request" }, 500));
    });

    const request = api.createPullRequest(12, { title: "Ship it" });
    const captured = request.catch((reason: unknown) => reason);
    await vi.advanceTimersByTimeAsync(1);
    await vi.advanceTimersByTimeAsync(api.API_TIMEOUT_POLICIES.externalOperation);
    const error = await captured;

    expect(error).toMatchObject({
      kind: "timeout",
      uncertain: true,
      message:
        "The request timed out. The operation may still be running or may have completed. Check its status before retrying.",
    });
    expect(api.isUncertainApiError(error)).toBe(true);
    expect(pullRequestCalls).toBe(1);
  });

  it("uses the discovery deadline for MCP connection tests", async () => {
    vi.useFakeTimers();
    let testCalls = 0;
    stubFetch((url, init) => {
      if (url === "/api/auth/session") return Promise.resolve(sessionResponse());
      if (url === "/api/mcp/servers/docs/test") {
        testCalls += 1;
        return pendingResponse(init);
      }
      return Promise.resolve(jsonResponse({ error: "unexpected request" }, 500));
    });

    const request = api.testMcpServer("docs");
    const captured = request.catch((reason: unknown) => reason);
    let settled = false;
    void request.then(
      () => {
        settled = true;
      },
      () => {
        settled = true;
      }
    );

    await vi.advanceTimersByTimeAsync(5_001);
    expect(settled).toBe(false);
    await vi.advanceTimersByTimeAsync(9_999);
    const error = await captured;

    expect(error).toMatchObject({
      kind: "timeout",
      uncertain: false,
      message:
        "Factory API did not respond before the request deadline. Check that `factory start` is running, then retry.",
    });
    expect(testCalls).toBe(1);
  });

  it("derives MCP discovery deadlines from server configuration with safe bounds", () => {
    expect(api.mcpDiscoveryRequestTimeoutMs()).toBe(15_000);
    expect(api.mcpDiscoveryRequestTimeoutMs(10_000)).toBe(15_000);
    expect(api.mcpDiscoveryRequestTimeoutMs(60_000)).toBe(65_000);
    expect(api.mcpDiscoveryRequestTimeoutMs(600_000)).toBe(605_000);
  });

  it("applies a per-server MCP discovery timeout override", async () => {
    vi.useFakeTimers();
    let pending!: ReturnType<typeof controlledResponse>;
    stubFetch((_url, init) => {
      pending = controlledResponse(init);
      return pending.response;
    });

    const timeoutMs = api.mcpDiscoveryRequestTimeoutMs(25_000);
    const request = api.fetchMcpTools("docs", { timeoutMs });
    let settled = false;
    void request.then(
      () => {
        settled = true;
      },
      () => {
        settled = true;
      }
    );

    await vi.advanceTimersByTimeAsync(29_999);
    expect(settled).toBe(false);

    pending.resolve(jsonResponse([]));
    await expect(request).resolves.toEqual([]);
  });

  it("bootstraps the CSRF session before mutations", async () => {
    let sessionCalls = 0;
    const fetchMock = stubFetch(async (url, init) => {
      if (url === "/api/auth/session") {
        sessionCalls += 1;
        return sessionResponse();
      }
      expect((init.headers as Headers).get("X-Factory-CSRF")).toBe("csrf-1");
      return jsonResponse({ id: 7 }, 202);
    });

    await expect(api.createWorkflow("Build it")).resolves.toEqual({ id: 7 });
    expect(sessionCalls).toBe(1);
    expect(fetchMock).toHaveBeenCalledWith(
      "/api/runs",
      expect.objectContaining({ method: "POST" })
    );
  });

  it("returns undefined for a 204 response", async () => {
    stubFetch(async (url) => {
      if (url === "/api/auth/session") return sessionResponse();
      return new Response(null, { status: 204 });
    });

    await expect(api.cancelWorkflow(12)).resolves.toBeUndefined();
  });

  it("rejects an invalid JSON response without guessing its contents", async () => {
    stubFetch(async () => new Response("not json", { status: 200 }));

    const error = await api.fetchRuns().catch((reason: unknown) => reason);

    expect(error).toMatchObject({
      kind: "invalid-response",
      message: "Factory API returned an invalid response.",
    });
  });

  it("rejects invalid timeout overrides before sending a request", async () => {
    const fetchMock = stubFetch(async () => jsonResponse([]));

    await expect(api.fetchRuns({ timeoutMs: 50 })).rejects.toMatchObject({
      kind: "invalid-request",
    });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("propagates caller cancellation for a read", async () => {
    stubFetch((_url, init) => pendingResponse(init));
    const controller = new AbortController();

    const request = api.fetchRuns({ signal: controller.signal });
    controller.abort();
    const error = await request.catch((reason: unknown) => reason);

    expect(error).toMatchObject({
      kind: "cancelled",
      uncertain: false,
      message: "Factory API request was cancelled.",
    });
  });

  it("marks cancellation after a write starts as an uncertain outcome", async () => {
    let postStarted!: () => void;
    const started = new Promise<void>((resolve) => {
      postStarted = resolve;
    });
    let pullRequestCalls = 0;
    stubFetch((url, init) => {
      if (url === "/api/auth/session") return Promise.resolve(sessionResponse());
      if (url === "/api/runs") {
        pullRequestCalls += 1;
        return new Promise<Response>((_resolve, reject) => {
          postStarted();
          init.signal?.addEventListener("abort", () => reject(abortError()), { once: true });
        });
      }
      return Promise.resolve(jsonResponse({}, 500));
    });

    const controller = new AbortController();
    const request = api.createWorkflow("Build it", undefined, undefined, {
      signal: controller.signal,
    });
    await started;
    controller.abort();
    const error = await request.catch((reason: unknown) => reason);

    expect(error).toMatchObject({
      kind: "cancelled",
      uncertain: true,
      message:
        "Request cancelled. The operation may still be running or may have completed. Check its status before retrying.",
    });
    expect(api.isUncertainApiError(error)).toBe(true);
    expect(pullRequestCalls).toBe(1);
  });

  it("refreshes a 401 session once and retries the read", async () => {
    let runCalls = 0;
    let sessionCalls = 0;
    stubFetch(async (url) => {
      if (url === "/api/auth/session") {
        sessionCalls += 1;
        return sessionResponse();
      }
      runCalls += 1;
      if (runCalls === 1) return jsonResponse({ error: "expired" }, 401);
      return jsonResponse([{ id: 1 }]);
    });

    await expect(api.fetchRuns()).resolves.toEqual([{ id: 1 }]);
    expect(runCalls).toBe(2);
    expect(sessionCalls).toBe(1);
  });

  it("rotates CSRF after a mutation returns 401", async () => {
    let sessionCalls = 0;
    let runCalls = 0;
    const fetchMock = stubFetch(async (url) => {
      if (url === "/api/auth/session") {
        sessionCalls += 1;
        return sessionResponse(`csrf-${sessionCalls}`);
      }
      runCalls += 1;
      if (runCalls === 1) return jsonResponse({ error: "expired" }, 401);
      return jsonResponse({ id: 7 }, 202);
    });

    await expect(api.createWorkflow("Build it")).resolves.toEqual({ id: 7 });

    const postCalls = fetchMock.mock.calls.filter(([input]) => String(input) === "/api/runs");
    const tokens = postCalls.map(([, init]) => (init?.headers as Headers).get("X-Factory-CSRF"));
    expect(tokens).toEqual(["csrf-1", "csrf-2"]);
    expect(sessionCalls).toBe(2);
  });

  it("does not retry a network failure for a mutation", async () => {
    let postCalls = 0;
    stubFetch(async (url) => {
      if (url === "/api/auth/session") return sessionResponse();
      postCalls += 1;
      throw new TypeError("Failed to fetch");
    });

    const error = await api.createWorkflow("Build it").catch((reason: unknown) => reason);

    expect(error).toMatchObject({
      kind: "network",
      uncertain: true,
      message:
        "Could not connect to the Factory API. Check that `factory start` is running; the operation may still have completed, so check its status before retrying.",
    });
    expect(api.isUncertainApiError(error)).toBe(true);
    expect(postCalls).toBe(1);
  });

  it("deduplicates concurrent session bootstrap requests", async () => {
    let authCalls = 0;
    let resolveSession!: (response: Response) => void;
    let postCalls = 0;
    stubFetch((url) => {
      if (url === "/api/auth/session") {
        authCalls += 1;
        return new Promise<Response>((resolve) => {
          resolveSession = resolve;
        });
      }
      postCalls += 1;
      return Promise.resolve(jsonResponse({ id: postCalls }, 202));
    });

    const first = api.createWorkflow("First");
    const second = api.createWorkflow("Second");
    await Promise.resolve();
    expect(authCalls).toBe(1);

    resolveSession(sessionResponse());
    await expect(Promise.all([first, second])).resolves.toEqual([{ id: 1 }, { id: 2 }]);
    expect(postCalls).toBe(2);
  });
});
