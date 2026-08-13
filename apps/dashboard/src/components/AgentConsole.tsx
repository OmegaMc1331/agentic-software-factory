import { useEffect, useMemo, useRef, useState } from "react";
import { agentSessionStreamUrl, fetchAgentSessions } from "../api";
import { connectionKind } from "../graphWorkspace";
import type { AgentActivity, AgentMeta, AgentSession, GraphNode } from "../types";

type AgentTab = "overview" | "console" | "sessions";

const ANSI = new RegExp(`${String.fromCharCode(27)}\\[([0-9;]*)m`, "g");
const ANSI_COLORS: Record<number, string> = {
  30: "#9097a5",
  31: "#d96b63",
  32: "#65b883",
  33: "#d6a85f",
  34: "#78a5e8",
  35: "#b49ad9",
  36: "#69b5bc",
  37: "#d8dce4",
  90: "#737b89",
  91: "#ef8178",
  92: "#7aca98",
  93: "#e5bb73",
  94: "#91b6ee",
  95: "#c4ace4",
  96: "#83c8ce",
  97: "#f2f4f7",
};

function AnsiText({ value }: { value: string }) {
  const fragments: React.ReactNode[] = [];
  let cursor = 0;
  let color: string | undefined;
  let bold = false;
  let match: RegExpExecArray | null;
  ANSI.lastIndex = 0;
  while ((match = ANSI.exec(value)) !== null) {
    if (match.index > cursor) {
      fragments.push(
        <span key={cursor} style={{ color, fontWeight: bold ? 650 : undefined }}>
          {value.slice(cursor, match.index)}
        </span>
      );
    }
    const codes = match[1] ? match[1].split(";").map(Number) : [0];
    for (const code of codes) {
      if (code === 0) {
        color = undefined;
        bold = false;
      } else if (code === 1) {
        bold = true;
      } else if (code === 22) {
        bold = false;
      } else if (code === 39) {
        color = undefined;
      } else if (ANSI_COLORS[code]) {
        color = ANSI_COLORS[code];
      }
    }
    cursor = ANSI.lastIndex;
  }
  if (cursor < value.length) {
    fragments.push(
      <span key={cursor} style={{ color, fontWeight: bold ? 650 : undefined }}>
        {value.slice(cursor)}
      </span>
    );
  }
  return <>{fragments}</>;
}

function duration(session: AgentSession): string {
  if (session.durationMs === null) return "";
  const seconds = Math.round(session.durationMs / 1000);
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

function sessionState(session: AgentSession | null): string {
  if (!session) return "Available";
  if (["running", "active"].includes(session.status)) return "Running";
  return session.exitCode === 0 || session.status === "success" ? "Completed" : "Failed";
}

export function AgentConsole({
  agentName,
  meta,
  activity,
  nodesById = new Map(),
  onClose,
  onDelete,
  onConnect,
}: {
  agentName: string;
  meta: AgentMeta;
  activity: AgentActivity | null;
  nodesById?: Map<string, GraphNode>;
  onClose: () => void;
  onDelete: () => void;
  onConnect?: (targetId: string) => void;
}) {
  const [tab, setTab] = useState<AgentTab>("console");
  const [sessions, setSessions] = useState<AgentSession[]>([]);
  const [selectedSessionId, setSelectedSessionId] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [follow, setFollow] = useState(true);
  const [connectionTarget, setConnectionTarget] = useState("");
  const outputRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    setError(null);
    fetchAgentSessions(agentName)
      .then((next) => {
        setSessions(next);
        setSelectedSessionId((current) => current ?? next[0]?.id ?? null);
      })
      .catch((reason: Error) => setError(reason.message));
  }, [agentName]);

  const session = useMemo(
    () => sessions.find((candidate) => candidate.id === selectedSessionId) ?? null,
    [selectedSessionId, sessions]
  );
  const sessionId = session?.id;
  const sessionStatus = session?.status;

  useEffect(() => {
    if (!sessionId || !sessionStatus || !["running", "active"].includes(sessionStatus)) return;
    const source = new EventSource(agentSessionStreamUrl(sessionId));
    source.addEventListener("session", (event) => {
      const next = JSON.parse((event as MessageEvent).data) as AgentSession;
      setSessions((current) =>
        current.map((candidate) => (candidate.id === next.id ? next : candidate))
      );
      if (!["running", "active"].includes(next.status)) source.close();
    });
    source.onerror = () => {
      setError("The agent session stream disconnected.");
      source.close();
    };
    return () => source.close();
  }, [sessionId, sessionStatus]);

  useEffect(() => {
    if (follow && outputRef.current) outputRef.current.scrollTop = outputRef.current.scrollHeight;
  }, [follow, session?.stdout, session?.stderr]);

  const state = sessionState(session);
  const agentNode = nodesById.get(`agent:${agentName}`);
  const connectionTargets = agentNode
    ? Array.from(nodesById.values())
        .filter((candidate) => connectionKind(agentNode, candidate) !== null)
        .sort((a, b) => a.label.localeCompare(b.label))
    : [];

  return (
    <aside className="agent-console" aria-label={`${agentName} Agent Console`}>
      <div className="agent-console-head">
        <div>
          <span className="inspector-kind">Agent Console</span>
          <h3>{agentName}</h3>
        </div>
        <div className="agent-console-state" aria-live="polite">
          <span className={state === "Failed" ? "net-bad" : "net-ok"}>{state}</span>
          {activity?.taskId ? <span>Task #{activity.taskId}</span> : null}
        </div>
        <button className="inspector-close" onClick={onClose} aria-label="Close Agent Console">
          x
        </button>
      </div>

      <div className="agent-console-tabs" role="tablist" aria-label="Agent details">
        {(["overview", "console", "sessions"] as AgentTab[]).map((name) => (
          <button
            key={name}
            role="tab"
            aria-selected={tab === name}
            className={tab === name ? "is-active" : ""}
            onClick={() => setTab(name)}
          >
            {name[0].toUpperCase() + name.slice(1)}
          </button>
        ))}
      </div>

      {error && <p className="inline-error">{error}</p>}

      {tab === "overview" && (
        <div className="agent-overview">
          <dl>
            <div>
              <dt>Command</dt>
              <dd>
                <code>{meta.command}</code>
              </dd>
            </div>
            <div>
              <dt>Availability</dt>
              <dd>{meta.available ? "Available" : "Missing"}</dd>
            </div>
            <div>
              <dt>Assigned roles</dt>
              <dd>{meta.roles.length ? meta.roles.join(", ") : "None"}</dd>
            </div>
            <div>
              <dt>Current task</dt>
              <dd>{activity?.taskId ? `#${activity.taskId}` : "None"}</dd>
            </div>
          </dl>
          {onConnect && connectionTargets.length > 0 && (
            <div className="inspector-connect">
              <label htmlFor="agent-connection-target">Add supported connection</label>
              <div>
                <select
                  id="agent-connection-target"
                  value={connectionTarget}
                  onChange={(event) => setConnectionTarget(event.target.value)}
                >
                  <option value="">Choose target</option>
                  {connectionTargets.map((candidate) => (
                    <option key={candidate.id} value={candidate.id}>
                      {candidate.label} ({candidate.kind})
                    </option>
                  ))}
                </select>
                <button
                  className="button"
                  disabled={!connectionTarget}
                  onClick={() => {
                    onConnect(connectionTarget);
                    setConnectionTarget("");
                  }}
                >
                  Connect
                </button>
              </div>
            </div>
          )}
          <button className="button inspector-delete" onClick={onDelete}>
            Remove agent
          </button>
        </div>
      )}

      {tab === "console" && (
        <div className="agent-console-body">
          {!session ? (
            <div className="agent-console-idle">
              <strong>No active session.</strong>
              <span>This configured agent is currently idle.</span>
            </div>
          ) : (
            <>
              <div className="agent-session-meta">
                <span>{session.status}</span>
                {session.exitCode !== null && <span>Exit code {session.exitCode}</span>}
                {session.durationMs !== null && <span>{duration(session)}</span>}
                <span>{new Date(session.startedAt).toLocaleString()}</span>
                <code>{session.workingDirectory}</code>
              </div>
              <div
                className="agent-terminal"
                ref={outputRef}
                tabIndex={0}
                onScroll={(event) => {
                  const element = event.currentTarget;
                  setFollow(element.scrollHeight - element.scrollTop - element.clientHeight < 24);
                }}
              >
                {session.stdout ? (
                  <pre>
                    <AnsiText value={session.stdout} />
                  </pre>
                ) : null}
                {session.stderr ? (
                  <pre className="agent-terminal-stderr">
                    <AnsiText value={session.stderr} />
                  </pre>
                ) : null}
                {!session.stdout && !session.stderr && (
                  <p className="agent-terminal-empty">No process output was recorded.</p>
                )}
              </div>
              {!follow && (
                <button className="agent-follow" onClick={() => setFollow(true)}>
                  Follow output
                </button>
              )}
              <p className="agent-console-input-note">
                {session.interactive
                  ? "Interactive input is available for this Factory session."
                  : "This agent session is non-interactive."}
              </p>
            </>
          )}
        </div>
      )}

      {tab === "sessions" && (
        <div className="agent-session-list">
          {sessions.length === 0 ? (
            <p>No recorded sessions for this agent.</p>
          ) : (
            sessions.map((candidate, index) => (
              <button
                key={candidate.id}
                className={candidate.id === selectedSessionId ? "is-active" : ""}
                onClick={() => {
                  setSelectedSessionId(candidate.id);
                  setTab("console");
                }}
              >
                <span>
                  {index === 0 ? "Current / latest" : `Previous session #${candidate.id}`}
                </span>
                <small>
                  {sessionState(candidate)}
                  {candidate.durationMs === null ? "" : ` / ${duration(candidate)}`}
                </small>
              </button>
            ))
          )}
        </div>
      )}
    </aside>
  );
}
