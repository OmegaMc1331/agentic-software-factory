import { afterEach, describe, expect, it, vi } from "vitest";
import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { ProfileEditor } from "./AgentManagement";
import { HandoffControls, SessionConfiguration } from "./HandoffControls";
import { newProfile, profileConfiguration, type Profile, type Providers } from "./types";
import { requestHandoff, saveProfile } from "./client";
import type { DockSession } from "../workspace/production/controller";

const mocks = vi.hoisted(() => ({
  request: vi.fn(),
  resources: new Map<string, unknown>(),
  refresh: vi.fn(),
}));
vi.mock("../../api", () => ({ request: mocks.request }));
vi.mock("../workspace/production/data", () => ({
  useData: (_project: string, path: string) => ({
    value: mocks.resources.get(path),
    loading: false,
  }),
  refreshData: mocks.refresh,
}));
const profile: Profile = {
  ...newProfile("trusted"),
  name: "Original",
  id: "ap_one",
  projectId: "p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  revision: 2,
  createdAt: "then",
  updatedAt: "now",
  archived: false,
};
const providers: Providers = {
  providers: [
    {
      id: "trusted",
      kind: "codex",
      available: true,
      modelSelection: true,
      modelDiscovery: false,
      reasoningSelection: true,
      maxConcurrency: 2,
      roles: [],
    },
  ],
  roles: [
    { id: "worker", name: "Worker" },
    { id: "reviewer", name: "Reviewer" },
  ],
};
afterEach(() => {
  cleanup();
  mocks.request.mockReset();
  mocks.resources.clear();
  mocks.refresh.mockReset();
});

describe("custom agent configuration", () => {
  it("retains a stale edit draft and sends the expected revision", async () => {
    mocks.request.mockRejectedValue(new Error("stale_profile_revision"));
    render(
      <ProfileEditor
        projectId="p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        profile={profile}
        providers={providers}
        done={vi.fn()}
      />
    );
    fireEvent.change(screen.getByLabelText("Agent name"), { target: { value: "My changes" } });
    fireEvent.click(screen.getByText("Save agent profile"));
    await screen.findByRole("alert");
    expect(screen.getByLabelText("Agent name")).toHaveValue("My changes");
    const [url, init] = mocks.request.mock.calls[0];
    expect(url).toContain("/projects/p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/agent-profiles/ap_one");
    expect(JSON.parse(init.body)).toMatchObject({
      expectedRevision: 2,
      configuration: { name: "My changes" },
    });
  });
  it("passes selected model and instructions as data, without executable or credential fields", async () => {
    mocks.request.mockResolvedValue({});
    const done = vi.fn();
    render(
      <ProfileEditor
        projectId="p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        providers={providers}
        done={done}
      />
    );
    fireEvent.change(screen.getByLabelText("Agent name"), { target: { value: "Specialist" } });
    fireEvent.change(screen.getByLabelText("Model identifier"), {
      target: { value: "configured-model" },
    });
    fireEvent.change(screen.getByLabelText("Agent instructions"), {
      target: { value: "Preserve migration compatibility" },
    });
    fireEvent.click(screen.getByText("Save agent profile"));
    await waitFor(() => expect(done).toHaveBeenCalledOnce());
    const payload = JSON.parse(mocks.request.mock.calls[0][1].body);
    expect(payload).toMatchObject({
      providerAdapterId: "trusted",
      modelId: "configured-model",
      instructions: "Preserve migration compatibility",
    });
    expect(payload).not.toHaveProperty("command");
    expect(payload).not.toHaveProperty("env");
  });
  it("serializes only editable configuration fields", () => {
    expect(profileConfiguration(profile)).not.toHaveProperty("revision");
    expect(profileConfiguration(profile)).not.toHaveProperty("projectId");
    expect(newProfile("trusted").permissions.filesystem?.write).toEqual([]);
  });
  it("keeps model availability distinct from executable availability", () => {
    render(
      <ProfileEditor
        projectId="p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        providers={providers}
        done={vi.fn()}
      />
    );
    expect(
      screen.getByText(/Syntax validation does not prove account access or quota/)
    ).toBeInTheDocument();
    expect(screen.getByText(/Reviewer roles remain read-only/)).toBeInTheDocument();
  });
});

describe("safe handoff", () => {
  it("does not describe a running task as undispatched while attempt data is pending", () => {
    mocks.resources.set("runs/1", {
      tasks: [{ id: 7, title: "Active source task", state: "running", role: "worker" }],
      attempts: [],
    });
    render(
      <HandoffControls
        projectId="p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        runId={1}
        dock={{} as DockSession}
      />
    );
    fireEvent.change(screen.getByLabelText("Task to reassign"), { target: { value: "7" } });
    expect(screen.getByText(/Awaiting recorded agent data/)).toBeInTheDocument();
    expect(screen.queryByText(/Not dispatched/)).not.toBeInTheDocument();
  });
  it("preserves the operation key across an uncertain retry and shows the real waiting state", async () => {
    mocks.resources.set("agent-profiles", [profile]);
    mocks.resources.set("runs/1", {
      tasks: [{ id: 7, title: "Active source task", state: "running", role: "worker" }],
      attempts: [{ taskId: 7, agent: "Original Worker" }],
    });
    mocks.resources.set("runs/1/agent-handoffs", []);
    mocks.request
      .mockRejectedValueOnce(new Error("Network timeout"))
      .mockResolvedValueOnce({ state: "waiting" });
    render(
      <HandoffControls
        projectId="p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        runId={1}
        dock={{} as DockSession}
      />
    );
    fireEvent.change(screen.getByLabelText("Task to reassign"), { target: { value: "7" } });
    fireEvent.change(screen.getByLabelText("Replacement agent"), { target: { value: profile.id } });
    fireEvent.change(screen.getByLabelText("Handoff reason"), {
      target: { value: "Use specialist" },
    });
    fireEvent.click(screen.getByText("Request safe handoff"));
    await screen.findByRole("alert");
    fireEvent.click(screen.getByText("Request safe handoff"));
    await waitFor(() => expect(mocks.request).toHaveBeenCalledTimes(2));
    expect(JSON.parse(mocks.request.mock.calls[0][1].body).idempotencyKey).toEqual(
      JSON.parse(mocks.request.mock.calls[1][1].body).idempotencyKey
    );
    expect(mocks.request.mock.calls[0][0]).toContain(
      "/projects/p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/runs/1/tasks/7/handoff"
    );
    expect(screen.getByText(/Immediate process interruption is unavailable/)).toBeInTheDocument();
  });
  it("does not present a pending request as a completed replacement", () => {
    mocks.resources.set("runs/1/agent-handoffs", [
      {
        id: "ho_one",
        taskId: 7,
        state: "waiting",
        target: profile,
        reason: "Review context",
        boundary: "worker_exit_and_evidence_capture",
      },
    ]);
    render(
      <HandoffControls
        projectId="p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        runId={1}
        dock={{} as DockSession}
      />
    );
    expect(screen.getByText("Task #7 · waiting")).toBeInTheDocument();
    expect(screen.queryByText(/Replacement session/)).not.toBeInTheDocument();
  });
  it("renders immutable historic model information rather than current profile configuration", () => {
    mocks.resources.set("agent-sessions/3/execution-snapshot", {
      profile: { ...profile, name: "Historic", revision: 1 },
      providerKind: "codex",
      requestedModel: "original-model",
    });
    render(<SessionConfiguration projectId="p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" sessionId={3} />);
    expect(
      screen.getByText(/Historic · revision 1 · codex · requested original-model/)
    ).toBeInTheDocument();
    expect(screen.getByText(/Observed model: Not reported/)).toBeInTheDocument();
  });
  it("client operations preserve project and profile identities", async () => {
    mocks.request.mockResolvedValue({});
    await saveProfile("p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", profileConfiguration(profile), profile);
    await requestHandoff(
      "p_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      1,
      1,
      profile,
      "explicit",
      "key-one"
    );
    expect(mocks.request.mock.calls[0][0]).toContain(
      "/projects/p_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/"
    );
    expect(mocks.request.mock.calls[1][0]).toContain(
      "/projects/p_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb/"
    );
    expect(JSON.parse(mocks.request.mock.calls[1][1].body)).toMatchObject({
      toProfileId: "ap_one",
      expectedProfileRevision: 2,
      idempotencyKey: "key-one",
    });
  });
});
