import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { Button, IconButton, Select } from "../../design-system/primitives/Controls";
import { Dialog, Dropdown, Popover } from "../../design-system/primitives/Overlays";
import {
  CompactStatus,
  EmptyState,
  LoadingIndicator,
  PanelHeader,
  ProgressRow,
  ViewErrorBoundary,
} from "../../design-system/components/Application";
import {
  AgentIdentity,
  AgentStatus,
  AttentionNotification,
  CheckpointState,
  CodeChangeSummary,
  CurrentMissionStep,
  MissionStatus,
  OrchestratorMessage,
  ReviewSummary,
  TestResult,
  UserMessage,
} from "../../design-system/patterns/Agentic";
import { TerminalHeader } from "../../design-system/patterns/Code";
import { tests } from "../../prototypes/fixtures/data";
import { Explorer } from "./Explorer";
import { useWorkspace, useWorkspaceState } from "../shell/context";
import type { ViewName } from "../layouts/presets";
import { Icon } from "../../design-system/icons/Icon";

const EditorPanel = lazy(() =>
  import("./CodePanel").then((module) => ({ default: module.EditorPanel }))
);
const DiffPanel = lazy(() =>
  import("./CodePanel").then((module) => ({ default: module.DiffPanel }))
);

function Conversation() {
  const model = useWorkspace();
  const state = useWorkspaceState();
  const [draft, setDraft] = useState(model.draft);
  const [attachments, setAttachments] = useState<string[]>([]);
  const [page, setPage] = useState(0);
  const input = useRef<HTMLTextAreaElement>(null);
  const pageSize = 24;
  const end = Math.max(pageSize, state.messages.length - page * pageSize);
  const start = Math.max(0, end - pageSize);
  useEffect(() => {
    setDraft(model.draft);
  }, [model, state.notice]);
  const submit = () => {
    model.addMessage(draft + (attachments.length ? `\n\nContext: ${attachments.join(", ")}` : ""));
    setDraft("");
    setAttachments([]);
    setPage(0);
  };
  return (
    <section className="v2-conversation" aria-label="Conversation workspace">
      <PanelHeader
        title="Checkpoint recovery"
        detail={`${model.project === "atlas" ? "Atlas" : "Fieldnotes"} / Session fixture`}
      >
        <CompactStatus
          status={
            state.mission.status === "attention"
              ? "attention"
              : state.mission.status === "running"
                ? "running"
                : "idle"
          }
          label={
            state.mission.status === "attention"
              ? "Needs your decision"
              : state.mission.status === "running"
                ? "Mission in progress"
                : "Ready"
          }
        />
      </PanelHeader>
      <div
        className="v2-conversation-scroll"
        role="log"
        aria-label="Fixture conversation"
        aria-live="polite"
      >
        {!state.messages.length ? (
          <EmptyState
            title="What would you like to work on?"
            description="Describe a change, ask about the code, or add a file for context. This preview records requests locally; no agent is connected."
            action={
              <Button tone="quiet" icon="code" onClick={() => model.openFile(state.selectedFile)}>
                Explore the fixture code
              </Button>
            }
          />
        ) : (
          <div className="v2-transcript">
            {start > 0 && (
              <Button tone="quiet" onClick={() => setPage(page + 1)}>
                Earlier {Math.min(pageSize, start)} messages
              </Button>
            )}
            {state.messages
              .slice(start, end)
              .map((message) =>
                message.role === "user" ? (
                  <UserMessage key={message.id} message={message} />
                ) : (
                  <OrchestratorMessage
                    key={message.id}
                    message={message}
                    onOpenFile={(path) => model.openFile(path)}
                    onReview={() => model.setPreset("Review")}
                  />
                )
              )}
            {page > 0 && (
              <Button tone="quiet" onClick={() => setPage(Math.max(0, page - 1))}>
                More recent messages
              </Button>
            )}
          </div>
        )}
      </div>
      <form
        className="v2-composer"
        onSubmit={(event) => {
          event.preventDefault();
          submit();
        }}
      >
        <label className="ds-sr-only" htmlFor="workspace-request">
          Message to orchestrator
        </label>
        <textarea
          ref={input}
          id="workspace-request"
          aria-describedby="composer-note"
          value={draft}
          rows={3}
          placeholder="Ask a question or describe a change…"
          onChange={(event) => {
            setDraft(event.target.value);
            model.draft = event.target.value;
            model.schedule();
          }}
          onKeyDown={(event) => {
            if ((event.ctrlKey || event.metaKey) && event.key === "Enter" && draft.trim()) {
              event.preventDefault();
              submit();
            }
          }}
        />
        {attachments.length > 0 && (
          <div className="v2-context-chips">
            {attachments.map((path) => (
              <Button
                key={path}
                tone="quiet"
                icon="file"
                onClick={() =>
                  setAttachments((previous) => previous.filter((item) => item !== path))
                }
              >
                {path.split("/").pop()}
                <Icon name="close" size={11} />
              </Button>
            ))}
          </div>
        )}
        <footer>
          <Dropdown
            label="Add context"
            icon="plus"
            entries={model.files.slice(0, 4).map((file) => ({
              id: file.path,
              label: file.path,
              icon: "file",
              action: () => {
                setAttachments((previous) => [...new Set([...previous, file.path])]);
                input.current?.focus();
              },
            }))}
          />
          <span id="composer-note">Local fixture · Ctrl Enter</span>
          <IconButton
            label="Send request"
            icon="arrow"
            tone="primary"
            disabled={!draft.trim()}
            onClick={submit}
          />
        </footer>
      </form>
    </section>
  );
}

function Mission() {
  const model = useWorkspace();
  const state = useWorkspaceState();
  const [details, setDetails] = useState(false);
  const mission = state.mission;
  return (
    <section className="v2-side-view" aria-label="Mission progress">
      <PanelHeader title="Mission" detail="Execution fixture" />
      <div className="v2-side-scroll">
        <MissionStatus mission={mission} />
        <CurrentMissionStep mission={mission} />
        {mission.status === "attention" && (
          <AttentionNotification
            title="An agent needs to be replaced"
            detail="The checkpoint is preserved. Select an available agent to continue verification."
          >
            <Button tone="quiet" onClick={() => model.showView("agents")}>
              Choose an agent
              <Icon name="chevron" size={12} />
            </Button>
          </AttentionNotification>
        )}
        <Button tone="quiet" onClick={() => setDetails(!details)} aria-expanded={details}>
          {details ? "Hide task details" : "Task details"}
          <Icon name={details ? "down" : "chevron"} size={12} />
        </Button>
        {details && (
          <ol className="v2-task-list">
            {mission.steps.map((step) => (
              <ProgressRow
                key={step.id}
                title={step.title}
                state={step.status}
                detail={step.detail}
              />
            ))}
          </ol>
        )}
        <CheckpointState checkpoint={mission.checkpoint} />
        {mission.status === "running" || mission.status === "suspended" ? (
          <Button
            icon={mission.status === "running" ? "pause" : "play"}
            onClick={() =>
              model.update({
                mission: {
                  ...mission,
                  status: mission.status === "running" ? "suspended" : "running",
                },
                notice: "Mission state changed in this fixture only",
              })
            }
          >
            {mission.status === "running" ? "Pause fixture mission" : "Resume fixture mission"}
          </Button>
        ) : null}
      </div>
    </section>
  );
}

function Agents() {
  const model = useWorkspace();
  const state = useWorkspaceState();
  const [transfer, setTransfer] = useState(false);
  const [replacement, setReplacement] = useState("claude");
  return (
    <section className="v2-side-view" aria-label="Agent supervision">
      <PanelHeader title="Configured profiles" detail="Agent fixtures">
        <IconButton
          label="Close agents panel"
          tone="quiet"
          icon="close"
          onClick={() => model.toggleView("agents")}
        />
      </PanelHeader>
      <div className="v2-side-scroll">
        {state.mission.status === "attention" && (
          <AttentionNotification
            title="Verification is paused"
            detail="Codex is unavailable. Changes and the previous attempt remain at checkpoint cp-014."
          />
        )}
        <div className="v2-agent-list">
          {state.agents.map((agent) => (
            <article key={agent.id} className="v2-agent-row">
              <div>
                <AgentIdentity agent={agent} />
                <AgentStatus agent={agent} />
              </div>
              <p>{agent.task}</p>
              <div className="ds-inline-actions">
                <Popover label={`${agent.name} permissions`} trigger="Permissions">
                  <p>{agent.permissions.join(" · ")}</p>
                  <p>These are fixture profile permissions.</p>
                </Popover>
                {agent.id === "codex" && (
                  <Button tone="quiet" onClick={() => setTransfer(true)}>
                    Reassign task
                  </Button>
                )}
              </div>
            </article>
          ))}
        </div>
        <CheckpointState checkpoint={state.mission.checkpoint} />
        <p className="v2-fine-print">
          Profiles, assignments and availability are deterministic fixtures. No agent process is
          connected.
        </p>
      </div>
      <Dialog
        open={transfer}
        onClose={() => setTransfer(false)}
        title="Reassign verification"
        description="Resume from verified checkpoint cp-014. Existing changes and the previous attempt remain available."
        footer={
          <>
            <Button onClick={() => setTransfer(false)}>Cancel</Button>
            <Button
              tone="primary"
              onClick={() => {
                model.update({
                  agents: state.agents.map((agent) => ({
                    ...agent,
                    status:
                      agent.id === replacement
                        ? "working"
                        : agent.id === "codex"
                          ? "unavailable"
                          : agent.status,
                  })),
                  mission: {
                    ...state.mission,
                    status: "running",
                    steps: state.mission.steps.map((step) =>
                      step.id === "test"
                        ? {
                            ...step,
                            status: "running",
                            detail: "Fixture verification assigned to Claude Code",
                          }
                        : step
                    ),
                  },
                });
                model.notify(
                  "Fixture assignment updated",
                  "Claude Code resumes verification from cp-014. No process was launched."
                );
                setTransfer(false);
              }}
            >
              Assign in fixture
            </Button>
          </>
        }
      >
        <Select
          label="Authorized replacement"
          value={replacement}
          onChange={(event) => setReplacement(event.target.value)}
        >
          <option value="claude">Claude Code · local tests allowed</option>
          <option value="gemini" disabled>
            Gemini CLI · read-only profile
          </option>
        </Select>
        <CheckpointState checkpoint={state.mission.checkpoint} />
      </Dialog>
    </section>
  );
}

function Review() {
  const model = useWorkspace();
  const state = useWorkspaceState();
  const files = model.files.filter(
    (file) => file.review === "pending" && (file.added || file.removed)
  );
  return (
    <section className="v2-side-view" aria-label="Review changes">
      <PanelHeader title="Review" detail="Local fixture changes" />
      <div className="v2-side-scroll">
        <ReviewSummary
          status={state.review}
          summary="Checkpoint ownership is checked before recovery. Missing checkpoints return a useful error, and existing permission checks remain in place."
        >
          <CodeChangeSummary files={files} />
          <div className="v2-review-files">
            {files.slice(0, 5).map((file) => (
              <button
                key={file.path}
                type="button"
                onClick={() => {
                  model.showView("diff");
                  model.update({ selectedFile: file.path });
                }}
              >
                <Icon name="file" size={13} />
                <span title={file.path}>{file.path.split("/").pop()}</span>
                <span>{file.origin}</span>
              </button>
            ))}
          </div>
          {files.length > 5 && (
            <Button
              tone="quiet"
              onClick={() => {
                model.update({ activity: "git" });
                model.showView("explorer");
              }}
            >
              Browse changed files · showing 5 of {files.length} pending paths
            </Button>
          )}
          <h4>Recorded test groups</h4>
          {tests.map((test) => (
            <TestResult key={test.name} test={test} />
          ))}
          <div className="v2-review-actions">
            <Button
              tone="primary"
              icon="check"
              disabled={state.review === "accepted"}
              onClick={() => {
                model.update({ review: "accepted" });
                model.notify(
                  "Fixture review accepted",
                  "Only preview state changed. No Git operation was performed."
                );
              }}
            >
              {state.review === "accepted" ? "Accepted" : "Accept fixture changes"}
            </Button>
            <Button disabled>Publish</Button>
          </div>
          <p className="v2-fine-print">
            Publication requires a connected project and explicit authorization. All test results
            above are fixture records.
          </p>
        </ReviewSummary>
      </div>
    </section>
  );
}

function Output() {
  const model = useWorkspace();
  const state = useWorkspaceState();
  const [wrap, setWrap] = useState(false);
  const normal =
    "atlas / checkpoint-recovery\n$ cargo test checkpoint_owner\n\nrunning 9 tests\n  checkpoint_owner_matches ... ok\n  checkpoint_owner_mismatch ... ok\n  missing_checkpoint_is_recoverable ... ok\n\nRecorded output fixture · no shell is connected.";
  const lines =
    state.scenario === "long"
      ? Array.from(
          { length: 1800 },
          (_, index) =>
            `fixture trace ${String(index).padStart(5, "0")}  checkpoint=cp-014 project=atlas session=checkpoint-recovery operation=validate_owner result=recorded_fixture_only` +
            (index % 30 === 0
              ? "  detail=" + "context_without_external_side_effects/".repeat(20)
              : "")
        ).join("\n")
      : normal;
  return (
    <section className="v2-output" aria-label="Output panel">
      <TerminalHeader
        panelId="fixture-output"
        provenance="fixture"
        channel={state.outputChannel}
        onChange={(outputChannel) => model.update({ outputChannel, outputCleared: false })}
        onClear={() => model.update({ outputCleared: true, notice: "Fixture output cleared" })}
      />
      <div className="v2-output-toolbar">
        <span>Recorded output · fixture · no process connection</span>
        <Button tone="quiet" aria-pressed={wrap} onClick={() => setWrap(!wrap)}>
          Wrap lines
        </Button>
      </div>
      <div
        id="fixture-output"
        role="tabpanel"
        aria-label={state.outputChannel}
        className="v2-output-content"
        tabIndex={0}
      >
        {state.outputCleared ? (
          <EmptyState
            title="Output cleared"
            description="Select another channel to view its fixture record."
          />
        ) : state.outputChannel === "Tests" ? (
          <div className="v2-output-tests">
            {tests.map((test) => (
              <TestResult key={test.name} test={test} />
            ))}
          </div>
        ) : state.outputChannel === "Diagnostics" ? (
          <div className="v2-output-tests">
            <EmptyState
              title="No connected language server"
              description="This is a presentation preview. Real diagnostics are provided by the Rust-owned LSP service in later integration."
            />
          </div>
        ) : (
          <pre className={wrap ? "v2-log--wrap" : ""}>
            {state.outputChannel === "Logs"
              ? `09:46:01 fixture checkpoint cp-014 verified\n09:46:02 fixture review state ready\n\n${state.scenario === "long" ? lines : "These records are local fixture data."}`
              : lines}
          </pre>
        )}
      </div>
    </section>
  );
}

export function PanelContent({
  name,
  params,
  panelId,
}: {
  name: ViewName;
  params: Record<string, unknown>;
  panelId: string;
}) {
  const model = useWorkspace();
  const state = useWorkspaceState();
  return (
    <ViewErrorBoundary>
      <Suspense
        fallback={
          <div className="v2-panel-loading">
            <LoadingIndicator label="Loading editor module" />
          </div>
        }
      >
        {name === "conversation" ? (
          <Conversation />
        ) : name === "explorer" ? (
          <Explorer />
        ) : name === "mission" ? (
          <Mission />
        ) : name === "agents" ? (
          <Agents />
        ) : name === "review" ? (
          <Review />
        ) : name === "output" ? (
          <Output />
        ) : name === "editor" ? (
          <EditorPanel model={model} path={String(params.path)} panelId={panelId} />
        ) : (
          <DiffPanel model={model} path={state.selectedFile} />
        )}
      </Suspense>
    </ViewErrorBoundary>
  );
}
