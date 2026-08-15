import { useMemo, useState } from "react";
import {
  addRoleAssignment,
  createRole,
  removeRoleAssignment,
  setPreferredAssignment,
  updateRole,
} from "../api";
import type { RoleInfo } from "../types";
import { RoleForm } from "./RoleForm";

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="inspector-row">
      <span className="inspector-label">{label}</span>
      <span className="inspector-value">{children}</span>
    </div>
  );
}

export function RoleInspector({
  role,
  agents,
  onClose,
  onChanged,
  onDelete,
}: {
  role: RoleInfo;
  agents: string[];
  onClose: () => void;
  onChanged: () => void;
  onDelete: () => void;
}) {
  const [addAgent, setAddAgent] = useState("");
  const [editing, setEditing] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const unassignedAgents = useMemo(
    () => agents.filter((agent) => !role.assignments.some((entry) => entry.agent === agent)),
    [agents, role.assignments]
  );

  const act = async (operation: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await operation();
      await onChanged();
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(false);
    }
  };

  const saveDefinition = async (value: {
    name: string;
    description: string;
    executionClass: RoleInfo["executionClass"];
    instructions: string;
  }) => {
    setBusy(true);
    setError(null);
    try {
      await updateRole(role.id, {
        name: value.name,
        description: value.description,
        executionClass: value.executionClass,
        instructions: value.instructions,
      });
      setEditing(false);
      await onChanged();
    } catch (reason) {
      setError((reason as Error).message);
    } finally {
      setBusy(false);
    }
  };

  if (editing) {
    return (
      <aside className="net-inspector role-inspector" aria-label={`Edit ${role.name} role`}>
        <div className="inspector-header">
          <span className="inspector-kind">Custom role</span>
          <button className="inspector-close" onClick={onClose} aria-label="Close role inspector">
            x
          </button>
        </div>
        <h3 className="inspector-title">Edit {role.name}</h3>
        <div className="inspector-body">
          <RoleForm
            mode="edit"
            agents={agents}
            initial={role}
            error={error}
            submitLabel="Save role"
            onSubmit={(value) => void saveDefinition(value)}
            onCancel={() => setEditing(false)}
          />
        </div>
      </aside>
    );
  }

  return (
    <aside className="net-inspector role-inspector" aria-label={`${role.name} role`}>
      <div className="inspector-header">
        <span className="inspector-kind">{role.kind === "core" ? "Core role" : "Custom role"}</span>
        <button className="inspector-close" onClick={onClose} aria-label="Close role inspector">
          x
        </button>
      </div>
      <h3 className="inspector-title">{role.name}</h3>
      <div className="inspector-body">
        <Row label="Role id">
          <code>{role.id}</code>
        </Row>
        <Row label="Execution class">{role.executionClass.replaceAll("_", " ")}</Row>
        <Row label="Status">{role.available ? "Available" : "No agent assigned"}</Row>
        {role.description && <p className="role-description">{role.description}</p>}
        {role.instructions && (
          <details className="role-instructions" open>
            <summary>Instructions</summary>
            <pre>{role.instructions}</pre>
          </details>
        )}

        <div className="role-agents">
          <span className="inspector-label">Agents</span>
          {role.assignments.length === 0 ? (
            <p className="inspector-hint">No agent is assigned to this role yet.</p>
          ) : (
            <ul className="role-agent-list">
              {role.assignments.map((assignment) => (
                <li key={assignment.agent} className="role-agent-row">
                  <code>{assignment.agent}</code>
                  {assignment.preferred ? (
                    <span className="role-preferred">Preferred</span>
                  ) : (
                    <button
                      className="button role-agent-action"
                      disabled={busy}
                      onClick={() =>
                        void act(() => setPreferredAssignment(role.id, assignment.agent))
                      }
                    >
                      Set preferred
                    </button>
                  )}
                  <button
                    className="button role-agent-remove"
                    disabled={busy}
                    aria-label={`Remove ${assignment.agent} from ${role.name}`}
                    onClick={() => void act(() => removeRoleAssignment(role.id, assignment.agent))}
                  >
                    ×
                  </button>
                </li>
              ))}
            </ul>
          )}
          {unassignedAgents.length > 0 && (
            <div className="role-add-agent">
              <select
                className="net-select"
                aria-label="Add agent to role"
                value={addAgent}
                onChange={(event) => setAddAgent(event.target.value)}
              >
                <option value="">Add agent</option>
                {unassignedAgents.map((agent) => (
                  <option key={agent} value={agent}>
                    {agent}
                  </option>
                ))}
              </select>
              <button
                className="button"
                disabled={!addAgent || busy}
                onClick={() => {
                  if (!addAgent) return;
                  const agent = addAgent;
                  setAddAgent("");
                  void act(() => addRoleAssignment(role.id, agent));
                }}
              >
                Assign
              </button>
            </div>
          )}
        </div>

        {role.kind === "custom" && (
          <div className="role-definition-actions">
            <button className="button" onClick={() => setEditing(true)}>
              Edit
            </button>
            <button
              className="button"
              disabled={busy}
              onClick={() =>
                void act(() =>
                  createRole({
                    name: `${role.name} (copy)`,
                    description: role.description,
                    executionClass: role.executionClass,
                    instructions: role.instructions,
                    agents: role.assignments.map((assignment) => assignment.agent),
                    preferredAgent:
                      role.assignments.find((assignment) => assignment.preferred)?.agent ??
                      undefined,
                  })
                )
              }
            >
              Duplicate
            </button>
            <button className="button inspector-delete" onClick={onDelete}>
              Delete
            </button>
          </div>
        )}
        {role.kind === "core" && (
          <p className="inline-note">
            Core role definitions are built in. Manage assignments instead of editing the role.
          </p>
        )}
        {error && (
          <p className="inline-error" role="alert">
            {error}
          </p>
        )}
      </div>
    </aside>
  );
}
