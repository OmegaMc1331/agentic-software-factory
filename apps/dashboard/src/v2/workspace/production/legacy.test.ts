import { afterEach, expect, it, vi } from "vitest";
it("accepts the existing empty 202 terminal-stop response without inventing a JSON failure", async () => {
  vi.resetModules();
  history.replaceState({}, "", "/");
  const fetch = vi.fn(async (input: string) =>
    input === "/api/auth/session"
      ? new Response(JSON.stringify({ csrfToken: "fixture-csrf" }), {
          status: 200,
          headers: { "Content-Type": "application/json" },
        })
      : new Response(null, { status: 202 })
  );
  vi.stubGlobal("fetch", fetch);
  const api = await import("../../../api");
  await expect(api.stopInteractiveAgentSession(1)).resolves.toBeUndefined();
  expect(fetch.mock.calls[1][0]).toBe("/api/sessions/1");
});
afterEach(() => {
  vi.unstubAllGlobals();
  vi.resetModules();
  history.replaceState({}, "", "/");
});
it("binds an explicit legacy project once while authentication stays application-scoped", async () => {
  const id = `p_${"b".repeat(32)}`;
  history.replaceState({}, "", `/?project=${id}`);
  vi.resetModules();
  const fetch = vi.fn(
    async (input: string) =>
      new Response(
        JSON.stringify(input === "/api/auth/session" ? { csrfToken: "c".repeat(64) } : []),
        { status: 200, headers: { "Content-Type": "application/json" } }
      )
  );
  vi.stubGlobal("fetch", fetch);
  const api = await import("../../../api");
  await api.fetchRunsPage();
  await api.refreshFactorySession();
  expect(fetch.mock.calls.map((call) => call[0])).toEqual([
    `/api/projects/${id}/runs?limit=25`,
    "/api/auth/session",
  ]);
  expect(api.agentSessionStreamUrl(1)).toBe(`/api/projects/${id}/sessions/1/stream`);
  expect(api.agentTerminalSocketUrl(1)).toContain(`/api/projects/${id}/sessions/1/terminal`);
});
it("does not let a URL project parameter change the production workspace global API client", async () => {
  history.replaceState({}, "", `/workspace.html?project=p_${"a".repeat(32)}`);
  vi.resetModules();
  const fetch = vi
    .fn<(input: string) => Promise<Response>>()
    .mockResolvedValue(new Response("[]", { status: 200 }));
  vi.stubGlobal("fetch", fetch);
  const api = await import("../../../api");
  await api.request("/projects");
  expect(fetch.mock.calls[0][0]).toBe("/api/projects");
});
it("malformed explicit legacy scope cannot silently fall back to the default project", async () => {
  history.replaceState({}, "", "/?project=invalid");
  vi.resetModules();
  const fetch = vi
    .fn<(input: string) => Promise<Response>>()
    .mockResolvedValue(new Response("[]", { status: 200 }));
  vi.stubGlobal("fetch", fetch);
  const api = await import("../../../api");
  await api.fetchRunsPage();
  expect(fetch.mock.calls[0][0]).toBe("/api/projects/invalid/runs?limit=25");
});
