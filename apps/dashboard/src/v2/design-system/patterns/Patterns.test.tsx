import "@testing-library/jest-dom/vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useState } from "react";
import {
  CompactStatus,
  InlineAlert,
  NotificationItem,
  ViewErrorBoundary,
} from "../components/Application";
import {
  AgentIdentity,
  AgentStatus,
  CheckpointState,
  MissionStatus,
  OrchestratorMessage,
  TestResult,
} from "./Agentic";
import { DebugToolbar, DiagnosticsIndicator, GitModificationIndicator } from "./Code";
import { agents, messagesFor, missionFor, tests } from "../../prototypes/fixtures/data";

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});
describe("V2 domain presentations", () => {
  it("runtime records retain their provenance without fixture labels", () => {
    render(
      <>
        <OrchestratorMessage
          message={{ ...messagesFor("conversation")[3], provenance: "runtime" }}
          onOpenFile={() => undefined}
          onReview={() => undefined}
        />
        <MissionStatus mission={{ ...missionFor("running"), provenance: "runtime" }} />
      </>
    );
    expect(screen.getByText(/Recorded evidence/)).toBeInTheDocument();
    expect(document.body.textContent).not.toContain("Fixture record");
    expect(screen.getByText("2 of 5 tasks completed")).toBeInTheDocument();
  });
  it.each([
    "idle",
    "running",
    "completed",
    "failed",
    "suspended",
    "attention",
    "cancelled",
  ] as const)("%s status has a text label as well as an icon", (status) => {
    const view = render(<CompactStatus status={status} />);
    expect(view.container.textContent?.trim().length).toBeGreaterThan(2);
    expect(view.container.querySelector('svg[aria-hidden="true"]')).not.toBeNull();
  });
  it("mission progress is derived from completed task records", () => {
    const mission = missionFor("running");
    render(<MissionStatus mission={mission} />);
    const progress = screen.getByRole("progressbar") as HTMLProgressElement;
    expect(progress.value).toBe(2);
    expect(progress.max).toBe(5);
    expect(screen.getByText("2 of 5 tasks completed · fixture")).toBeInTheDocument();
  });
  it("completed mission progress matches all tasks", () => {
    render(<MissionStatus mission={missionFor("review")} />);
    const progress = screen.getByRole("progressbar") as HTMLProgressElement;
    expect(progress.value).toBe(progress.max);
  });
  it("file references and result summaries emit their supplied intentions", () => {
    const open = vi.fn();
    const review = vi.fn();
    render(
      <OrchestratorMessage
        message={messagesFor("conversation")[3]}
        onOpenFile={open}
        onReview={review}
      />
    );
    fireEvent.click(
      screen
        .getAllByRole("button")
        .find((button) => button.textContent?.includes("checkpoints.rs"))!
    );
    expect(open).toHaveBeenCalledWith("src/runtime/checkpoints.rs");
    fireEvent.click(screen.getByRole("button", { name: "Open review" }));
    expect(review).toHaveBeenCalledOnce();
    expect(screen.getByText(/Fixture record/)).toBeInTheDocument();
  });
  it("long messages expand without truncating the domain content", () => {
    const message = { ...messagesFor("conversation")[1], body: "Relevant context ".repeat(200) };
    render(
      <OrchestratorMessage
        message={message}
        onOpenFile={() => undefined}
        onReview={() => undefined}
      />
    );
    const short = document.querySelector(".ds-message-body")!.textContent!.length;
    fireEvent.click(screen.getByRole("button", { name: "Read full message" }));
    expect(document.querySelector(".ds-message-body")!.textContent!.length).toBeGreaterThan(short);
    expect(document.querySelector(".ds-message-body")).toHaveTextContent(message.body.trim());
  });
  it("agent identity and availability are separate from provider model claims", () => {
    render(
      <>
        <AgentIdentity agent={agents[0]} />
        <AgentStatus agent={agents[0]} />
      </>
    );
    expect(screen.getByText("Codex")).toBeInTheDocument();
    expect(screen.getByText("Working")).toBeInTheDocument();
    expect(screen.getByText(/Project CLI profile/)).toBeInTheDocument();
  });
  it("checkpoint state exposes verification and durable identity", () => {
    render(<CheckpointState checkpoint={missionFor("running").checkpoint} />);
    expect(screen.getByText("Checkpoint verified")).toBeInTheDocument();
    expect(screen.getByText(/cp-014/)).toBeInTheDocument();
  });
  it("test records distinguish not-run from actual supplied pass state", () => {
    render(
      <>
        <TestResult test={tests[0]} />
        <TestResult test={{ ...tests[1], status: "not-run" }} />
      </>
    );
    expect(screen.getByText("Passed")).toBeInTheDocument();
    expect(screen.getByText("Not run")).toBeInTheDocument();
  });
  it("notifications dismiss by identity", () => {
    const dismiss = vi.fn();
    render(
      <NotificationItem
        item={{
          id: "n-1",
          title: "Review ready",
          detail: "Fixture result",
          severity: "success",
          createdAt: "Fixture",
        }}
        onDismiss={dismiss}
      />
    );
    fireEvent.click(screen.getByRole("button", { name: "Dismiss Review ready" }));
    expect(dismiss).toHaveBeenCalledWith("n-1");
  });
  it("critical alerts expose an assertive role and a named dismissal", () => {
    const dismiss = vi.fn();
    render(
      <InlineAlert tone="danger" title="Save failed" onDismiss={dismiss}>
        Buffer retained
      </InlineAlert>
    );
    expect(screen.getByRole("alert")).toHaveTextContent("Buffer retained");
    fireEvent.click(screen.getByRole("button", { name: "Dismiss Save failed" }));
    expect(dismiss).toHaveBeenCalledOnce();
  });
  it("error boundaries preserve other UI and retry a corrected child", () => {
    vi.spyOn(console, "error").mockImplementation(() => undefined);
    function Broken({ failed }: { failed: boolean }) {
      if (failed) throw new Error("Fixture render failed");
      return <span>Recovered content</span>;
    }
    function Example() {
      const [failed, set] = useState(true);
      return (
        <>
          <button>Other panel</button>
          <ViewErrorBoundary onRetry={() => set(false)}>
            <Broken failed={failed} />
          </ViewErrorBoundary>
        </>
      );
    }
    render(<Example />);
    expect(screen.getByRole("alert")).toHaveTextContent("Fixture render failed");
    expect(screen.getByRole("button", { name: "Other panel" })).toBeEnabled();
    fireEvent.click(screen.getByRole("button", { name: "Reload view" }));
    expect(screen.getByText("Recovered content")).toBeInTheDocument();
  });
  it("debug presentation enables only actions valid for the supplied state", () => {
    const action = vi.fn();
    const { rerender } = render(
      <DebugToolbar provenance="fixture" state="running" onAction={action} />
    );
    expect(screen.getByRole("button", { name: "Step debug fixture" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Pause debug fixture" }));
    expect(action).toHaveBeenCalledWith("pause");
    rerender(<DebugToolbar provenance="fixture" state="stopped" onAction={action} />);
    expect(
      screen.getAllByRole("button").every((button) => (button as HTMLButtonElement).disabled)
    ).toBe(true);
  });
  it("diagnostics and Git indicators expose meaningful accessible labels", () => {
    render(
      <>
        <DiagnosticsIndicator errors={2} warnings={1} />
        <GitModificationIndicator status="modified" origin="agent" />
      </>
    );
    expect(screen.getByLabelText("2 errors, 1 warnings")).toBeInTheDocument();
    expect(screen.getByLabelText("modified, agent changes")).toBeInTheDocument();
  });
});
