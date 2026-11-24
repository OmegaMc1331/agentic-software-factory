import { lazy, memo, Suspense, useState, useSyncExternalStore } from "react";
import { request } from "../../../api";
import type { AgentSession, ConfigData, RunDetail, RunPage } from "../../../types";
import { AttemptEvidence } from "./AttemptEvidence";
import { HandoffControls, SessionConfiguration } from "../../agents/HandoffControls";
const AgentManagement = lazy(() =>
  import("../../agents/AgentManagement").then((module) => ({ default: module.AgentManagement }))
);
const MemoryPanel = lazy(() =>
  import("../../memory/MemoryPanel").then((module) => ({ default: module.MemoryPanel }))
);
const MemoryUsage = lazy(() =>
  import("../../memory/MemoryUsage").then((module) => ({ default: module.MemoryUsage }))
);
const SourceControlPanel = lazy(() =>
  import("../../source-control/SourceControlPanel").then((module) => ({
    default: module.SourceControlPanel,
  }))
);
const ReviewPanel = lazy(() =>
  import("../../source-control/ReviewPanel").then((module) => ({ default: module.ReviewPanel }))
);
import { projectMutation, projectPath, type Project } from "../../projects/projectApi";
import { Button, SearchField } from "../../design-system/primitives/Controls";
import {
  EmptyState,
  InlineAlert,
  LoadingIndicator,
  ViewErrorBoundary,
} from "../../design-system/components/Application";
import { useData, refreshData, useSessionStream, sessionStreams } from "./data";
import type { DockSession, WorkspaceModel } from "./controller";
import type { WorkspaceView } from "./model";
const ConversationPanel = lazy(() =>
  import("../../conversation/ConversationPanel").then((module) => ({
    default: module.ConversationPanel,
  }))
);
const ConversationProgress = lazy(() =>
  import("../../conversation/ConversationPanel").then((module) => ({
    default: module.ConversationProgress,
  }))
);
const TerminalView = lazy(() => import("./TerminalView"));
const ScratchNote = lazy(() => import("./ScratchNote"));
const Explorer = lazy(() =>
  import("../../files/ExplorerPanel").then((module) => ({ default: module.ExplorerPanel }))
);
const Editor = lazy(() =>
  import("../../files/EditorPanel").then((module) => ({ default: module.EditorPanel }))
);
const FileSearch = lazy(() =>
  import("../../files/SearchPanel").then((module) => ({ default: module.FileSearchPanel }))
);
const Diff = lazy(() =>
  import("../../files/DiffPanel").then((module) => ({ default: module.DiffPanel }))
);
const DebugPanel = lazy(() =>
  import("../../ide/DebugPanel").then((module) => ({ default: module.DebugPanel }))
);
const DevelopmentTerminalPanel = lazy(() =>
  import("../../ide/TerminalPanel").then((module) => ({ default: module.DevelopmentTerminalPanel }))
);
const ExtensionsPanel = lazy(() =>
  import("../../ide/ExtensionsPanel").then((module) => ({ default: module.ExtensionsPanel }))
);
const ExtensionView = lazy(() =>
  import("../../ide/ExtensionsPanel").then((module) => ({ default: module.ExtensionView }))
);
const DiagnosticsPanel = lazy(() =>
  import("../../ide/DiagnosticsPanel").then((module) => ({ default: module.DiagnosticsPanel }))
);
const SettingsPanel = lazy(() =>
  import("../../product/SettingsPanel").then((module) => ({ default: module.SettingsPanel }))
);
const NotificationCenter = lazy(() =>
  import("../../product/Notifications").then((module) => ({ default: module.NotificationCenter }))
);
function ErrorState({ error, retry }: { error: string; retry: () => void }) {
  return (
    <InlineAlert tone="danger" title="View unavailable">
      <p>{error}</p>
      <Button onClick={retry}>Retry view</Button>
    </InlineAlert>
  );
}
export const LiveView = memo(function LiveView({
  model,
  dock,
  id,
}: {
  model: WorkspaceModel;
  dock: DockSession;
  id: string;
}) {
  const view = useSyncExternalStore(model.subscribe, () =>
    model.state.document.sessions
      .find((s) => s.id === dock.sessionId)
      ?.views.find((v) => v.id === id)
  );
  const registry = useData<Project[]>("", "/projects");
  if (!view || view.closed) return null;
  const project = registry.value?.find((p) => p.project_id === view.projectId);
  return (
    <section
      className="ws-view"
      aria-label={`${view.title} - ${project?.display_name ?? view.projectId}`}
      data-view-id={view.id}
      data-project-id={view.projectId}
    >
      <header className="ws-view-context">
        <span title={view.projectId}>{project?.display_name ?? "Missing project"}</span>
        <span>{view.resourceId ? `#${view.resourceId}` : view.title}</span>
      </header>
      <ViewErrorBoundary>
        <Suspense fallback={<LoadingIndicator label="Loading view" />}>
          {view.kind === "settings" ? (
            <ViewContent view={view} dock={dock} model={model} />
          ) : registry.error ? (
            <ErrorState error={registry.error} retry={() => void refreshData("", "/projects")} />
          ) : !registry.value ? (
            <LoadingIndicator label="Loading project" />
          ) : !project ? (
            <EmptyState
              title="Project is no longer registered"
              description="The saved view and its draft are retained. Select a healthy project or repair the registry entry."
              action={<a href="/projects.html">Manage projects</a>}
            />
          ) : project.availability !== "available" ? (
            <ErrorState
              error={
                project.problem ??
                "Project is unavailable. Restore or relink its directory; other projects remain usable."
              }
              retry={() => void refreshData("", "/projects")}
            />
          ) : project.lifecycle !== "open" ? (
            <EmptyState
              title="Project runtime is closed"
              description="Restoring a layout does not start workflows or reopen runtimes. Open this registered project to reconnect its views."
              action={
                <Button
                  onClick={() =>
                    void projectMutation(projectPath(view.projectId, "open"), "POST")
                      .then(() => refreshData("", "/projects"))
                      .catch((error) => model.setIssue(String(error)))
                  }
                >
                  Open project runtime
                </Button>
              }
            />
          ) : (
            <ViewContent view={view} dock={dock} model={model} />
          )}
        </Suspense>
      </ViewErrorBoundary>
    </section>
  );
});
function ViewContent({
  view,
  dock,
  model,
}: {
  view: WorkspaceView;
  dock: DockSession;
  model: WorkspaceModel;
}) {
  switch (view.kind) {
    case "memory":
      return (
        <MemoryPanel
          key={`${view.id}:${view.state.memoryId ?? ""}`}
          projectId={view.projectId}
          initialId={view.state.memoryId}
          dock={dock}
        />
      );
    case "home":
      return <ConversationPanel view={view} dock={dock} model={model} />;
    case "workflows":
      return <Workflows view={view} dock={dock} model={model} />;
    case "workflow":
    case "progress":
      if (!view.resourceId) return <ConversationProgress view={view} model={model} dock={dock} />;
      return <Workflow view={view} dock={dock} model={model} />;
    case "sourceControl":
      return (
        <SourceControlPanel
          key={`${view.projectId}:${view.id}`}
          view={view}
          model={model}
          dock={dock}
        />
      );
    case "review":
      return (
        <ReviewPanel key={`${view.projectId}:${view.id}`} view={view} model={model} dock={dock} />
      );
    case "tests":
      return <Workflow view={view} dock={dock} model={model} />;
    case "agents":
      return (
        <AgentManagement key={view.projectId} projectId={view.projectId} dock={dock}>
          <Agents view={view} dock={dock} embedded />
        </AgentManagement>
      );
    case "console":
      return view.resourceId ? (
        <SessionOutput view={view} dock={dock} />
      ) : (
        <Agents view={view} dock={dock} sessions />
      );
    case "terminal":
      return view.resourceId ? (
        <SessionOutput view={view} dock={dock} terminal />
      ) : (
        <Boundary
          title="Select an existing interactive agent session"
          detail="Open Agents, choose a recorded session, then Open terminal. This view does not create a shell or an agent process."
        />
      );
    case "search":
      return <FileSearch view={view} model={model} dock={dock} />;
    case "settings":
      return <SettingsPanel view={view} model={model} dock={dock} />;
    case "notifications":
      return <NotificationCenter projectId={view.projectId} model={model} />;
    case "debug":
      return <DebugPanel key={view.projectId} view={view} model={model} dock={dock} />;
    case "developmentTerminal":
      return (
        <DevelopmentTerminalPanel
          key={`${view.projectId}:${view.agent ?? ""}`}
          view={view}
          model={model}
          dock={dock}
        />
      );
    case "extensions":
      return <ExtensionsPanel key={view.projectId} view={view} dock={dock} />;
    case "extensionView":
      return <ExtensionView view={view} />;
    case "diagnostics":
      return <DiagnosticsPanel projectId={view.projectId} model={model} dock={dock} />;
    case "scratch":
      return <ScratchNote view={view} model={model} />;
    case "explorer":
      return <Explorer view={view} model={model} dock={dock} />;
    case "code":
    case "editor":
      return <Editor view={view} model={model} dock={dock} />;
    case "diff":
      return <Diff view={view} model={model} dock={dock} />;
    default:
      return (
        <Boundary
          title={`${view.title} integration is unavailable`}
          detail="This view is unavailable. Open another view or restore a layout preset; saved resources and drafts are retained."
        />
      );
  }
}
function Boundary({
  title,
  detail,
  action,
}: {
  title: string;
  detail: string;
  action?: React.ReactNode;
}) {
  return (
    <div className="ws-scroll">
      <EmptyState title={title} description={detail} action={action} />
    </div>
  );
}
function Workflows({
  view,
  dock,
  model,
}: {
  view: WorkspaceView;
  dock: DockSession;
  model: WorkspaceModel;
}) {
  const path = `runs?limit=100${view.state.cursor ? `&before=${view.state.cursor}` : ""}`,
    data = useData<RunPage>(view.projectId, path),
    query = view.state.query ?? "",
    page = view.state.page ?? 0;
  const items = (data.value?.items ?? []).filter((run) =>
    run.objective.toLowerCase().includes(query.toLowerCase())
  );
  return (
    <div className="ws-scroll">
      <SearchField
        label="Filter workflow page"
        placeholder="Filter this workflow page"
        value={query}
        onChange={(value) => model.updateView(view.sessionId, view.id, { query: value, page: 0 })}
      />
      {data.error && (
        <ErrorState error={data.error} retry={() => void refreshData(view.projectId, path)} />
      )}
      <div className="ws-list">
        {items.slice(page * 20, page * 20 + 20).map((run) => (
          <button
            key={run.id}
            title={run.objective}
            onClick={() =>
              dock.open("workflow", {
                projectId: view.projectId,
                resourceId: run.id,
                title: `#${run.id} ${objectivePreview(run.objective)}`,
              })
            }
          >
            <span>#{run.id}</span>
            <span>{objectivePreview(run.objective)}</span>
            <small>
              Run status: {run.status} · {run.counts.total} recorded tasks
            </small>
          </button>
        ))}
      </div>
      {data.loading ? (
        <LoadingIndicator label="Loading workflow page" />
      ) : (
        !items.length && <p>No workflows on this page.</p>
      )}
      <div className="ws-actions">
        {view.state.cursor && (
          <Button
            onClick={() =>
              model.updateView(view.sessionId, view.id, { cursor: undefined, page: 0 })
            }
          >
            Newest workflows
          </Button>
        )}
        <Button
          disabled={page === 0}
          onClick={() => model.updateView(view.sessionId, view.id, { page: page - 1 })}
        >
          Previous
        </Button>
        <span>{items.length} on fetched page</span>
        <Button
          disabled={(page + 1) * 20 >= items.length}
          onClick={() => model.updateView(view.sessionId, view.id, { page: page + 1 })}
        >
          Next
        </Button>
        {data.value?.hasMore && (
          <Button
            onClick={() =>
              model.updateView(view.sessionId, view.id, {
                cursor: data.value!.nextCursor ?? undefined,
                page: 0,
              })
            }
          >
            Fetch next 100
          </Button>
        )}
      </div>
    </div>
  );
}
function Workflow({
  view,
  dock,
  model,
}: {
  view: WorkspaceView;
  dock: DockSession;
  model: WorkspaceModel;
}) {
  const [error, setError] = useState(""),
    [starting, setStarting] = useState(false);
  const id = view.resourceId;
  const list = useData<RunPage>(view.projectId, "runs?limit=25");
  const selected = id ?? list.value?.items[0]?.id;
  return selected ? (
    <WorkflowDetail
      key={`${view.projectId}:${selected}`}
      id={selected}
      view={view}
      dock={dock}
      model={model}
      error={error}
      starting={starting}
      start={() => {
        setStarting(true);
        void request(projectPath(view.projectId, `runs/${selected}/start`), { method: "POST" })
          .then(() => refreshData(view.projectId, `runs/${selected}`))
          .catch((e) => setError(String(e)))
          .finally(() => setStarting(false));
      }}
    />
  ) : list.error ? (
    <ErrorState
      error={list.error}
      retry={() => void refreshData(view.projectId, "runs?limit=25")}
    />
  ) : (
    <Boundary
      title={list.loading ? "Loading workflows" : "No workflow selected"}
      detail="Choose a real workflow from the workflow list."
      action={<Button onClick={() => dock.open("workflows")}>Open workflows</Button>}
    />
  );
}
function objectivePreview(text: string) {
  const first = text.split(/\r?\n\s*\r?\n/, 1)[0];
  const preview = first.slice(0, 240).trimEnd();
  return preview.length < text.length ? `${preview}…` : preview;
}
function WorkflowDetail({
  id,
  view,
  dock,
  error,
  starting,
  start,
}: {
  id: number;
  view: WorkspaceView;
  dock: DockSession;
  model: WorkspaceModel;
  error: string;
  starting: boolean;
  start: () => void;
}) {
  const path = `runs/${id}`,
    data = useData<RunDetail>(view.projectId, path),
    detail = data.value;
  if (data.error)
    return <ErrorState error={data.error} retry={() => void refreshData(view.projectId, path)} />;
  if (!detail) return <LoadingIndicator label="Loading workflow details" />;
  const current = detail.tasks.filter((t) => t.state !== "superseded");
  const completed = current.filter((t) => t.state === "completed").length;
  return (
    <div className="ws-scroll">
      <div className="ws-state-line">
        <span>Run status: {detail.run.status}</span>
        <span>#{id}</span>
      </div>
      <h2 className="ws-objective">{objectivePreview(detail.run.objective)}</h2>
      {detail.run.objective.includes("\n") || detail.run.objective.length > 240 ? (
        <details>
          <summary>Full objective and context</summary>
          <pre className="ws-code">{detail.run.objective}</pre>
        </details>
      ) : null}
      <div className="ws-actions">
        <Button onClick={() => dock.open("review", { projectId: view.projectId, resourceId: id })}>
          Open mission review
        </Button>
        <Button
          onClick={() => dock.open("sourceControl", { projectId: view.projectId, resourceId: id })}
        >
          Open mission Source Control
        </Button>
      </div>
      <p>
        {completed} of {current.length} current tasks completed
        {current.length !== detail.tasks.length &&
          ` · ${detail.tasks.length - current.length} superseded in history`}
      </p>
      {error && (
        <InlineAlert tone="danger" title="Workflow action failed">
          {error}
        </InlineAlert>
      )}
      {detail.run.status === "planned" && (
        <Button loading={starting} onClick={start}>
          Start workflow
        </Button>
      )}
      {view.kind === "sourceControl" ? (
        <>
          <h3>Integration branch</h3>
          <p className="ws-path">{detail.integration.branch}</p>
          <p className="ws-path">{detail.integration.head ?? "No integration commit recorded"}</p>
          <Button
            onClick={() =>
              dock.open("sourceControl", { projectId: view.projectId, resourceId: id })
            }
          >
            Open actual Source Control
          </Button>
        </>
      ) : (
        <>
          <div className="ws-task-list">
            {detail.tasks.map((task) => (
              <div key={task.id}>
                <span className="ws-state">{task.state}</span>
                <span>{task.title}</span>
                <small>{task.operation ?? "planning"}</small>
              </div>
            ))}
          </div>
          {(view.kind === "review" || view.kind === "tests") && (
            <>
              <h3>Recorded evidence</h3>
              {detail.artifacts.length ? (
                detail.artifacts.map((artifact) => (
                  <details key={artifact.id}>
                    <summary>
                      {artifact.kind} · {artifact.role}
                    </summary>
                    <pre>{artifact.content}</pre>
                  </details>
                ))
              ) : (
                <p>
                  No review/test artifact has been recorded. No passing test result is inferred.
                </p>
              )}
            </>
          )}
        </>
      )}
      {detail.attempts.length > 0 && (
        <>
          <h3>Task changes and review</h3>
          {detail.attempts.map((attempt) => (
            <AttemptEvidence key={attempt.id} attempt={attempt} />
          ))}
        </>
      )}
      <h3>Agent process sessions</h3>
      <HandoffControls
        key={`${view.projectId}:${id}`}
        projectId={view.projectId}
        runId={id}
        dock={dock}
      />
      {detail.sessions.length ? (
        detail.sessions.map((session) => (
          <Button
            key={session.id}
            onClick={() =>
              dock.open(
                "console",
                {
                  projectId: view.projectId,
                  resourceId: session.id,
                  title: `${session.agent} · session ${session.id}`,
                },
                "below"
              )
            }
          >
            {session.agent} · #{session.id} · {session.status}
          </Button>
        ))
      ) : (
        <p>No recorded agent sessions.</p>
      )}
    </div>
  );
}
function Agents({
  view,
  dock,
  sessions = false,
  embedded = false,
}: {
  view: WorkspaceView;
  dock: DockSession;
  sessions?: boolean;
  embedded?: boolean;
}) {
  const config = useData<ConfigData>(view.projectId, "config");
  if (config.error)
    return (
      <ErrorState error={config.error} retry={() => void refreshData(view.projectId, "config")} />
    );
  return (
    <div className={embedded ? "asf-legacy-agents" : "ws-scroll"}>
      <h2>{sessions ? "Existing agent sessions" : "Configured agents"}</h2>
      {!config.value ? (
        <LoadingIndicator label="Loading agents" />
      ) : (
        Object.entries(config.value.agents).map(([name, agent]) => (
          <div className="ws-agent" key={name}>
            <h3>{name}</h3>
            <p className="ws-path">{agent.command}</p>
            <p>
              {config
                .value!.role_assignments.filter((r) => r.agent === name)
                .map((r) => r.role)
                .join(", ") || "No assigned role"}
            </p>
            <AgentSessions projectId={view.projectId} name={name} dock={dock} />
          </div>
        ))
      )}
    </div>
  );
}
function AgentSessions({
  projectId,
  name,
  dock,
}: {
  projectId: string;
  name: string;
  dock: DockSession;
}) {
  const sessions = useData<AgentSession[]>(
    projectId,
    `agents/${encodeURIComponent(name)}/sessions`
  );
  return (
    <>
      {sessions.error ? (
        <p role="alert">{sessions.error}</p>
      ) : sessions.value?.length ? (
        sessions.value.slice(0, 20).map((session) => (
          <div className="ws-agent-session" key={session.id}>
            <span>
              #{session.id} · {session.status}
            </span>
            <Button
              onClick={() =>
                dock.open(
                  "console",
                  { projectId, resourceId: session.id, title: `${name} · #${session.id}` },
                  "below"
                )
              }
            >
              Open output
            </Button>
            {(session.interactive || session.mode === "interactive") && (
              <Button
                onClick={() =>
                  dock.open(
                    "terminal",
                    {
                      projectId,
                      resourceId: session.id,
                      title: `Terminal · ${name} #${session.id}`,
                    },
                    "below"
                  )
                }
              >
                Open terminal
              </Button>
            )}
          </div>
        ))
      ) : (
        <p className="ws-muted">No recorded sessions.</p>
      )}
    </>
  );
}
function SessionOutput({
  view,
  dock,
  terminal = false,
}: {
  view: WorkspaceView;
  dock: DockSession;
  terminal?: boolean;
}) {
  const stream = useSessionStream(view.projectId, view.resourceId!),
    data = stream.value;
  return (
    <div className="ws-output">
      {stream.error && (
        <ErrorState
          error={stream.error}
          retry={() => void sessionStreams.connect(view.projectId, view.resourceId!)}
        />
      )}
      <div className="ws-output-toolbar">
        <span>
          AgentSession #{view.resourceId} · {data?.agent ?? "connecting"} ·{" "}
          {data?.status ?? "unknown"}
        </span>
        {data && !terminal && (data.interactive || data.mode === "interactive") && (
          <Button
            onClick={() =>
              dock.open(
                "terminal",
                { projectId: view.projectId, resourceId: data.id, title: `Terminal #${data.id}` },
                "within"
              )
            }
          >
            Open terminal
          </Button>
        )}
      </div>
      {!terminal && (
        <>
          <SessionConfiguration
            key={`${view.projectId}:${view.resourceId}`}
            projectId={view.projectId}
            sessionId={view.resourceId!}
          />
          <MemoryUsage
            key={`memory:${view.projectId}:${view.resourceId}`}
            projectId={view.projectId}
            sessionId={view.resourceId!}
          />
        </>
      )}
      {data ? (
        terminal && (data.interactive || data.mode === "interactive") ? (
          <TerminalView projectId={view.projectId} session={data} />
        ) : (
          <pre
            tabIndex={0}
            data-working-directory={data.workingDirectory}
            aria-label={`Recorded output ${view.projectId} session ${data.id}`}
          >
            {data.stdout ?? ""}
            {data.stderr ? `\n${data.stderr}` : ""}
            {!data.stdout && !data.stderr ? "No process output was recorded." : ""}
          </pre>
        )
      ) : (
        !stream.error && <LoadingIndicator label="Connecting to session stream" />
      )}
    </div>
  );
}
