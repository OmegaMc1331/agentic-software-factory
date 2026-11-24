import { useEffect, useRef, useState, useSyncExternalStore, memo, lazy, Suspense } from "react";
import { request } from "../../api";
import { HandoffControls } from "../agents/HandoffControls";
import { MissionLifecycleControls } from "../lifecycle/LifecycleControls";
import { projectPath } from "../projects/projectApi";
import { Button, Input, Select, Textarea } from "../design-system/primitives/Controls";
import { InlineAlert, LoadingIndicator } from "../design-system/components/Application";
import { useData, refreshData } from "../workspace/production/data";
import type { WorkspaceModel, DockSession } from "../workspace/production/controller";
import type { WorkspaceView } from "../workspace/production/model";
import { useConversation, conversationPath } from "./client";
import type {
  ContextReference,
  Conversation,
  Message,
  Mission,
  MissionRequest,
  Submission,
} from "./types";
const MissionForm = lazy(() =>
  import("./MissionForm").then((module) => ({ default: module.MissionForm }))
);
import "./conversation.css";
interface Props {
  view: WorkspaceView;
  model: WorkspaceModel;
  dock: DockSession;
}
export function ConversationPanel({ view, model, dock }: Props) {
  const [cursor, setCursor] = useState("");
  const listPath = `conversations?limit=100${cursor ? `&cursor=${cursor}` : ""}`;
  const list = useData<Conversation[]>(view.projectId, listPath);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [filter, setFilter] = useState("");
  const selected = view.state.conversationId;
  const choose = (id: string) => {
    model.updateView(view.sessionId, view.id, { conversationId: id });
    const session = model.state.document.sessions.find((s) => s.id === view.sessionId);
    for (const panel of session?.views ?? [])
      if (
        panel.kind === "progress" &&
        panel.projectId === view.projectId &&
        !panel.resourceId &&
        panel.state.conversationId !== id
      )
        model.updateView(view.sessionId, panel.id, { conversationId: id });
  };
  return (
    <div className="chat-panel">
      <div className="chat-toolbar">
        <Select
          label="Conversation"
          value={selected ?? ""}
          onChange={(event) => choose(event.target.value)}
        >
          <option value="">Select a conversation</option>
          {selected && !list.value?.some((c) => c.id === selected) && (
            <option value={selected}>Saved conversation</option>
          )}
          {list.value
            ?.filter(
              (c) => c.title.toLowerCase().includes(filter.toLowerCase()) || c.id === selected
            )
            .map((c) => (
              <option key={c.id} value={c.id}>
                {c.title}
                {c.status === "archived" ? " · archived" : ""}
              </option>
            ))}
        </Select>
        <Button
          loading={busy}
          onClick={() => {
            setBusy(true);
            setError("");
            void request<Conversation>(projectPath(view.projectId, "conversations"), {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({ title: "Development conversation" }),
            })
              .then((c) => {
                choose(c.id);
                setCursor("");
                void refreshData(view.projectId, "conversations?limit=100");
              })
              .catch((error) => setError(String(error)))
              .finally(() => setBusy(false));
          }}
        >
          New conversation
        </Button>
        {selected && (
          <Button
            onClick={() => {
              const id = dock.open(
                "home",
                { projectId: view.projectId, title: "Conversation" },
                "right",
                true
              );
              if (id) model.updateView(view.sessionId, id, { conversationId: selected });
            }}
          >
            Open beside
          </Button>
        )}
      </div>
      {(cursor || list.value?.length === 100) && (
        <div className="chat-subtoolbar">
          <Button disabled={!cursor} onClick={() => setCursor("")}>
            Newest conversations
          </Button>
          <Button
            disabled={list.value?.length !== 100}
            onClick={() => setCursor(list.value?.at(-1)?.id ?? "")}
          >
            Next 100 conversations
          </Button>
          <span>Filter applies to the fetched page.</span>
        </div>
      )}
      {list.value && list.value.length > 10 && (
        <Input
          label="Filter conversations"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
      )}
      {(error || list.error) && (
        <InlineAlert title="Conversation unavailable" tone="danger">
          {error || list.error}
          <Button onClick={() => void refreshData(view.projectId, listPath)}>Retry history</Button>
        </InlineAlert>
      )}
      {selected ? (
        <ConversationContent
          key={`${view.projectId}:${selected}`}
          {...{ view, model, dock }}
          id={selected}
        />
      ) : (
        <div className="chat-empty">
          <h2>Orchestrator</h2>
          <p>
            Create or select a conversation for this project. Questions use the configured Planner
            agent; development requests create a real workflow under existing project policies.
          </p>
          <p>History stays in this project. Closing this view does not cancel execution.</p>
        </div>
      )}
    </div>
  );
}
function ConversationContent({ view, model, dock, id }: Props & { id: string }) {
  const { store, state } = useConversation(view.projectId, id);
  const meta = useData<Conversation>(view.projectId, `conversations/${id}`);
  const status = useData<{ available: boolean; agent?: string; error?: string }>(
    view.projectId,
    "conversations/status"
  );
  const [draft, setDraft] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [mode, setMode] = useState<"chat" | "form" | "visual">("chat");
  useEffect(() => {
    if (view.state.missionMode) setMode(view.state.missionMode);
  }, [view.state.missionMode]);
  const [attachments, setAttachments] = useState<ContextReference[]>([]);
  const [fileId, setFileId] = useState("");
  const [path, setPath] = useState("");
  const [title, setTitle] = useState("");
  const [selection, setSelection] = useState(false);
  const [includeDiagnostics, setIncludeDiagnostics] = useState(false);
  const [draftKey, setDraftKey] = useState("");
  const composer = useRef<HTMLTextAreaElement>(null);
  const command = useRef<Submission>();
  const bottom = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  useSyncExternalStore(model.documents.subscribe, () =>
    model.documents
      .all()
      .map((d) => `${d.id}:${d.generation}:${d.status}`)
      .join("|")
  );
  const documents = model.documents.all();
  useSyncExternalStore(model.language.subscribe, model.language.snapshot);
  useEffect(() => {
    let active = true;
    void model.documents.clientIdentity().then((owner) => {
      if (!active) return;
      const key = `asf.v2.composer.1:${owner}:${view.projectId}:${view.sessionId}:${id}`;
      setDraftKey(key);
      try {
        const raw = localStorage.getItem(key);
        if (raw) {
          const stored = JSON.parse(raw) as { text: string; pending?: Submission };
          if (typeof stored.text === "string" && stored.text.length <= 16000) setDraft(stored.text);
          if (stored.pending) {
            command.current = stored.pending;
            setError(
              "A submission acknowledgment may be missing. Retry retained submission to reconcile its command key before sending new work."
            );
          }
        }
      } catch {
        setError("Composer recovery is unavailable. Keep a copy before closing this window.");
      }
    });
    return () => {
      active = false;
    };
  }, [model, id, view.projectId, view.sessionId]);
  useEffect(() => {
    if (!draftKey) return;
    try {
      localStorage.setItem(draftKey, JSON.stringify({ text: draft, pending: command.current }));
    } catch {
      setError("Composer recovery could not be stored. Your text is still in this view.");
    }
  }, [draft, draftKey]);
  useEffect(() => {
    if (following.current) bottom.current?.scrollIntoView({ block: "end" });
  }, [state.messages]);
  useEffect(() => {
    const focus = (event: KeyboardEvent) => {
      if (
        event.altKey &&
        event.key.toLowerCase() === "c" &&
        model.state.document.activeSessionId === view.sessionId &&
        (model.session?.activeViewId === view.id ||
          model.session?.views.find((v) => v.kind === "home" && !v.closed && !v.hidden)?.id ===
            view.id)
      ) {
        event.preventDefault();
        composer.current?.focus();
      }
    };
    window.addEventListener("keydown", focus);
    return () => window.removeEventListener("keydown", focus);
  }, [model, dock, view.id, view.sessionId]);
  const active = [...state.messages]
    .reverse()
    .find(
      (message) =>
        !["finished", "failed", "interrupted", "cancelled", "blocked", "timed_out"].includes(
          message.status
        ) && message.role === "orchestrator"
    );
  const send = async (mission?: MissionRequest) => {
    const body = command.current ?? {
      key: crypto.randomUUID(),
      workspace_session_id: model.missionSession(view.projectId, view.sessionId),
      content: mission?.objective ?? draft,
      context: attachments,
      ...(mission ? { mission } : {}),
    };
    if (!body.content.trim()) return;
    command.current = body;
    setBusy(true);
    setError("");
    try {
      if (draftKey) localStorage.setItem(draftKey, JSON.stringify({ text: draft, pending: body }));
      const accepted = await store.api.send(body);
      command.current = undefined;
      setDraft("");
      setAttachments([]);
      if (draftKey) localStorage.removeItem(draftKey);
      setMode("chat");
      await store.refresh();
      if (model.state.document.activeProjectId === view.projectId) {
        const panel = dock.open("progress", { projectId: view.projectId }, "right");
        if (panel) model.updateView(view.sessionId, panel, { conversationId: id });
      }
      model.announce(
        `Message accepted as command ${accepted.id}. Backend processing continues independently of this view.`
      );
    } catch (error) {
      setError(
        `${String(error)} Your submission key and text are retained; Retry submission reconciles uncertain acceptance.`
      );
      throw error;
    } finally {
      setBusy(false);
    }
  };
  const attach = () => {
    const document = documents.find((d) => d.id === fileId && d.file.project_id === view.projectId);
    if (!document || document.status !== "ready") {
      setError("Open a readable file in this project first.");
      return;
    }
    const range = model.documents.contextSelection(document.id);
    const content = document.text.sliceString(
      0,
      document.text.length,
      document.newline === "crlf" ? "\r\n" : "\n"
    );
    const from = selection && range && !range.empty ? range.from : 0,
      to = selection && range && !range.empty ? range.to : document.text.length;
    const byteOffset = (offset: number) =>
      new TextEncoder().encode(
        document.text.sliceString(0, offset, document.newline === "crlf" ? "\r\n" : "\n")
      ).length;
    const reference: ContextReference = {
      file: document.file,
      revision: document.baseRevision,
      ...(document.dirty ? { buffer: content } : {}),
      ...(includeDiagnostics
        ? {
            diagnostic: [...model.language.connections.values()]
              .filter((connection) => connection.projectId === view.projectId)
              .flatMap((connection) => [...connection.diagnostics.values()])
              .filter(
                (set) =>
                  set.file.root_id === document.file.root_id &&
                  set.file.path === document.file.path &&
                  set.version === document.generation
              )
              .flatMap((set) => set.diagnostics.map((d) => d.message))
              .slice(0, 8)
              .join("\n")
              .slice(0, 2000),
          }
        : {}),
      ...(selection && range && !range.empty
        ? { selection: [byteOffset(from), byteOffset(to)] as [number, number] }
        : {}),
    };
    if (attachments.length >= 8) {
      setError("At most eight file references may be attached.");
      return;
    }
    setAttachments([...attachments, reference]);
  };
  return (
    <>
      <div className="chat-subtoolbar">
        <span>
          {status.value?.available
            ? `Planner · ${status.value.agent}`
            : "Orchestrator configuration required"}
        </span>
        <span role="status">{state.connected ? "Connected" : "Reconnecting"}</span>
        <Button onClick={() => setMode(mode === "form" ? "chat" : "form")}>Mission form</Button>
        <Button onClick={() => setMode(mode === "visual" ? "chat" : "visual")}>Visual plan</Button>
      </div>
      {!status.value?.available && (
        <InlineAlert title="Configure an orchestrator" tone="warning">
          {status.value?.error ?? status.error ?? "Checking configured Planner agent…"}
          <Button onClick={() => dock.open("settings")}>Project settings</Button>
          <a href={`/?project=${encodeURIComponent(view.projectId)}`}>
            Configure existing roles and agents
          </a>
        </InlineAlert>
      )}
      {(error || state.error || meta.error) && (
        <InlineAlert title="Conversation needs attention" tone="danger">
          {error || state.error || meta.error}
          <Button onClick={() => void store.refresh()}>Reload durable state</Button>
          {command.current && (
            <Button disabled={busy} onClick={() => void send().catch(() => {})}>
              Retry submission
            </Button>
          )}
        </InlineAlert>
      )}
      {mode !== "chat" ? (
        <div className="chat-mission-scroll">
          <Suspense fallback={<LoadingIndicator label="Loading mission editor" />}>
            <MissionForm
              project={view.projectId}
              mode={mode}
              onSubmit={send}
              onCancel={() => setMode("chat")}
            />
          </Suspense>
        </div>
      ) : (
        <>
          <div
            className="chat-history"
            aria-label="Conversation history"
            tabIndex={0}
            onScroll={(event) => {
              const node = event.currentTarget;
              following.current = node.scrollHeight - node.clientHeight - node.scrollTop < 90;
            }}
          >
            {state.loading && <LoadingIndicator label="Loading conversation history" />}
            {state.older && (
              <Button onClick={() => void store.older()}>
                Load earlier messages · up to 200 visible
              </Button>
            )}
            {!state.messages.length && !state.loading && (
              <p className="chat-hint">
                Ask a question or describe a development objective. Authorized missions are planned
                and started automatically.
              </p>
            )}
            {state.messages.map((message) => (
              <MessageRow
                key={message.id}
                message={message}
                view={view}
                dock={dock}
                openMemory={(memoryId) => {
                  const panel = dock.open("memory", { projectId: view.projectId }, "within", true);
                  if (panel) model.updateView(dock.sessionId, panel, { memoryId });
                }}
                onRetry={() => {
                  if (command.current) {
                    setError("Reconcile the pending submission before starting another.");
                    return;
                  }
                  const user = state.messages.find((m) => m.id === message.parent_message_id);
                  if (user) {
                    const text = draft ? `${draft}\n\n${user.content}` : user.content;
                    if (text.length > 16000) {
                      setError("Composer is full; copy the objective into a new conversation.");
                      return;
                    }
                    setDraft(text);
                    composer.current?.focus();
                  }
                }}
              />
            ))}
            <div ref={bottom} />
          </div>
          <div className="chat-composer-area">
            {active && (
              <div className="chat-actions">
                <span role="status">
                  {active.run_id
                    ? `Workflow #${active.run_id} · ${state.missions.find((mission) => mission.run_id === active.run_id)?.status ?? active.status}`
                    : "Configured agent is processing the message; no partial answer has been returned."}
                </span>
                <Button
                  tone="danger"
                  onClick={() => {
                    const commandId = active.provenance.command_id;
                    if (commandId)
                      void store.api
                        .cancel(commandId)
                        .then(() => store.refresh())
                        .catch((error) => setError(String(error)));
                  }}
                >
                  Cancel active interaction
                </Button>
              </div>
            )}
            <details className="chat-context">
              <summary>
                Attach project context{attachments.length ? ` · ${attachments.length}` : ""}
              </summary>
              <div className="chat-context-controls">
                <Select
                  label="Open editor file"
                  value={fileId}
                  onChange={(e) => setFileId(e.target.value)}
                >
                  <option value="">Select an open file</option>
                  {documents
                    .filter((d) => d.file.project_id === view.projectId)
                    .map((d) => (
                      <option key={d.id} value={d.id}>
                        {d.file.path}
                        {` · ${d.file.root_id.slice(-8)}`}
                        {d.disk?.read_only ? " · read-only" : ""}
                        {d.dirty ? " · unsaved" : ""}
                      </option>
                    ))}
                </Select>
                <label>
                  <input
                    type="checkbox"
                    checked={selection}
                    onChange={(e) => setSelection(e.target.checked)}
                  />{" "}
                  Use editor selection when present
                </label>
                <label>
                  <input
                    type="checkbox"
                    checked={includeDiagnostics}
                    onChange={(event) => setIncludeDiagnostics(event.target.checked)}
                  />{" "}
                  Include current language diagnostics
                </label>
                <Button onClick={attach}>Attach editor context</Button>
                <Input
                  label="Repository-relative path"
                  value={path}
                  onChange={(e) => setPath(e.target.value)}
                />
                <Button
                  onClick={() => {
                    void model.documents.api
                      .roots(view.projectId)
                      .then((catalog) => {
                        const root = catalog.roots.find((root) => root.kind === "repository");
                        if (!root) throw new Error("Repository root unavailable.");
                        return model.documents.api.read({
                          project_id: view.projectId,
                          root_id: root.id,
                          path,
                        });
                      })
                      .then((file) => {
                        setAttachments([
                          ...attachments,
                          { file: file.file, revision: file.revision },
                        ]);
                        setPath("");
                      })
                      .catch((error) => setError(String(error)));
                  }}
                >
                  Attach saved file
                </Button>
              </div>
            </details>
            {attachments.map((reference, index) => (
              <div
                className="chat-reference"
                key={`${reference.file.root_id}:${reference.file.path}:${index}`}
              >
                <span>
                  {reference.file.path} · {reference.file.root_id.slice(-8)} ·{" "}
                  {reference.buffer !== undefined ? "unsaved buffer" : "saved file"}
                  {reference.selection ? " · selection" : ""}
                </span>
                <Button onClick={() => setAttachments(attachments.filter((_, i) => i !== index))}>
                  Remove reference
                </Button>
              </div>
            ))}
            <form
              onSubmit={(event) => {
                event.preventDefault();
                void send().catch(() => {});
              }}
            >
              <Textarea
                ref={composer}
                label="Message to orchestrator"
                value={draft}
                maxLength={16000}
                placeholder="Ask about the project or describe a development objective…"
                onChange={(e) => setDraft(e.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    if (draft.trim() && !busy && !active) void send().catch(() => {});
                  }
                }}
              />
              <div className="chat-actions">
                <span>Enter sends · Shift+Enter inserts a line · Alt+C focuses composer</span>
                <Button
                  type="submit"
                  tone="primary"
                  loading={busy}
                  disabled={
                    !draft.trim() ||
                    busy ||
                    !!active ||
                    !!command.current ||
                    meta.value?.status === "archived"
                  }
                >
                  Send
                </Button>
              </div>
            </form>
            <details>
              <summary>Conversation metadata and retention</summary>
              <Input
                label="Conversation title"
                value={title || meta.value?.title || ""}
                onChange={(e) => setTitle(e.target.value)}
              />
              <Button
                onClick={() => {
                  if (meta.value)
                    void request(conversationPath(view.projectId, id), {
                      method: "PATCH",
                      headers: { "Content-Type": "application/json" },
                      body: JSON.stringify({
                        title: title || meta.value.title,
                        status: "active",
                        revision: meta.value.revision,
                      }),
                    })
                      .then(() => {
                        void refreshData(view.projectId, `conversations/${id}`);
                        void refreshData(view.projectId, "conversations?limit=100");
                      })
                      .catch((error) => setError(String(error)));
                }}
              >
                Rename conversation
              </Button>
              <Button
                disabled={!!active}
                onClick={() => {
                  if (meta.value)
                    void request(conversationPath(view.projectId, id), {
                      method: "PATCH",
                      headers: { "Content-Type": "application/json" },
                      body: JSON.stringify({
                        title: meta.value.title,
                        status: meta.value.status === "archived" ? "active" : "archived",
                        revision: meta.value.revision,
                      }),
                    })
                      .then(() => refreshData(view.projectId, `conversations/${id}`))
                      .catch((error) => setError(String(error)));
                }}
              >
                {meta.value?.status === "archived"
                  ? "Restore conversation"
                  : "Archive conversation"}
              </Button>
              <p>
                History is stored in this project’s SQLite database. Bounded context is supplied to
                the configured agent, whose provider may process it externally. Archiving preserves
                history; it does not delete it.
              </p>
            </details>
          </div>
        </>
      )}
    </>
  );
}
export const MessageRow = memo(function MessageRow({
  message,
  view,
  dock,
  onRetry,
  openMemory,
}: {
  openMemory?: (memoryId: string) => void;
  onRetry?: () => void;
  message: Message;
  view: WorkspaceView;
  dock: DockSession;
}) {
  const text = message.content.split(/(```[\s\S]*?```)/g);
  const recordedLifecycle = message.provenance.snapshot?.lifecycle;
  const displayStatus =
    (recordedLifecycle && !["active", "settled"].includes(recordedLifecycle.state)
      ? recordedLifecycle.state.replaceAll("_", " ")
      : message.provenance.snapshot?.status) ??
    (["planning", "running"].includes(message.status)
      ? "processing"
      : message.status.replaceAll("_", " "));
  const reference = message.run_id ?? message.provenance.snapshot?.run_id;
  const linkedRun =
    typeof reference === "number" && Number.isSafeInteger(reference) && reference > 0
      ? reference
      : undefined;
  return (
    <article
      className="chat-message"
      data-role={message.role}
      data-message-id={message.id}
      role={
        message.status === "failed" ||
        message.status === "interrupted" ||
        message.status === "timed_out" ||
        message.provenance.error ||
        message.provenance.snapshot?.status === "failed"
          ? "alert"
          : undefined
      }
    >
      <header>
        <strong>
          {message.role === "user"
            ? "You"
            : message.provenance.source === "runtime"
              ? "Mission result"
              : "Orchestrator"}
        </strong>
        {message.role === "orchestrator" && displayStatus !== "finished" && (
          <span>{displayStatus}</span>
        )}
        {onRetry &&
          (message.status === "failed" ||
            message.status === "interrupted" ||
            message.status === "timed_out" ||
            message.provenance.snapshot?.status === "failed") && (
            <Button onClick={onRetry}>Copy objective to composer</Button>
          )}
        {message.provenance.agent_session_id && (
          <Button
            onClick={() =>
              dock.open("console", {
                projectId: view.projectId,
                resourceId: message.provenance.agent_session_id,
              })
            }
          >
            Agent session
          </Button>
        )}
        {message.provenance.memory_references?.map((id) => (
          <Button
            key={id}
            onClick={() =>
              openMemory ? openMemory(id) : dock.open("memory", { projectId: view.projectId })
            }
          >
            Inspect memory {id}
          </Button>
        ))}
      </header>
      <div className="chat-message-content">
        {text.map((part, index) =>
          part.startsWith("```") ? (
            <pre key={index} tabIndex={0}>
              <code>{part.slice(3, -3).replace(/^\w*\n/, "")}</code>
            </pre>
          ) : (
            <p key={index}>{part}</p>
          )
        )}
      </div>
      {message.role === "orchestrator" && !message.content && (
        <p role="status">Awaiting validated agent response.</p>
      )}
      {message.provenance.context?.map((reference, index) => (
        <Button key={index} onClick={() => void dock.openFile(reference.file)}>
          {reference.file.path} · {reference.file.root_id.slice(-8)} · {reference.state}
          {reference.read_only ? " · read-only" : ""}
        </Button>
      ))}
      {linkedRun && (
        <div className="chat-actions">
          <Button
            onClick={() =>
              dock.open("progress", { projectId: view.projectId, resourceId: linkedRun }, "right")
            }
          >
            Workflow #{linkedRun}
          </Button>
          <Button
            onClick={() =>
              dock.open("workflow", { projectId: view.projectId, resourceId: linkedRun })
            }
          >
            Evidence and review
          </Button>
        </div>
      )}
      {typeof message.provenance.error === "string" &&
        message.provenance.error !== message.content && <p>{message.provenance.error}</p>}
    </article>
  );
});
export function ConversationProgress({ view, model, dock }: Props) {
  const workspace = useSyncExternalStore(model.subscribe, model.snapshot);
  const session = workspace.document.sessions.find((s) => s.id === view.sessionId);
  const id =
    view.state.conversationId ??
    session?.views.find(
      (v) => v.kind === "home" && v.projectId === view.projectId && v.state.conversationId
    )?.state.conversationId;
  return id ? (
    <ProgressContent key={`${view.projectId}:${id}`} view={view} dock={dock} id={id} />
  ) : (
    <div className="ws-scroll">
      <p>Select a conversation to follow its missions. Existing workflows remain available.</p>
      <Button onClick={() => dock.open("workflows")}>Browse workflows</Button>
    </div>
  );
}
function ProgressContent({
  view,
  dock,
  id,
}: {
  view: WorkspaceView;
  dock: DockSession;
  id: string;
}) {
  const { state } = useConversation(view.projectId, id);
  return (
    <div className="ws-scroll chat-progress" aria-label="Mission progression">
      {state.error && <p role="alert">{state.error}</p>}
      {!state.missions.length && <p>No mission has been started from this conversation.</p>}
      {state.missions.map((mission) => (
        <MissionProgress key={mission.run_id} mission={mission} view={view} dock={dock} />
      ))}
    </div>
  );
}
export function MissionProgress({
  mission,
  view,
  dock,
}: {
  mission: Mission;
  view: WorkspaceView;
  dock: DockSession;
}) {
  return (
    <section className="chat-mission">
      <header>
        <strong>
          #{mission.run_id} ·{" "}
          {mission.lifecycle && !["active", "settled"].includes(mission.lifecycle.state)
            ? mission.lifecycle.state.replaceAll("_", " ")
            : mission.status}
        </strong>
        <h3 title={mission.objective}>{mission.objective.split("\n")[0]}</h3>
      </header>
      <p>
        {mission.completed} / {mission.total} tasks completed
      </p>
      <p>Plan revision {mission.plan_revision ?? 1}</p>
      <MissionLifecycleControls
        key={`lifecycle:${view.projectId}:${mission.run_id}`}
        projectId={view.projectId}
        runId={mission.run_id}
      />
      {mission.active_tasks ? (
        <div className="chat-parallel" aria-label="Actual active tasks" aria-live="polite">
          <p>
            {mission.active_tasks.filter((task) => task.stage === "running").length} tasks executing
            · {mission.active_tasks.filter((task) => task.stage === "reviewing").length} awaiting
            review outcome
          </p>
          <ul>
            {mission.active_tasks.map((task) => (
              <li key={task.id}>
                <span>
                  #{task.id} {task.title}
                </span>
                <span>
                  {task.stage.replaceAll("_", " ")} · {task.role ?? "worker"}
                </span>
              </li>
            ))}
          </ul>
        </div>
      ) : mission.current ? (
        <p>
          Current: {mission.current.title} · {mission.current.role ?? "worker"}
        </p>
      ) : null}
      <HandoffControls
        key={`${view.projectId}:${mission.run_id}`}
        projectId={view.projectId}
        runId={mission.run_id}
        dock={dock}
      />
      {!!mission.plan_revisions?.length && (
        <details aria-label="Plan revision history">
          <summary>Plan history · {mission.plan_revisions.length} revisions</summary>
          {mission.plan_revisions.map((revision) => (
            <section key={revision.revision}>
              <h4>
                Revision {revision.revision} · {revision.source}
              </h4>
              {revision.reason && <p>{revision.reason}</p>}
              {revision.plannerSessionId && (
                <Button
                  onClick={() =>
                    dock.open("console", {
                      projectId: view.projectId,
                      resourceId: revision.plannerSessionId!,
                    })
                  }
                >
                  Planner session #{revision.plannerSessionId}
                </Button>
              )}
              <ul>
                {revision.tasks.map((task) => (
                  <li key={task.id}>
                    #{task.id} {task.title} · {task.state} · dependencies{" "}
                    {task.dependencies.length ? task.dependencies.join(", ") : "none"}
                  </li>
                ))}
              </ul>
            </section>
          ))}
        </details>
      )}
      {!!mission.integrations?.length && (
        <details aria-label="Integration history">
          <summary>
            Integration ·{" "}
            {mission.integrations.filter((record) => record.status === "published").length}{" "}
            published
          </summary>
          <p>
            Approved changes publish serially to factory/run-{mission.run_id}. The main working tree
            remains separate.
          </p>
          <ul>
            {mission.integrations.map((record) => (
              <li key={record.id}>
                Task #{record.taskId} · {record.status}
                <code>{record.candidateSha ?? record.sourceSha}</code>
              </li>
            ))}
          </ul>
        </details>
      )}
      {mission.status === "blocked" && (
        <InlineAlert title="Intervention required" tone="warning">
          Inspect workflow policies, roles and evidence before starting it again.
        </InlineAlert>
      )}
      <Button
        onClick={() =>
          dock.open("workflow", { projectId: view.projectId, resourceId: mission.run_id })
        }
      >
        Open workflow and review
      </Button>
      <details>
        <summary>Tasks, agents and evidence</summary>
        <ul>
          {mission.tasks.map((task) => (
            <li key={task.id}>
              <Button
                onClick={() =>
                  dock.open("workflow", { projectId: view.projectId, resourceId: mission.run_id })
                }
              >
                #{task.id} {task.title}
              </Button>
              <span>
                {task.state} · {task.role ?? "worker"}
              </span>
            </li>
          ))}
        </ul>
        <h4>Actual agent sessions</h4>
        {mission.sessions.map((session) => (
          <div className="chat-session" key={session.id}>
            <span>
              {session.agent} · {session.role} · {session.status}
            </span>
            <Button
              onClick={() =>
                dock.open("console", { projectId: view.projectId, resourceId: session.id })
              }
            >
              Session #{session.id}
            </Button>
          </div>
        ))}
        <h4>Recorded commands and reviews</h4>
        {mission.attempts.map((attempt) => (
          <div key={attempt.id}>
            <span>
              Attempt #{attempt.id} · {attempt.status} · exit {attempt.exit_code ?? "unknown"}
            </span>
            {attempt.commands?.map((command, i) => (
              <pre key={i}>{command}</pre>
            ))}
            {attempt.error && <p>{attempt.error}</p>}
            {attempt.review && <p>Review: {attempt.review.decision}</p>}
          </div>
        ))}
        <h4>Changed files from evidence</h4>
        {mission.changed_files.map((file) => (
          <p key={file}>{file}</p>
        ))}
        {mission.integration_sha && (
          <p>
            Integration: <code>{mission.integration_sha}</code>
          </p>
        )}
      </details>
    </section>
  );
}
