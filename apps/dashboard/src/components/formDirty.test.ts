import { describe, expect, it } from "vitest";
import { agentDraftFrom, emptyAgentDraft } from "./agentFormState";
import {
  deepEqual,
  isAgentDraftDirty,
  isRecipeDraftDirty,
  isRoleDraftDirty,
  recipeDraftFromInitial,
  recipeValueFromDraft,
  roleDraftFromInitial,
  roleValueFromDraft,
} from "./formDirty";

describe("semantic equality", () => {
  it("compares primitives and object identity", () => {
    expect(deepEqual(1, 1)).toBe(true);
    expect(deepEqual("value", "value")).toBe(true);
    expect(deepEqual(null, null)).toBe(true);
    expect(deepEqual(1, 2)).toBe(false);
    expect(deepEqual(null, {})).toBe(false);
  });

  it("compares nested objects and arrays by value", () => {
    expect(
      deepEqual(
        { name: "agent", args: ["run", { flag: true }], env: { A: "1" } },
        { env: { A: "1" }, args: ["run", { flag: true }], name: "agent" }
      )
    ).toBe(true);
  });

  it("detects differing key sets", () => {
    expect(deepEqual({ a: 1 }, { a: 1, b: 2 })).toBe(false);
    expect(deepEqual({ a: 1, b: 2 }, { a: 1 })).toBe(false);
  });

  it("detects array order and length changes", () => {
    expect(deepEqual(["a", "b"], ["b", "a"])).toBe(false);
    expect(deepEqual(["a"], ["a", "b"])).toBe(false);
  });
});

describe("agent draft dirtiness", () => {
  it("treats a draft copied from configuration as clean", () => {
    const entry = {
      command: "codex",
      args: ["exec"],
      env: { API_KEY: "secret" },
      capabilities: ["coding"],
    };
    expect(isAgentDraftDirty(agentDraftFrom("codex", entry), agentDraftFrom("codex", entry))).toBe(
      false
    );
  });

  it("detects a changed name", () => {
    const baseline = emptyAgentDraft();
    expect(isAgentDraftDirty({ ...baseline, name: "renamed" }, baseline)).toBe(true);
  });

  it("detects reordered arguments", () => {
    const baseline = { ...emptyAgentDraft(), args: ["run", "--flag"] };
    expect(isAgentDraftDirty({ ...baseline, args: ["--flag", "run"] }, baseline)).toBe(true);
  });

  it("treats malformed environment text as unsaved work", () => {
    const baseline = { ...emptyAgentDraft(), envText: "VALID=1" };
    expect(isAgentDraftDirty({ ...baseline, envText: "INVALID" }, baseline)).toBe(true);
  });

  it("becomes clean when the environment text is restored", () => {
    const baseline = { ...emptyAgentDraft(), envText: "VALID=1" };
    expect(isAgentDraftDirty({ ...baseline, envText: "VALID=1" }, baseline)).toBe(false);
  });
});

describe("role draft dirtiness", () => {
  it("builds the expected default draft", () => {
    expect(roleDraftFromInitial()).toEqual({
      id: "",
      name: "",
      description: "",
      executionClass: "execution",
      instructions: "",
      selectedAgents: [],
      preferredAgent: null,
      policyPreset: null,
    });
  });

  it("treats a draft copied from an existing role as clean", () => {
    const initial = {
      id: "worker",
      name: "Worker",
      description: "Implements tasks",
      executionClass: "execution" as const,
      instructions: "Do the work",
      policyPreset: "implementation" as const,
    };
    expect(isRoleDraftDirty(roleDraftFromInitial(initial), roleDraftFromInitial(initial))).toBe(
      false
    );
  });

  it("detects a changed role name", () => {
    const baseline = roleDraftFromInitial({ name: "Worker" });
    expect(isRoleDraftDirty({ ...baseline, name: "Reviewer" }, baseline)).toBe(true);
  });

  it("detects selected-agent changes", () => {
    const baseline = roleDraftFromInitial();
    expect(isRoleDraftDirty({ ...baseline, selectedAgents: ["codex"] }, baseline)).toBe(true);
  });

  it("detects preferred-agent changes", () => {
    const baseline = roleDraftFromInitial();
    expect(isRoleDraftDirty({ ...baseline, preferredAgent: "codex" }, baseline)).toBe(true);
  });

  it("trims create values and derives the id from the slug", () => {
    const draft = {
      ...roleDraftFromInitial({ name: "  Code Reviewer  ", description: " Reviews code " }),
      id: " ",
    };
    expect(roleValueFromDraft(draft, "create", "code_reviewer")).toEqual({
      id: "code_reviewer",
      name: "Code Reviewer",
      description: "Reviews code",
      executionClass: "execution",
      instructions: "",
      agents: [],
      preferredAgent: null,
      policyPreset: null,
    });
  });

  it("omits create-only fields when editing", () => {
    const draft = {
      ...roleDraftFromInitial({ id: "worker", name: "Worker" }),
      selectedAgents: ["codex"],
      preferredAgent: "codex",
    };
    expect(roleValueFromDraft(draft, "edit", "worker")).toMatchObject({
      id: undefined,
      agents: [],
      preferredAgent: null,
    });
  });
});

describe("recipe draft dirtiness", () => {
  it("starts a new recipe with the planner role", () => {
    expect(recipeDraftFromInitial()).toMatchObject({
      roles: ["planner"],
      routing: "project_default",
      parallelismText: "",
      toolsText: "",
    });
  });

  it("treats a duplicated recipe as clean", () => {
    const initial = {
      name: "Diagnose (copy)",
      roles: ["planner", "worker"],
      maxParallelTasks: 3,
      tools: ["docs.*"],
    };
    expect(
      isRecipeDraftDirty(recipeDraftFromInitial(initial), recipeDraftFromInitial(initial))
    ).toBe(false);
  });

  it("detects edited tool patterns", () => {
    const baseline = recipeDraftFromInitial({ tools: ["docs.*"] });
    expect(isRecipeDraftDirty({ ...baseline, toolsText: "docs.*\ngithub.read_*" }, baseline)).toBe(
      true
    );
  });

  it("parses tools and preserves the validated parallelism", () => {
    const draft = recipeDraftFromInitial({
      name: "  Diagnose  ",
      description: "  Find the cause ",
      tools: [" docs.* ", "github.read_*", "  "],
    });
    expect(recipeValueFromDraft(draft, 4)).toMatchObject({
      name: "Diagnose",
      description: "Find the cause",
      maxParallelTasks: 4,
      tools: ["docs.*", "github.read_*"],
    });
  });
});
