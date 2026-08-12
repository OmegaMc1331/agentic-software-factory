import type { GraphNode, TaskState } from "../types";
import { agentMeta, roleMeta, runMeta, taskMeta } from "../types";
import type { AgentActivity } from "../types";
import { STATE_META } from "../state";

function taskFlowNote(state: TaskState): string {
  switch (state) {
    case "completed":
      return "completed; terminal";
    case "running":
      return "execution in progress";
    case "failed":
      return "failed; blocks dependents";
    case "blocked":
      return "blocked by unmet dependencies";
    case "ready":
      return "ready to run";
    default:
      return "waiting on dependencies";
  }
}

function StateChip({ state }: { state: TaskState }) {
  const meta = STATE_META[state];
  return (
    <span className="net-chip" style={{ borderColor: meta.color, color: meta.color }}>
      {meta.label}
    </span>
  );
}

function DepRow({ id, nodesById }: { id: string; nodesById: Map<string, GraphNode> }) {
  const node = nodesById.get(id);
  const state = node?.kind === "task" ? taskMeta(node).state : null;
  const color = state ? STATE_META[state].color : "#8a93a3";
  const label = node?.kind === "task" ? `#${taskMeta(node).taskId}` : id;
  return (
    <div className="net-dep-row">
      <span className="net-status-dot" style={{ backgroundColor: color }} />
      <code>{label}</code>
      {node?.kind === "task" && (
        <span className="net-dep-state">{STATE_META[taskMeta(node).state].label}</span>
      )}
    </div>
  );
}

function Frame({
  node,
  onClose,
  children,
}: {
  node: GraphNode;
  onClose: () => void;
  children: React.ReactNode;
}) {
  return (
    <aside className="net-inspector">
      <div className="inspector-header">
        <span className="inspector-kind">{node.kind}</span>
        <button className="inspector-close" onClick={onClose} aria-label="Close inspector">
          ×
        </button>
      </div>
      <h3 className="inspector-title">{node.label}</h3>
      <div className="inspector-body">{children}</div>
    </aside>
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="inspector-row">
      <span className="inspector-label">{label}</span>
      {children}
    </div>
  );
}

export function NodeInspector({
  node,
  nodesById,
  activity,
  onClose,
}: {
  node: GraphNode | null;
  nodesById: Map<string, GraphNode>;
  activity: AgentActivity | null;
  onClose: () => void;
}) {
  if (!node) {
    return (
      <aside className="net-inspector net-inspector--empty">
        <p className="inspector-hint">Hover or click a node to inspect it.</p>
      </aside>
    );
  }

  if (node.kind === "agent") {
    const meta = agentMeta(node);
    return (
      <Frame node={node} onClose={onClose}>
        <Row label="Status">
          <span className={meta.available ? "net-ok" : "net-bad"}>
            {meta.available ? "available" : "missing"}
          </span>
        </Row>
        <Row label="Roles">
          <span className="inspector-value">
            {meta.roles.length === 0 ? "–" : meta.roles.join(", ")}
          </span>
        </Row>
        <Row label="Activity">
          <span className={activity ? "net-ok" : "inspector-value"}>
            {activity
              ? activity.taskId !== null
                ? `working · #${activity.taskId} (run #${activity.runId})`
                : `orchestrating run #${activity.runId}`
              : "idle"}
          </span>
        </Row>
        <Row label="Command">
          <code>{meta.command}</code>
        </Row>
      </Frame>
    );
  }

  if (node.kind === "role") {
    const meta = roleMeta(node);
    return (
      <Frame node={node} onClose={onClose}>
        <Row label="Bound agent">
          <code>{meta.agent}</code>
        </Row>
      </Frame>
    );
  }

  if (node.kind === "run") {
    const meta = runMeta(node);
    const counts = meta.counts;
    const order: TaskState[] = ["pending", "ready", "running", "blocked", "failed", "completed"];
    return (
      <Frame node={node} onClose={onClose}>
        <Row label="Status">
          <span className="inspector-value">{meta.status}</span>
        </Row>
        {meta.objective && (
          <Row label="Objective">
            <span className="inspector-value inspector-objective">{meta.objective}</span>
          </Row>
        )}
        <Row label="Planner">
          <code>{meta.plannerAgent ?? "–"}</code>
        </Row>
        <Row label="Created">
          <span className="inspector-value">{new Date(meta.createdAt).toLocaleString()}</span>
        </Row>
        <Row label="Tasks">
          <span className="inspector-value">{counts.total}</span>
        </Row>
        <ul className="inspector-counts">
          {order.map((state) => {
            if (counts[state] === 0) return null;
            const metaColor = STATE_META[state];
            return (
              <li key={state}>
                <span className="net-status-dot" style={{ backgroundColor: metaColor.color }} />
                <span>{state}</span>
                <code>{counts[state]}</code>
              </li>
            );
          })}
        </ul>
      </Frame>
    );
  }

  const meta = taskMeta(node);
  const dependents: GraphNode[] = [];
  for (const other of nodesById.values()) {
    if (other.kind === "task" && taskMeta(other).dependencies.includes(meta.taskId)) {
      dependents.push(other);
    }
  }
  return (
    <Frame node={node} onClose={onClose}>
      <Row label="State">
        <StateChip state={meta.state} />
      </Row>
      <Row label="Flow">
        <span
          className={STATE_META[meta.state].color === "#dc2626" ? "net-bad" : "inspector-value"}
        >
          {taskFlowNote(meta.state)}
        </span>
      </Row>
      {meta.objective && (
        <Row label="Objective">
          <span className="inspector-value inspector-objective">{meta.objective}</span>
        </Row>
      )}
      <Row label="Run">
        <code>#{meta.runId}</code>
      </Row>
      <Row label="Depends on">
        {meta.dependencies.length === 0 ? (
          <span className="inspector-value">–</span>
        ) : (
          <span className="inspector-deps">
            {meta.dependencies.map((id) => (
              <DepRow key={id} id={`task:${id}`} nodesById={nodesById} />
            ))}
          </span>
        )}
      </Row>
      <Row label="Blocks">
        {dependents.length === 0 ? (
          <span className="inspector-value">–</span>
        ) : (
          <span className="inspector-deps">
            {dependents.map((dep) => (
              <DepRow key={dep.id} id={dep.id} nodesById={nodesById} />
            ))}
          </span>
        )}
      </Row>
      {meta.worktreePath && (
        <Row label="Worktree">
          <code className="inspector-objective">{meta.worktreePath}</code>
        </Row>
      )}
    </Frame>
  );
}
