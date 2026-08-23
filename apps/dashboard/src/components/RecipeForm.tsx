import { useState } from "react";
import type { RecipeRoutingValue, RoleInfo, RolePolicyPreset } from "../types";
import { RECIPE_ROUTING_LABELS } from "../types";

const POLICY_PRESETS: { value: RolePolicyPreset; label: string }[] = [
  { value: "read_only", label: "Read-only" },
  { value: "implementation", label: "Implementation" },
  { value: "documentation", label: "Documentation" },
  { value: "review", label: "Review" },
  { value: "custom", label: "Custom" },
];

export interface RecipeFormValue {
  name: string;
  description: string;
  roles: string[];
  routing: RecipeRoutingValue;
  maxParallelTasks: number | null;
  policyPresets: Record<string, string>;
  plannerGuidance: string;
}

/**
 * The compact recipe editor: configuration fields only. There is deliberately
 * no task or DAG editing here — the Planner generates plans from objectives.
 */
export function RecipeForm({
  mode,
  roles,
  initial,
  error,
  submitLabel,
  onSubmit,
  onCancel,
}: {
  mode: "create" | "edit";
  roles: RoleInfo[];
  initial?: Partial<RecipeFormValue>;
  error: string | null;
  submitLabel: string;
  onSubmit: (value: RecipeFormValue) => void;
  onCancel: () => void;
}) {
  const [name, setName] = useState(initial?.name ?? "");
  const [description, setDescription] = useState(initial?.description ?? "");
  const [selectedRoles, setSelectedRoles] = useState<string[]>(initial?.roles ?? ["planner"]);
  const [routing, setRouting] = useState<RecipeRoutingValue>(initial?.routing ?? "project_default");
  const [parallelismText, setParallelismText] = useState(
    initial?.maxParallelTasks != null ? String(initial.maxParallelTasks) : ""
  );
  const [policyPresets, setPolicyPresets] = useState<Record<string, string>>(
    initial?.policyPresets ?? {}
  );
  const [guidance, setGuidance] = useState(initial?.plannerGuidance ?? "");
  const [validationError, setValidationError] = useState<string | null>(null);

  const plannerRequired = mode === "create" || selectedRoles.length > 0;

  const toggleRole = (roleId: string, checked: boolean) => {
    setSelectedRoles((current) =>
      checked ? [...current, roleId] : current.filter((candidate) => candidate !== roleId)
    );
  };

  const submit = (event: React.FormEvent) => {
    event.preventDefault();
    const trimmedName = name.trim();
    if (!trimmedName) {
      setValidationError("The recipe needs a name.");
      return;
    }
    if (selectedRoles.length === 0) {
      setValidationError("Select at least the planner role.");
      return;
    }
    if (!selectedRoles.includes("planner")) {
      setValidationError("The recipe must include the planner role.");
      return;
    }
    const parallelism = parallelismText.trim();
    let maxParallelTasks: number | null = null;
    if (parallelism !== "") {
      const parsed = Number.parseInt(parallelism, 10);
      if (!Number.isInteger(parsed) || parsed < 1 || parsed > 32) {
        setValidationError(
          "Parallelism must be between 1 and 32 (or empty for the project default)."
        );
        return;
      }
      maxParallelTasks = parsed;
    }
    setValidationError(null);
    onSubmit({
      name: trimmedName,
      description: description.trim(),
      roles: selectedRoles,
      routing,
      maxParallelTasks,
      policyPresets,
      plannerGuidance: guidance,
    });
  };

  const presetFor = (roleId: string) => policyPresets[roleId] ?? "";

  return (
    <form className="recipe-form" onSubmit={submit}>
      <label>
        <span>Name</span>
        <input
          value={name}
          onChange={(event) => setName(event.target.value)}
          autoFocus={mode === "create"}
          required
        />
      </label>
      <label>
        <span>Description</span>
        <input
          value={description}
          onChange={(event) => setDescription(event.target.value)}
          placeholder="What kind of work is this recipe for?"
        />
      </label>
      <fieldset className="workflow-team-group">
        <legend>Roles</legend>
        {roles.map((role) => (
          <label key={role.id} className="workflow-team-check">
            <input
              type="checkbox"
              checked={selectedRoles.includes(role.id)}
              disabled={role.id === "planner" && plannerRequired}
              onChange={(event) => toggleRole(role.id, event.target.checked)}
            />
            <span>
              {role.name}
              {!role.available && <small className="muted"> (no agent assigned)</small>}
            </span>
          </label>
        ))}
      </fieldset>
      <div className="settings-routing recipe-form-row">
        <label>
          <span>Routing</span>
          <select
            value={routing}
            onChange={(event) => setRouting(event.target.value as RecipeRoutingValue)}
          >
            {(Object.keys(RECIPE_ROUTING_LABELS) as RecipeRoutingValue[]).map((value) => (
              <option key={value} value={value}>
                {RECIPE_ROUTING_LABELS[value]}
              </option>
            ))}
          </select>
        </label>
        <label>
          <span>Parallelism (1–32)</span>
          <input
            type="number"
            min={1}
            max={32}
            value={parallelismText}
            onChange={(event) => setParallelismText(event.target.value)}
            placeholder="Project default"
          />
        </label>
      </div>
      <details className="agent-advanced">
        <summary>Policy presets</summary>
        <p className="inline-note">
          Optional per-role presets. They can only narrow what the Policy Engine already permits —
          never widen it.
        </p>
        {selectedRoles
          .filter((roleId) => roleId !== "planner")
          .map((roleId) => {
            const role = roles.find((candidate) => candidate.id === roleId);
            return (
              <label key={roleId}>
                <span>{role?.name ?? roleId}</span>
                <select
                  value={presetFor(roleId)}
                  onChange={(event) =>
                    setPolicyPresets((current) => {
                      const next = { ...current };
                      if (event.target.value === "") delete next[roleId];
                      else next[roleId] = event.target.value;
                      return next;
                    })
                  }
                >
                  <option value="">None (project policy)</option>
                  {POLICY_PRESETS.map((preset) => (
                    <option key={preset.value} value={preset.value}>
                      {preset.label}
                    </option>
                  ))}
                </select>
              </label>
            );
          })}
      </details>
      <label>
        <span>Planner guidance</span>
        <textarea
          rows={4}
          value={guidance}
          onChange={(event) => setGuidance(event.target.value)}
          placeholder="Bounded planning context, e.g. prefer a compact diagnosis/fix/regression-test workflow."
        />
      </label>
      {(validationError ?? error) && <p className="inline-error">{validationError ?? error}</p>}
      <div className="settings-actions">
        <button className="button" type="submit">
          {submitLabel}
        </button>
        <button className="button" type="button" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
