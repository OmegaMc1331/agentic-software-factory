import type { GraphNode, TaskState } from "../types";
import { agentMeta, roleMeta, runMeta, taskMeta } from "../types";
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

export function NodeInfo({
  node,
  nodesById,
  onClose,
}: {
  node: GraphNode | null;
  nodesById: Map<string, GraphNode>;
  onClose: () => void;
}) {
  if (!node) {
    return (
      <aside className="net-inspector">
        <p className="inspector-hint">Hover or click a node to inspect it.</p>
      </aside>
    );
  }

  const kindLabel: Record<GraphNode["kind"], string> = {
    agent: "agent",
    role: "role",
    run: "run",
    task: "task",
  };

  if (node.kind === "agent") {
    const meta = agentMeta(node);
    return (
      <aside className="net-inspector">
        <div className="inspector-header">
          <span className="inspector-kind">{kindLabel[node.kind]}</span>
          <button className="inspector-close" onClick={onClose} aria-label="Close inspector">
            ×
          </button>
        </div>
        <h3 className="inspector-title">{node.label}</h3>
        <div className="inspector-body">
          <div className="inspector-row">
            <span className="inspector-label">Command</span>
            <code>{meta.command}</code>
          </div>
          <div className="inspector-row">
            <span className="inspector-label">Availability</span>
            <span className={meta.available ? "net-ok" : "net-bad"}>
              {meta.available ? "available" : "missing"}
            </span>
          </div>
          <div className="inspector-row">
            <span className="inspector-label">Assigned roles</span>
            <span className="inspector-value">
              {meta.roles.length === 0 ? "–" : meta.roles.join(", ")}
            </span>
          </div>
        </div>
      </aside>
    );
  }

  if (node.kind === "role") {
    const meta = roleMeta(node);
    return (
      <aside className="net-inspector">
        <div className="inspector-header">
          <span className="inspector-kind">{kindLabel[node.kind]}</span>
          <button className="inspector-close" onClick={onClose} aria-label="Close inspector">
            ×
          </button>
        </div>
        <h3 className="inspector-title">{node.label}</h3>
        <div className="inspector-body">
          <div className="inspector-row">
            <span className="inspector-label">Bound agent</span>
            <code>{meta.agent}</code>
          </div>
        </div>
      </aside>
    );
  }

  if (node.kind === "run") {
    const meta = runMeta(node);
    const counts = meta.counts;
    const order: TaskState[] = ["pending", "ready", "running", "blocked", "failed", "completed"];
    return (
      <aside className="net-inspector">
        <div className="inspector-header">
          <span className="inspector-kind">{kindLabel[node.kind]}</span>
          <button className="inspector-close" onClick={onClose} aria-label="Close inspector">
            ×
          </button>
        </div>
        <h3 className="inspector-title">{node.label}</h3>
        <div className="inspector-body">
          <div className="inspector-row">
            <span className="inspector-label">Status</span>
            <span className="inspector-value">{meta.status}</span>
          </div>
          <div className="inspector-row">
            <span className="inspector-label">Objective</span>
            <span className="inspector-value inspector-objective">{meta.objective}</span>
          </div>
          <div className="inspector-row">
            <span className="inspector-label">Planner</span>
            <code>{meta.plannerAgent ?? "–"}</code>
          </div>
          <div className="inspector-row">
            <span className="inspector-label">Created</span>
            <span className="inspector-value">{new Date(meta.createdAt).toLocaleString()}</span>
          </div>
          <div className="inspector-row">
            <span className="inspector-label">Tasks</span>
            <span className="inspector-value">{counts.total}</span>
          </div>
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
        </div>
      </aside>
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
    <aside className="net-inspector">
      <div className="inspector-header">
        <span className="inspector-kind">{kindLabel[node.kind]}</span>
        <button className="inspector-close" onClick={onClose} aria-label="Close inspector">
          ×
        </button>
      </div>
      <h3 className="inspector-title">{node.label}</h3>
      <div className="inspector-body">
        <div className="inspector-row">
          <span className="inspector-label">State</span>
          <StateChip state={meta.state} />
        </div>
        <div className="inspector-row">
          <span className="inspector-label">Flow</span>
          <span
            className={STATE_META[meta.state].color === "#dc2626" ? "net-bad" : "inspector-value"}
          >
            {taskFlowNote(meta.state)}
          </span>
        </div>
        <div className="inspector-row">
          <span className="inspector-label">Objective</span>
          <span className="inspector-value inspector-objective">{meta.objective}</span>
        </div>
        <div className="inspector-row">
          <span className="inspector-label">Run</span>
          <code>#{meta.runId}</code>
        </div>
        <div className="inspector-row">
          <span className="inspector-label">Position</span>
          <span className="inspector-value">{meta.position}</span>
        </div>
        <div className="inspector-row">
          <span className="inspector-label">Depends on</span>
          {meta.dependencies.length === 0 ? (
            <span className="inspector-value">–</span>
          ) : (
            <span className="inspector-deps">
              {meta.dependencies.map((id) => (
                <DepRow key={id} id={`task:${id}`} nodesById={nodesById} />
              ))}
            </span>
          )}
        </div>
        <div className="inspector-row">
          <span className="inspector-label">Blocks</span>
          {dependents.length === 0 ? (
            <span className="inspector-value">–</span>
          ) : (
            <span className="inspector-deps">
              {dependents.map((dep) => (
                <DepRow key={dep.id} id={dep.id} nodesById={nodesById} />
              ))}
            </span>
          )}
        </div>
        {meta.worktreePath && (
          <div className="inspector-row">
            <span className="inspector-label">Worktree</span>
            <code className="inspector-objective">{meta.worktreePath}</code>
          </div>
        )}
      </div>
    </aside>
  );
}
