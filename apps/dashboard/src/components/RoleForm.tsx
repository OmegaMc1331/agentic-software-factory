import { useId, useRef, useState } from "react";
import type { ExecutionClass, RolePolicyPreset } from "../types";
import { UNSAVED_ROLE_MESSAGE, useUnsavedChanges } from "../unsavedChanges";
import {
  isRoleDraftDirty,
  roleDraftFromInitial,
  roleValueFromDraft,
  type RoleFormDraft,
  type RoleFormInitial,
  type RoleFormValue,
} from "./formDirty";

export type { RoleFormInitial, RoleFormValue } from "./formDirty";

const POLICY_PRESETS: { value: RolePolicyPreset | ""; label: string }[] = [
  { value: "implementation", label: "Implementation — task-worktree write" },
  { value: "read_only", label: "Read-only" },
  { value: "review", label: "Review — read-only" },
  { value: "documentation", label: "Documentation — README/docs write" },
  { value: "custom", label: "Custom — defined in config.toml" },
];

function roleSlug(name: string): string {
  return (
    name
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "_")
      .replace(/^_+|_+$/g, "")
      .slice(0, 64) || ""
  );
}

const EXECUTION_CLASSES: { value: ExecutionClass; label: string }[] = [
  { value: "planning", label: "Planning" },
  { value: "execution", label: "Execution" },
  { value: "review", label: "Review" },
  { value: "advisory", label: "Advisory" },
  { value: "post_process", label: "Post-process" },
];

const ROLE_TEMPLATES: {
  id: string;
  label: string;
  executionClass: ExecutionClass;
  description: string;
  instructions: string;
}[] = [
  {
    id: "implementation",
    label: "Implementation",
    executionClass: "execution",
    description: "Implements a planned task in an isolated worktree.",
    instructions:
      "Purpose: implement exactly one task and verify it locally.\nResponsibilities:\n- make the code changes the task requires;\n- keep edits scoped to the task;\n- run focused local validation where practical;\n- report evidence of what changed and what was run.\nBoundaries:\n- do not restructure unrelated code;\n- do not weaken or skip tests to make them pass.\nExpected output: implementation plus a concise JSON report with `summary` and `commands`.",
  },
  {
    id: "review",
    label: "Review",
    executionClass: "review",
    description: "Independently evaluates task output against acceptance criteria.",
    instructions:
      "Purpose: decide whether the implementation satisfies the task.\nResponsibilities:\n- check the diff and evidence against every acceptance criterion;\n- verify claimed commands and outcomes are plausible;\n- approve, or request changes with concrete, actionable feedback.\nBoundaries:\n- do not modify files;\n- do not approve on trust without evidence.\nExpected output: one JSON object with `decision`, `reason` and `feedback`.",
  },
  {
    id: "research",
    label: "Research",
    executionClass: "advisory",
    description: "Gathers technical context another role needs.",
    instructions:
      "Purpose: collect and summarize repository or dependency context.\nResponsibilities:\n- inspect the repository and existing documentation;\n- investigate relevant dependency or API behavior;\n- identify constraints, unknowns and prior art;\n- return concise, evidence-backed findings.\nBoundaries:\n- do not modify production code unless the task assigns implementation.\nExpected output: findings with file references, written where the task requires them.",
  },
  {
    id: "architecture",
    label: "Architecture",
    executionClass: "advisory",
    description: "Analyzes architecture and technical boundaries around a change.",
    instructions:
      "Purpose: resolve structural questions before or during implementation.\nResponsibilities:\n- identify component boundaries and interfaces affected by the task;\n- describe data flow and technical constraints;\n- propose a migration or rollout strategy when behavior changes;\n- write the decision down where the task requires it.\nBoundaries:\n- do not silently restructure unrelated subsystems.\nExpected output: implementation or a design note the task can build on.",
  },
  {
    id: "testing",
    label: "Testing",
    executionClass: "execution",
    description: "Designs and runs verification for task requirements.",
    instructions:
      "Purpose: make the task's behavior verifiable.\nResponsibilities:\n- add or extend tests covering the acceptance criteria;\n- cover regressions and relevant edge cases;\n- run the affected test suites and report results.\nBoundaries:\n- do not lower coverage bars or mark tests as ignored;\n- do not change production behavior unless the task requires it.\nExpected output: tests, passing runs, and verification evidence.",
  },
  {
    id: "documentation",
    label: "Documentation",
    executionClass: "post_process",
    description: "Produces or updates documentation after implementation.",
    instructions:
      "Purpose: keep documentation consistent with the change.\nResponsibilities:\n- update README and usage docs affected by the change;\n- refresh architecture or design notes where behavior moved;\n- add migration notes for behavior or interface changes.\nBoundaries:\n- do not document behavior that does not exist;\n- keep instructions runnable and copy-pasteable.\nExpected output: documentation changes with verification evidence.",
  },
  { id: "blank", label: "Blank", executionClass: "execution", description: "", instructions: "" },
];

export function RoleForm({
  mode,
  agents,
  initial,
  error,
  submitLabel,
  onSubmit,
  onCancel,
  onDirtyChange,
}: {
  mode: "create" | "edit";
  agents: string[];
  initial?: RoleFormInitial;
  error?: string | null;
  submitLabel: string;
  onSubmit: (value: RoleFormValue) => Promise<boolean> | boolean;
  onCancel: () => void;
  onDirtyChange?: (dirty: boolean) => void;
}) {
  const preferredRadioName = useId();
  const [templateId, setTemplateId] = useState("");
  const [draft, setDraft] = useState<RoleFormDraft>(() => roleDraftFromInitial(initial));
  const [baseline, setBaseline] = useState<RoleFormDraft>(() => roleDraftFromInitial(initial));
  const [roleIdEdited, setRoleIdEdited] = useState(initial?.id !== undefined);
  const [validationError, setValidationError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const dirty = isRoleDraftDirty(draft, baseline);
  const { requestClose } = useUnsavedChanges(dirty, {
    message: UNSAVED_ROLE_MESSAGE,
    onDirtyChange,
  });

  const applyTemplate = (nextTemplateId: string) => {
    setTemplateId(nextTemplateId);
    const template = ROLE_TEMPLATES.find((candidate) => candidate.id === nextTemplateId);
    if (!template) return;
    setDraft((current) => ({
      ...current,
      description: template.description,
      executionClass: template.executionClass,
      instructions: template.instructions,
    }));
    setValidationError(null);
  };

  const toggleAgent = (agent: string, checked: boolean) => {
    setDraft((current) => {
      const selectedAgents = checked
        ? [...current.selectedAgents, agent]
        : current.selectedAgents.filter((candidate) => candidate !== agent);
      let preferredAgent = current.preferredAgent;
      if (!checked && preferredAgent === agent) preferredAgent = null;
      if (checked && preferredAgent === null) preferredAgent = agent;
      return { ...current, selectedAgents, preferredAgent };
    });
  };

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (savingRef.current) return;
    const trimmedName = draft.name.trim();
    if (!trimmedName) {
      setValidationError("A role name is required.");
      return;
    }
    if (!draft.description.trim()) {
      setValidationError("Describe what this role does.");
      return;
    }
    const submittedDraft = draft;
    setValidationError(null);
    savingRef.current = true;
    setSaving(true);
    try {
      const saved = await onSubmit(roleValueFromDraft(draft, mode, roleSlug(trimmedName)));
      if (saved !== false) setBaseline(submittedDraft);
    } catch (reason) {
      setValidationError((reason as Error).message);
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  };

  return (
    <form
      className="add-node-form role-form"
      onSubmit={submit}
      onKeyDown={(event) => {
        if (event.key !== "Escape" || savingRef.current) return;
        event.preventDefault();
        event.stopPropagation();
        requestClose(onCancel);
      }}
    >
      {mode === "create" && (
        <label>
          <span>Template</span>
          <select value={templateId} onChange={(event) => applyTemplate(event.target.value)}>
            <option value="">Start from scratch</option>
            {ROLE_TEMPLATES.map((template) => (
              <option key={template.id} value={template.id}>
                {template.label}
              </option>
            ))}
          </select>
        </label>
      )}
      <label>
        <span>Name</span>
        <input
          value={draft.name}
          onChange={(event) => {
            const name = event.target.value;
            setDraft((current) => ({
              ...current,
              name,
              ...(roleIdEdited ? {} : { id: roleSlug(name) }),
            }));
            setValidationError(null);
          }}
          required
          autoFocus
        />
      </label>
      {mode === "create" && (
        <div className="role-form-id">
          <label>
            <span>Role id</span>
            <input
              value={draft.id}
              onChange={(event) => {
                const id = event.target.value;
                setDraft((current) => ({ ...current, id }));
                setRoleIdEdited(true);
              }}
              placeholder={roleSlug(draft.name) || "derived-from-name"}
              aria-describedby="role-id-preview"
            />
          </label>
          <small className="role-form-hint" id="role-id-preview">
            {roleSlug(draft.name) || "derived from the name"}
          </small>
        </div>
      )}
      <label>
        <span>Description</span>
        <textarea
          rows={2}
          value={draft.description}
          onChange={(event) =>
            setDraft((current) => ({ ...current, description: event.target.value }))
          }
          required
        />
      </label>
      <label>
        <span>Execution class</span>
        <select
          value={draft.executionClass}
          onChange={(event) =>
            setDraft((current) => ({
              ...current,
              executionClass: event.target.value as ExecutionClass,
            }))
          }
        >
          {EXECUTION_CLASSES.map((option) => (
            <option key={option.value} value={option.value}>
              {option.label}
            </option>
          ))}
        </select>
      </label>
      <label>
        <span>Instructions</span>
        <textarea
          rows={8}
          value={draft.instructions}
          onChange={(event) =>
            setDraft((current) => ({ ...current, instructions: event.target.value }))
          }
          placeholder="Purpose, responsibilities, boundaries and expected output"
        />
      </label>
      <label>
        <span>Policy preset</span>
        <select
          value={draft.policyPreset ?? ""}
          onChange={(event) =>
            setDraft((current) => ({
              ...current,
              policyPreset:
                event.target.value === "" ? null : (event.target.value as RolePolicyPreset),
            }))
          }
          aria-describedby="role-policy-hint"
        >
          <option value="">Default (permissive)</option>
          {POLICY_PRESETS.map((preset) => (
            <option key={preset.value} value={preset.value}>
              {preset.label}
            </option>
          ))}
        </select>
        <small className="role-form-hint" id="role-policy-hint">
          The policy is what Factory enforces; the instructions above only guide the agent.
        </small>
      </label>
      {mode === "create" && (
        <fieldset className="role-form-agents">
          <legend>Assigned agents</legend>
          {agents.length === 0 ? (
            <p className="inline-note">Configure an agent first to assign it to this role.</p>
          ) : (
            agents.map((agent) => (
              <div key={agent} className="role-form-agent">
                <label className="role-form-agent-check">
                  <input
                    type="checkbox"
                    checked={draft.selectedAgents.includes(agent)}
                    onChange={(event) => toggleAgent(agent, event.target.checked)}
                  />
                  <span>{agent}</span>
                </label>
                {draft.selectedAgents.includes(agent) && (
                  <label className="role-form-agent-preferred">
                    <input
                      type="radio"
                      name={preferredRadioName}
                      checked={draft.preferredAgent === agent}
                      onChange={() =>
                        setDraft((current) => ({ ...current, preferredAgent: agent }))
                      }
                    />
                    <span>Preferred</span>
                  </label>
                )}
              </div>
            ))
          )}
        </fieldset>
      )}
      {(validationError ?? error) && <p className="inline-error">{validationError ?? error}</p>}
      <div className="add-node-actions">
        <button className="button" type="submit" disabled={saving}>
          {saving ? "Saving…" : submitLabel}
        </button>
        <button
          className="button"
          type="button"
          onClick={() => requestClose(onCancel)}
          disabled={saving}
        >
          {mode === "create" ? "Back" : "Cancel"}
        </button>
      </div>
    </form>
  );
}
