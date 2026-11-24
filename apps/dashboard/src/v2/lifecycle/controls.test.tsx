import { afterEach, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { request } from "../../api";
import { useData } from "../workspace/production/data";
import { MissionLifecycleControls } from "./LifecycleControls";
vi.mock("../../api", () => ({ request: vi.fn() }));
vi.mock("../workspace/production/data", () => ({
  useData: vi.fn(),
  refreshData: vi.fn().mockResolvedValue(undefined),
}));
afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});
const project = `p_${"a".repeat(32)}`;
it("refuses invalid project identity before subscribing or presenting lifecycle actions", () => {
  const view = render(<MissionLifecycleControls projectId="unscoped" runId={1} />);
  expect(view.container.textContent).toBe("");
  expect(useData).not.toHaveBeenCalled();
  expect(request).not.toHaveBeenCalled();
});
it("shows backend suspension and sends its exact generation without browser process targets", async () => {
  vi.mocked(useData).mockReturnValue({
    loading: false,
    updated: 1,
    value: {
      lifecycle: {
        schemaVersion: 1,
        projectId: project,
        workspaceSessionId: `s_${"b".repeat(32)}`,
        runId: 1,
        generation: 2,
        state: "suspended",
        reason: "last_browser_client_expired",
        checkpointId: 7,
        problem: null,
        updatedAt: "2026-11-02T00:00:00Z",
      },
      legacy: false,
      processes: [],
    },
  });
  vi.mocked(request).mockResolvedValue({ state: "reconciling" });
  render(<MissionLifecycleControls projectId={project} runId={1} />);
  expect(screen.getByRole("status").textContent).toBe("Execution: suspended");
  fireEvent.click(screen.getByRole("button", { name: "Validate and resume" }));
  await waitFor(() => expect(request).toHaveBeenCalledTimes(1));
  const [url, options] = vi.mocked(request).mock.calls[0];
  expect(url).toBe(`/projects/${project}/runs/1/resume`);
  const body = JSON.parse(String(options?.body));
  expect(body.expectedGeneration).toBe(2);
  expect(Object.keys(body).sort()).toEqual(["expectedGeneration", "idempotencyKey"]);
});
