import { useRef, useState } from "react";
import type { RoleInfo, RolePolicyPreset } from "../types";
import { RECIPE_ROUTING_LABELS } from "../types";
import { UNSAVED_RECIPE_MESSAGE, useUnsavedChanges } from "../unsavedChanges";
import {
  isRecipeDraftDirty,
  recipeDraftFromInitial,
  recipeValueFromDraft,
  type RecipeFormDraft,
  type RecipeFormInitial,
  type RecipeFormValue,
} from "./formDirty";

export type { RecipeFormInitial, RecipeFormValue } from "./formDirty";

const POLICY_PRESETS: { value: RolePolicyPreset; label: string }[] = [
  { value: "read_only", label: "Read-only" },
  { value: "implementation", label: "Implementation" },
  { value: "documentation", label: "Documentation" },
  { value: "review", label: "Review" },
  { value: "custom", label: "Custom" },
];

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
  onDirtyChange,
}: {
  mode: "create" | "edit";
  roles: RoleInfo[];
  initial?: RecipeFormInitial;
  error: string | null;
  submitLabel: string;
  onSubmit: (value: RecipeFormValue) => Promise<boolean> | boolean;
  onCancel: () => void;
  onDirtyChange?: (dirty: boolean) => void;
}) {
  const [draft, setDraft] = useState<RecipeFormDraft>(() => recipeDraftFromInitial(initial));
  const [baseline, setBaseline] = useState<RecipeFormDraft>(() => recipeDraftFromInitial(initial));
  const [validationError, setValidationError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const savingRef = useRef(false);
  const dirty = isRecipeDraftDirty(draft, baseline);
  const { requestClose } = useUnsavedChanges(dirty, {
    message: UNSAVED_RECIPE_MESSAGE,
    onDirtyChange,
  });

  const plannerRequired = mode === "create" || draft.roles.length > 0;

  const toggleRole = (roleId: string, checked: boolean) => {
    setDraft((current) => ({
      ...current,
      roles: checked
        ? [...current.roles, roleId]
        : current.roles.filter((candidate) => candidate !== roleId),
    }));
  };

  const submit = async (event: React.FormEvent) => {
    event.preventDefault();
    if (savingRef.current) return;
    const trimmedName = draft.name.trim();
    if (!trimmedName) {
      setValidationError("The recipe needs a name.");
      return;
    }
    if (draft.roles.length === 0) {
      setValidationError("Select at least the planner role.");
      return;
    }
    if (!draft.roles.includes("planner")) {
      setValidationError("The recipe must include the planner role.");
      return;
    }
    const parallelism = draft.parallelismText.trim();
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
    const submittedDraft = draft;
    setValidationError(null);
    savingRef.current = true;
    setSaving(true);
    try {
      const saved = await onSubmit(recipeValueFromDraft(draft, maxParallelTasks));
      if (saved !== false) setBaseline(submittedDraft);
    } catch (reason) {
      setValidationError((reason as Error).message);
    } finally {
      savingRef.current = false;
      setSaving(false);
    }
  };

  const presetFor = (roleId: string) => draft.policyPresets[roleId] ?? "";

  return (
    <form
      className="recipe-form"
      onSubmit={submit}
      onKeyDown={(event) => {
        if (event.key !== "Escape" || savingRef.current) return;
        event.preventDefault();
        event.stopPropagation();
        requestClose(onCancel);
      }}
    >
      <label>
        <span>Name</span>
        <input
          value={draft.name}
          onChange={(event) => setDraft((current) => ({ ...current, name: event.target.value }))}
          autoFocus={mode === "create"}
          required
        />
      </label>
      <label>
        <span>Description</span>
        <input
          value={draft.description}
          onChange={(event) =>
            setDraft((current) => ({ ...current, description: event.target.value }))
          }
          placeholder="What kind of work is this recipe for?"
        />
      </label>
      <fieldset className="workflow-team-group">
        <legend>Roles</legend>
        {roles.map((role) => (
          <label key={role.id} className="workflow-team-check">
            <input
              type="checkbox"
              checked={draft.roles.includes(role.id)}
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
            value={draft.routing}
            onChange={(event) =>
              setDraft((current) => ({
                ...current,
                routing: event.target.value as RecipeFormDraft["routing"],
              }))
            }
          >
            {(Object.keys(RECIPE_ROUTING_LABELS) as RecipeFormDraft["routing"][]).map((value) => (
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
            value={draft.parallelismText}
            onChange={(event) =>
              setDraft((current) => ({ ...current, parallelismText: event.target.value }))
            }
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
        {draft.roles
          .filter((roleId) => roleId !== "planner")
          .map((roleId) => {
            const role = roles.find((candidate) => candidate.id === roleId);
            return (
              <label key={roleId}>
                <span>{role?.name ?? roleId}</span>
                <select
                  value={presetFor(roleId)}
                  onChange={(event) =>
                    setDraft((current) => {
                      const policyPresets = { ...current.policyPresets };
                      if (event.target.value === "") delete policyPresets[roleId];
                      else policyPresets[roleId] = event.target.value;
                      return { ...current, policyPresets };
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
          value={draft.plannerGuidance}
          onChange={(event) =>
            setDraft((current) => ({ ...current, plannerGuidance: event.target.value }))
          }
          placeholder="Bounded planning context, e.g. prefer a compact diagnosis/fix/regression-test workflow."
        />
      </label>
      <label>
        <span>External tool selection</span>
        <textarea
          rows={3}
          value={draft.toolsText}
          onChange={(event) =>
            setDraft((current) => ({ ...current, toolsText: event.target.value }))
          }
          placeholder={"docs.*\ngithub.read_*"}
        />
        <small className="inline-note">
          One policy pattern per line. Recipes can only select or narrow access already granted to
          the role; they can never elevate it.
        </small>
      </label>
      {(validationError ?? error) && <p className="inline-error">{validationError ?? error}</p>}
      <div className="settings-actions">
        <button className="button" type="submit" disabled={saving}>
          {saving ? "Saving…" : submitLabel}
        </button>
        <button
          className="button"
          type="button"
          onClick={() => requestClose(onCancel)}
          disabled={saving}
        >
          Cancel
        </button>
      </div>
    </form>
  );
}
