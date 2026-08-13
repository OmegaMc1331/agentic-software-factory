import { afterEach, describe, expect, it, vi } from "vitest";
import { fetchRuns } from "./api";

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("Factory API client", () => {
  it("reports HTTP errors with their status", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(
        new Response(JSON.stringify({ error: "service unavailable" }), {
          status: 503,
        })
      )
    );

    await expect(fetchRuns()).rejects.toThrow(
      "Factory API request failed (HTTP 503): service unavailable"
    );
  });

  it("replaces browser network noise with a useful connection error", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));

    await expect(fetchRuns()).rejects.toThrow(
      "Could not connect to the Factory API. Check that `factory start` is running."
    );
  });

  it("aborts a stalled request after five seconds", async () => {
    vi.useFakeTimers();
    vi.stubGlobal(
      "fetch",
      vi.fn((_input: RequestInfo | URL, init?: RequestInit) => {
        return new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener("abort", () => {
            reject(new DOMException("The operation was aborted.", "AbortError"));
          });
        });
      })
    );

    const request = fetchRuns();
    const rejection = expect(request).rejects.toThrow(
      "Factory API did not respond. Check that `factory start` is still running."
    );
    await vi.advanceTimersByTimeAsync(5000);
    await rejection;
  });
});
