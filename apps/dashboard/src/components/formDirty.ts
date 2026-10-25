import type { ExecutionClass, RecipeRoutingValue, RolePolicyPreset } from "../types";
import type { AgentDraft } from "./agentFormState";

export interface RoleFormDraft {
  id: string;
  name: string;
  description: string;
  executionClass: ExecutionClass;
  instructions: string;
  selectedAgents: string[];
  preferredAgent: string | null;
  policyPreset: RolePolicyPreset | null;
}

export interface RoleFormInitial {
  id?: string;
  name?: string;
  description?: string;
  executionClass?: ExecutionClass;
  instructions?: string;
  policyPreset?: RolePolicyPreset | null;
}

export interface RoleFormValue {
  id?: string;
  name: string;
  description: string;
  executionClass: ExecutionClass;
  instructions: string;
  agents: string[];
  preferredAgent: string | null;
  policyPreset: RolePolicyPreset | null;
}

export interface RecipeFormDraft {
  name: string;
  description: string;
  roles: string[];
  routing: RecipeRoutingValue;
  parallelismText: string;
  policyPresets: Record<string, string>;
  plannerGuidance: string;
  toolsText: string;
}

export interface RecipeFormInitial {
  name?: string;
  description?: string;
  roles?: string[];
  routing?: RecipeRoutingValue;
  maxParallelTasks?: number | null;
  policyPresets?: Record<string, string>;
  plannerGuidance?: string;
  tools?: string[];
}

export interface RecipeFormValue {
  name: string;
  description: string;
  roles: string[];
  routing: RecipeRoutingValue;
  maxParallelTasks: number | null;
  policyPresets: Record<string, string>;
  plannerGuidance: string;
  tools: string[];
}

export function deepEqual(left: unknown, right: unknown): boolean {
  if (Object.is(left, right)) return true;
  if (typeof left !== "object" || left === null) return false;
  if (typeof right !== "object" || right === null) return false;
  if (Array.isArray(left) || Array.isArray(right)) {
    if (!Array.isArray(left) || !Array.isArray(right) || left.length !== right.length) return false;
    return left.every((value, index) => deepEqual(value, right[index]));
  }
  const leftRecord = left as Record<string, unknown>;
  const rightRecord = right as Record<string, unknown>;
  const leftKeys = Object.keys(leftRecord);
  const rightKeys = Object.keys(rightRecord);
  if (leftKeys.length !== rightKeys.length) return false;
  return leftKeys.every(
    (key) =>
      Object.prototype.hasOwnProperty.call(rightRecord, key) &&
      deepEqual(leftRecord[key], rightRecord[key])
  );
}

export function isAgentDraftDirty(draft: AgentDraft, baseline: AgentDraft): boolean {
  return !deepEqual(draft, baseline);
}

export function isRoleDraftDirty(draft: RoleFormDraft, baseline: RoleFormDraft): boolean {
  return !deepEqual(draft, baseline);
}

export function isRecipeDraftDirty(draft: RecipeFormDraft, baseline: RecipeFormDraft): boolean {
  return !deepEqual(draft, baseline);
}

export function roleDraftFromInitial(initial?: RoleFormInitial): RoleFormDraft {
  return {
    id: initial?.id ?? "",
    name: initial?.name ?? "",
    description: initial?.description ?? "",
    executionClass: initial?.executionClass ?? "execution",
    instructions: initial?.instructions ?? "",
    selectedAgents: [],
    preferredAgent: null,
    policyPreset: initial?.policyPreset ?? null,
  };
}

export function recipeDraftFromInitial(initial?: RecipeFormInitial): RecipeFormDraft {
  return {
    name: initial?.name ?? "",
    description: initial?.description ?? "",
    roles: [...(initial?.roles ?? ["planner"])],
    routing: initial?.routing ?? "project_default",
    parallelismText: initial?.maxParallelTasks != null ? String(initial.maxParallelTasks) : "",
    policyPresets: { ...(initial?.policyPresets ?? {}) },
    plannerGuidance: initial?.plannerGuidance ?? "",
    toolsText: (initial?.tools ?? []).join("\n"),
  };
}

export function roleValueFromDraft(
  draft: RoleFormDraft,
  mode: "create" | "edit",
  slug: string
): RoleFormValue {
  return {
    id: mode === "create" ? draft.id.trim() || slug || undefined : undefined,
    name: draft.name.trim(),
    description: draft.description.trim(),
    executionClass: draft.executionClass,
    instructions: draft.instructions.trim(),
    agents: mode === "create" ? draft.selectedAgents : [],
    preferredAgent: mode === "create" ? draft.preferredAgent : null,
    policyPreset: draft.policyPreset,
  };
}

export function recipeValueFromDraft(
  draft: RecipeFormDraft,
  maxParallelTasks: number | null
): RecipeFormValue {
  return {
    name: draft.name.trim(),
    description: draft.description.trim(),
    roles: draft.roles,
    routing: draft.routing,
    maxParallelTasks,
    policyPresets: draft.policyPresets,
    plannerGuidance: draft.plannerGuidance,
    tools: draft.toolsText
      .split("\n")
      .map((value) => value.trim())
      .filter(Boolean),
  };
}
