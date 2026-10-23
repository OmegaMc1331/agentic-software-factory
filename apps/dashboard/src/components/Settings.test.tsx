import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  addRoleAssignment,
  createMcpServer,
  createRecipe,
  deleteRecipe,
  deleteRole,
  deleteMcpServer,
  fetchAgents,
  fetchConfig,
  fetchMcpServers,
  fetchMcpTools,
  fetchRecipes,
  fetchRoles,
  mcpDiscoveryRequestTimeoutMs,
  removeRoleAssignment,
  saveConfig,
  setPreferredAssignment,
  testMcpServer,
  updateMcpServer,
  updateRecipe,
  updateRole,
} from "../api";
import type { ConfigData, McpServerView, McpTool, Recipe, RoleInfo } from "../types";
import { SettingsView } from "./Settings";

vi.mock("../api", () => ({
  addRoleAssignment: vi.fn(),
  deleteRole: vi.fn(),
  fetchAgents: vi.fn(),
  fetchConfig: vi.fn(),
  fetchRoles: vi.fn(),
  fetchRecipes: vi.fn(),
  fetchMcpServers: vi.fn(),
  createMcpServer: vi.fn(),
  updateMcpServer: vi.fn(),
  deleteMcpServer: vi.fn(),
  testMcpServer: vi.fn(),
  fetchMcpTools: vi.fn(),
  mcpDiscoveryRequestTimeoutMs: vi.fn(),
  createRecipe: vi.fn(),
  updateRecipe: vi.fn(),
  deleteRecipe: vi.fn(),
  removeRoleAssignment: vi.fn(),
  saveConfig: vi.fn(),
  setPreferredAssignment: vi.fn(),
  updateRole: vi.fn(),
}));

const config: ConfigData = {
  agents: {
    codex: { command: "codex", args: ["exec"], env: {} },
    claude: { command: "claude", args: ["-p"], env: {} },
    gemini: { command: "gemini", args: ["-p"], env: {} },
  },
  roles: {},
  role_assignments: [],
};

function role(id: string, name: string, assignments: RoleInfo["assignments"]): RoleInfo {
  return {
    id,
    name,
    kind: "core",
    description: "",
    instructions: "",
    executionClass: "execution",
    assignments,
    available: assignments.length > 0,
  };
}

const workerRole = role("worker", "Worker", [
  { agent: "codex", preferred: true },
  { agent: "gemini", preferred: false },
]);
const reviewerRole = role("reviewer", "Reviewer", [{ agent: "codex", preferred: true }]);
const customRole: RoleInfo = {
  id: "database_engineer",
  name: "Database Engineer",
  kind: "custom",
  description: "Designs schema migrations.",
  instructions: "",
  executionClass: "advisory",
  assignments: [],
  available: false,
};

beforeEach(() => {
  vi.mocked(fetchConfig).mockReset().mockResolvedValue(config);
  vi.mocked(fetchAgents).mockReset().mockResolvedValue([]);
  vi.mocked(fetchRoles).mockReset().mockResolvedValue([workerRole, reviewerRole, customRole]);
  vi.mocked(fetchRecipes).mockReset().mockResolvedValue([]);
  vi.mocked(fetchMcpServers).mockReset().mockResolvedValue([]);
  vi.mocked(fetchMcpTools).mockReset().mockResolvedValue([]);
  vi.mocked(createMcpServer).mockReset();
  vi.mocked(updateMcpServer).mockReset();
  vi.mocked(deleteMcpServer).mockReset().mockResolvedValue();
  vi.mocked(testMcpServer).mockReset();
  vi.mocked(mcpDiscoveryRequestTimeoutMs)
    .mockReset()
    .mockImplementation((configured) =>
      Math.min(605_000, Math.max(15_000, (configured ?? 10_000) + 5_000))
    );
  vi.mocked(saveConfig).mockReset().mockResolvedValue();
  vi.mocked(addRoleAssignment).mockReset().mockResolvedValue(workerRole);
  vi.mocked(removeRoleAssignment).mockReset().mockResolvedValue(workerRole);
  vi.mocked(setPreferredAssignment).mockReset().mockResolvedValue(workerRole);
  vi.mocked(updateRole).mockReset().mockResolvedValue(customRole);
  vi.mocked(deleteRole).mockReset().mockResolvedValue();
  vi.spyOn(window, "confirm").mockReturnValue(true);
});

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe("Settings roles", () => {
  it("renders one card per role with agent chips and preferred markers", async () => {
    render(<SettingsView />);

    expect(await screen.findByText("Worker")).toBeTruthy();
    expect(screen.getByText("Reviewer")).toBeTruthy();
    expect(screen.getByText("Database Engineer")).toBeTruthy();
    expect(screen.getByText(/Custom role/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove codex from Worker" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove codex from Reviewer" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Remove gemini from Worker" })).toBeTruthy();
    expect(screen.getAllByLabelText("preferred").length).toBe(2);
    expect(screen.queryByRole("button", { name: "Edit Worker role" })).toBeNull();
    expect(screen.getAllByRole("button", { name: "Delete" }).length).toBe(1);
    expect(screen.getByRole("button", { name: "Edit Database Engineer role" })).toBeTruthy();
  });

  it("assigns and removes agents through the role assignment API", async () => {
    render(<SettingsView />);

    fireEvent.change(await screen.findByLabelText("Add agent to Worker"), {
      target: { value: "claude" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Assign agent to Worker" }));

    await waitFor(() => expect(addRoleAssignment).toHaveBeenCalledWith("worker", "claude"));

    fireEvent.click(screen.getByRole("button", { name: "Remove gemini from Worker" }));
    await waitFor(() => expect(removeRoleAssignment).toHaveBeenCalledWith("worker", "gemini"));
  });

  it("marks the preferred assignment from the radio group", async () => {
    render(<SettingsView />);

    fireEvent.click(await screen.findByLabelText("gemini"));

    await waitFor(() => expect(setPreferredAssignment).toHaveBeenCalledWith("worker", "gemini"));
  });

  it("edits a custom role definition", async () => {
    render(<SettingsView />);

    fireEvent.click(await screen.findByRole("button", { name: "Edit Database Engineer role" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Data Engineer" } });
    fireEvent.change(screen.getByLabelText("Description"), {
      target: { value: "Owns data pipelines." },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save role" }));

    await waitFor(() =>
      expect(updateRole).toHaveBeenCalledWith("database_engineer", {
        name: "Data Engineer",
        description: "Owns data pipelines.",
        executionClass: "advisory",
        instructions: "",
      })
    );
  });

  it("deletes a custom role and surfaces in-use failures", async () => {
    vi.mocked(deleteRole).mockRejectedValue(
      new Error("Factory API request failed (HTTP 409): used by an active workflow")
    );
    render(<SettingsView />);

    fireEvent.click(await screen.findByRole("button", { name: "Delete" }));

    await waitFor(() => expect(deleteRole).toHaveBeenCalledWith("database_engineer"));
    expect(await screen.findByText(/used by an active workflow/)).toBeTruthy();
  });
});

describe("Settings agents", () => {
  async function openCustomAgentEditor() {
    fireEvent.click(await screen.findByRole("button", { name: /^Add agent$/ }));
    fireEvent.change(screen.getByLabelText("Agent type"), { target: { value: "custom" } });
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "test-agent" } });
    fireEvent.change(screen.getByLabelText("Command"), { target: { value: "test-agent" } });
  }

  function saveNewAgent() {
    const buttons = screen.getAllByRole("button", { name: /^Add agent$/ });
    fireEvent.click(buttons[buttons.length - 1]);
  }

  it("saves zero arguments and ignores empty rows", async () => {
    render(<SettingsView />);
    await openCustomAgentEditor();
    fireEvent.click(screen.getByRole("button", { name: "+ Add argument" }));
    saveNewAgent();

    await waitFor(() =>
      expect(saveConfig).toHaveBeenCalledWith(
        expect.objectContaining({
          agents: expect.objectContaining({
            "test-agent": expect.objectContaining({ args: [] }),
          }),
        })
      )
    );
  });

  it("keeps one argument containing spaces as one argv entry", async () => {
    render(<SettingsView />);
    await openCustomAgentEditor();
    fireEvent.click(screen.getByRole("button", { name: "+ Add argument" }));
    fireEvent.change(screen.getByLabelText("Argument 1"), {
      target: { value: "--model openai/gpt-5" },
    });
    saveNewAgent();

    await waitFor(() =>
      expect(saveConfig).toHaveBeenCalledWith(
        expect.objectContaining({
          agents: expect.objectContaining({
            "test-agent": expect.objectContaining({ args: ["--model openai/gpt-5"] }),
          }),
        })
      )
    );
  });

  it("preserves multiple argument order and supports reordering rows", async () => {
    render(<SettingsView />);
    await openCustomAgentEditor();
    const addArgument = screen.getByRole("button", { name: "+ Add argument" });
    for (let index = 0; index < 5; index += 1) fireEvent.click(addArgument);

    ["run", "--model", "openai/gpt-5", "--format", "json"].forEach((value, index) => {
      fireEvent.change(screen.getByLabelText(`Argument ${index + 1}`), {
        target: { value },
      });
    });
    fireEvent.click(screen.getByRole("button", { name: "Move argument 5 up" }));
    fireEvent.click(screen.getByRole("button", { name: "Move argument 4 down" }));
    saveNewAgent();

    await waitFor(() =>
      expect(saveConfig).toHaveBeenCalledWith(
        expect.objectContaining({
          agents: expect.objectContaining({
            "test-agent": expect.objectContaining({
              args: ["run", "--model", "openai/gpt-5", "--format", "json"],
            }),
          }),
        })
      )
    );
  });

  it("removes an argument row before saving", async () => {
    render(<SettingsView />);
    await openCustomAgentEditor();
    const addArgument = screen.getByRole("button", { name: "+ Add argument" });
    fireEvent.click(addArgument);
    fireEvent.click(addArgument);
    fireEvent.change(screen.getByLabelText("Argument 1"), { target: { value: "first" } });
    fireEvent.change(screen.getByLabelText("Argument 2"), { target: { value: "second" } });
    fireEvent.click(screen.getByRole("button", { name: "Remove argument 1" }));
    saveNewAgent();

    await waitFor(() =>
      expect(saveConfig).toHaveBeenCalledWith(
        expect.objectContaining({
          agents: expect.objectContaining({
            "test-agent": expect.objectContaining({ args: ["second"] }),
          }),
        })
      )
    );
  });

  it("loads and saves existing arguments, including a mission placeholder, unchanged", async () => {
    const existing: ConfigData = {
      ...config,
      agents: {
        ...config.agents,
        opencode: {
          command: "opencode",
          args: ["run", "--model", "openai/gpt-5", "{mission}", "--label hello world"],
          env: {},
          max_concurrency: 3,
        },
      },
    };
    vi.mocked(fetchConfig).mockReset().mockResolvedValueOnce(existing).mockResolvedValue(config);
    render(<SettingsView />);

    const row = await screen.findByRole("row", { name: /opencode/ });
    fireEvent.click(within(row).getByRole("button", { name: "Edit" }));
    expect((screen.getByLabelText("Argument 1") as HTMLInputElement).value).toBe("run");
    expect((screen.getByLabelText("Argument 4") as HTMLInputElement).value).toBe("{mission}");
    expect((screen.getByLabelText("Argument 5") as HTMLInputElement).value).toBe(
      "--label hello world"
    );
    expect((screen.getByLabelText("Max concurrency (optional)") as HTMLInputElement).value).toBe(
      "3"
    );
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(saveConfig).toHaveBeenCalledWith(
        expect.objectContaining({
          agents: expect.objectContaining({
            opencode: expect.objectContaining({
              args: ["run", "--model", "openai/gpt-5", "{mission}", "--label hello world"],
            }),
          }),
        })
      )
    );
  });

  it("opens an existing agent editor and prefills API-shaped values when env is omitted", async () => {
    const existing: ConfigData = {
      ...config,
      agents: {
        ...config.agents,
        opencode: {
          kind: "custom",
          command: "opencode",
          args: ["run", "--model", "openai/gpt-5", "{mission}"],
          prompt_transport: "argument",
          interactive_args: ["--interactive"],
          capabilities: ["coding", "review"],
          max_concurrency: 3,
        },
      },
    };
    vi.mocked(fetchConfig).mockReset().mockResolvedValue(existing);
    render(<SettingsView />);

    const row = await screen.findByRole("row", { name: /opencode/ });
    fireEvent.click(within(row).getByRole("button", { name: "Edit" }));

    expect(await screen.findByRole("region", { name: "Agent configuration form" })).toBeTruthy();
    expect((screen.getByLabelText("Name") as HTMLInputElement).value).toBe("opencode");
    expect((screen.getByLabelText("Command") as HTMLInputElement).value).toBe("opencode");
    expect((screen.getByLabelText("Argument 1") as HTMLInputElement).value).toBe("run");
    expect((screen.getByLabelText("Argument 3") as HTMLInputElement).value).toBe("openai/gpt-5");
    expect((screen.getByLabelText("Argument 4") as HTMLInputElement).value).toBe("{mission}");
    expect((screen.getByLabelText("Workflow prompt transport") as HTMLSelectElement).value).toBe(
      "argument"
    );
    expect(
      (screen.getByLabelText("Interactive arguments (one per line)") as HTMLTextAreaElement).value
    ).toBe("--interactive");
    expect((screen.getByLabelText("Max concurrency (optional)") as HTMLInputElement).value).toBe(
      "3"
    );
    expect(
      (screen.getByLabelText("Capabilities (one per line)") as HTMLTextAreaElement).value
    ).toBe("coding\nreview");
  });

  it("saves an existing agent through config without creating a duplicate or changing assignments", async () => {
    const existing: ConfigData = {
      ...config,
      agents: {
        codex: { command: "codex", args: ["exec"], env: {} },
        opencode: {
          kind: "custom",
          command: "opencode",
          args: ["run", "{mission}"],
        },
      },
      role_assignments: [
        { role: "worker", agent: "opencode", preferred: true },
        { role: "reviewer", agent: "codex", preferred: false },
      ],
    };
    vi.mocked(fetchConfig).mockReset().mockResolvedValue(existing);
    render(<SettingsView />);

    const row = await screen.findByRole("row", { name: /opencode/ });
    fireEvent.click(within(row).getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Command"), { target: { value: "opencode-next" } });
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() => expect(saveConfig).toHaveBeenCalledTimes(1));
    const saved = vi.mocked(saveConfig).mock.calls[0][0];
    expect(Object.keys(saved.agents).sort()).toEqual(["codex", "opencode"]);
    expect(saved.agents.opencode).toEqual(
      expect.objectContaining({ command: "opencode-next", args: ["run", "{mission}"] })
    );
    expect(saved.role_assignments).toEqual(existing.role_assignments);
  });

  it("cancels an existing agent edit without saving or changing the entry", async () => {
    const existing: ConfigData = {
      ...config,
      agents: {
        ...config.agents,
        opencode: { command: "opencode", args: ["run"], env: {} },
      },
    };
    vi.mocked(fetchConfig).mockReset().mockResolvedValue(existing);
    render(<SettingsView />);

    const row = await screen.findByRole("row", { name: /opencode/ });
    fireEvent.click(within(row).getByRole("button", { name: "Edit" }));
    fireEvent.change(screen.getByLabelText("Command"), { target: { value: "changed" } });
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(screen.queryByRole("region", { name: "Agent configuration form" })).toBeNull();
    expect(saveConfig).not.toHaveBeenCalled();
    expect(screen.getByRole("row", { name: /opencode/ })).toBeTruthy();
  });

  it("shows a config API error and keeps the existing editor open", async () => {
    const existing: ConfigData = {
      ...config,
      agents: {
        ...config.agents,
        opencode: { command: "opencode", args: ["run"], env: {} },
      },
    };
    vi.mocked(fetchConfig).mockReset().mockResolvedValue(existing);
    vi.mocked(saveConfig).mockRejectedValue(new Error("invalid agent configuration"));
    render(<SettingsView />);

    const row = await screen.findByRole("row", { name: /opencode/ });
    fireEvent.click(within(row).getByRole("button", { name: "Edit" }));
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    expect((await screen.findByRole("alert")).textContent).toContain("invalid agent configuration");
    expect(screen.getByRole("region", { name: "Agent configuration form" })).toBeTruthy();
  });

  it("loads a legacy agent configuration without an args field", async () => {
    const legacy: ConfigData = {
      ...config,
      agents: {
        codex: { command: "codex", env: {} },
      },
    };
    vi.mocked(fetchConfig).mockReset().mockResolvedValueOnce(legacy).mockResolvedValue(config);
    render(<SettingsView />);

    const row = await screen.findByRole("row", { name: /codex/ });
    fireEvent.click(within(row).getByRole("button", { name: "Edit" }));
    expect(screen.queryByLabelText("Argument 1")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }));

    await waitFor(() =>
      expect(saveConfig).toHaveBeenCalledWith(
        expect.objectContaining({
          agents: expect.objectContaining({
            codex: expect.objectContaining({ args: [] }),
          }),
        })
      )
    );
  });
});

const builtinSecurity: Recipe = {
  id: "security",
  name: "Security Sensitive",
  description: "Security-relevant work.",
  roles: ["planner", "worker", "reviewer"],
  routing: "project_default",
  maxParallelTasks: 2,
  policyPresets: {},
  plannerGuidance: "",
  builtin: true,
};

const customRecipe: Recipe = {
  id: "backend_feature",
  name: "Backend Feature",
  description: "Backend implementation.",
  roles: ["planner", "worker", "reviewer"],
  routing: "performance",
  maxParallelTasks: null,
  policyPresets: {},
  plannerGuidance: "Prefer a compact plan.",
  builtin: false,
};

describe("Settings MCP servers", () => {
  const server: McpServerView = {
    id: "docs",
    transport: "http",
    args: [],
    url: "http://127.0.0.1:3001/mcp",
    env: {},
    headers: {},
    timeoutMs: 60000,
    discoveryTimeoutMs: 10000,
    maxResponseBytes: 262144,
    maxConcurrency: 4,
    classifications: {},
    status: "connected",
    toolCount: 1,
  };

  it("tests, inspects, edits, and removes a configured server", async () => {
    vi.mocked(fetchMcpServers).mockResolvedValue([server]);
    vi.mocked(testMcpServer).mockResolvedValue(server);
    vi.mocked(fetchMcpTools).mockResolvedValue([
      {
        server: "docs",
        name: "search",
        qualifiedName: "docs.search",
        description: "Search documentation.",
        inputSchema: { type: "object" },
        classification: "read",
        discoveredAt: "2026-08-23T00:00:00Z",
      },
    ]);
    render(<SettingsView />);

    expect(await screen.findByText(/Connected/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Test connection" }));
    await waitFor(() => expect(testMcpServer).toHaveBeenCalledWith("docs", { timeoutMs: 15_000 }));
    fireEvent.click(screen.getByRole("button", { name: "Inspect tools" }));
    expect(await screen.findByText("docs.search")).toBeTruthy();
    expect(fetchMcpTools).toHaveBeenCalledWith("docs", { timeoutMs: 15_000 });
    const mcpSection = screen.getByLabelText("External Tools MCP");
    fireEvent.click(within(mcpSection).getByRole("button", { name: "Edit" }));
    expect((screen.getByLabelText("Server id") as HTMLInputElement).value).toBe("docs");
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    fireEvent.click(within(mcpSection).getByRole("button", { name: "Remove" }));
    await waitFor(() => expect(deleteMcpServer).toHaveBeenCalledWith("docs"));
  });

  it("adds a structured stdio server with environment references", async () => {
    vi.mocked(createMcpServer).mockResolvedValue({ ...server, id: "github" });
    render(<SettingsView />);
    fireEvent.click(await screen.findByRole("button", { name: "Add server" }));
    fireEvent.change(screen.getByLabelText("Server id"), { target: { value: "github" } });
    fireEvent.change(screen.getByLabelText("Command"), {
      target: { value: "github-mcp-server" },
    });
    fireEvent.change(screen.getByLabelText(/Environment references/), {
      target: { value: "GITHUB_TOKEN=${GITHUB_TOKEN}" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save server" }));
    await waitFor(() =>
      expect(createMcpServer).toHaveBeenCalledWith(
        "github",
        expect.objectContaining({
          transport: "stdio",
          command: "github-mcp-server",
          env: { GITHUB_TOKEN: "${GITHUB_TOKEN}" },
        })
      )
    );
  });

  it("shows MCP discovery pending state and ignores duplicate actions", async () => {
    vi.mocked(fetchMcpServers).mockResolvedValue([server]);
    let resolveTest!: (value: McpServerView) => void;
    vi.mocked(testMcpServer).mockReturnValue(
      new Promise<McpServerView>((resolve) => {
        resolveTest = resolve;
      })
    );

    render(<SettingsView />);
    const testButton = await screen.findByRole("button", { name: "Test connection" });
    fireEvent.click(testButton);
    fireEvent.click(testButton);

    expect(await screen.findByRole("button", { name: "Testing…" })).toBeTruthy();
    expect(testMcpServer).toHaveBeenCalledTimes(1);
    expect(testMcpServer).toHaveBeenCalledWith("docs", { timeoutMs: 15_000 });
    expect(
      (screen.getByRole("button", { name: "Inspect tools" }) as HTMLButtonElement).disabled
    ).toBe(true);

    resolveTest(server);
    expect(await screen.findByRole("button", { name: "Test connection" })).toBeTruthy();

    let resolveTools!: (tools: McpTool[]) => void;
    vi.mocked(fetchMcpTools).mockReturnValue(
      new Promise<McpTool[]>((resolve) => {
        resolveTools = resolve;
      })
    );
    const inspectButton = screen.getByRole("button", { name: "Inspect tools" });
    fireEvent.click(inspectButton);
    fireEvent.click(inspectButton);

    expect(await screen.findByRole("button", { name: "Inspecting…" })).toBeTruthy();
    expect(fetchMcpTools).toHaveBeenCalledTimes(1);
    expect(fetchMcpTools).toHaveBeenCalledWith("docs", { timeoutMs: 15_000 });

    resolveTools([]);
    expect(await screen.findByRole("button", { name: "Inspect tools" })).toBeTruthy();
  });
});

describe("Settings recipes", () => {
  beforeEach(() => {
    vi.mocked(fetchConfig).mockReset().mockResolvedValue(config);
    vi.mocked(fetchAgents).mockReset().mockResolvedValue([]);
    vi.mocked(fetchRoles)
      .mockReset()
      .mockResolvedValue([
        workerRole,
        reviewerRole,
        role("planner", "Planner", [{ agent: "codex", preferred: true }]),
      ]);
    vi.mocked(fetchRecipes).mockReset().mockResolvedValue([builtinSecurity, customRecipe]);
    vi.mocked(saveConfig).mockReset().mockResolvedValue();
    vi.mocked(createRecipe).mockReset().mockResolvedValue(customRecipe);
    vi.mocked(updateRecipe).mockReset().mockResolvedValue(customRecipe);
    vi.mocked(deleteRecipe).mockReset().mockResolvedValue();
    vi.spyOn(window, "confirm").mockReturnValue(true);
  });

  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("lists built-in read-only and custom editable recipes", async () => {
    render(<SettingsView />);

    expect(await screen.findByText("Security Sensitive")).toBeTruthy();
    expect(screen.getByText("Backend Feature")).toBeTruthy();
    expect(screen.getAllByText(/Built-in · read-only/).length).toBe(1);
    expect(screen.getByText(/Custom · Performance-aware/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Duplicate Security Sensitive" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Duplicate Backend Feature" })).toBeTruthy();
    expect(screen.getAllByRole("button", { name: /^Delete$/ }).length).toBe(1);
    expect(screen.getByRole("button", { name: "Edit Backend Feature recipe" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Edit Security Sensitive recipe" })).toBeNull();
  });

  it("creates a custom recipe from the compact editor", async () => {
    render(<SettingsView />);

    fireEvent.click(await screen.findByRole("button", { name: "New recipe" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Hotfix" } });
    fireEvent.change(screen.getByLabelText("Routing"), { target: { value: "round_robin" } });
    fireEvent.change(screen.getByLabelText("Parallelism (1–32)"), {
      target: { value: "2" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Create recipe" }));

    await waitFor(() =>
      expect(createRecipe).toHaveBeenCalledWith(
        expect.objectContaining({
          name: "Hotfix",
          roles: ["planner"],
          routing: "round_robin",
          maxParallelTasks: 2,
        })
      )
    );
  });

  it("duplicates a built-in recipe into a custom one", async () => {
    render(<SettingsView />);

    fireEvent.click(await screen.findByRole("button", { name: "Duplicate Security Sensitive" }));
    const name = await screen.findByLabelText("Name");
    expect((name as HTMLInputElement).value).toBe("Security Sensitive (copy)");
    fireEvent.click(screen.getByRole("button", { name: "Create recipe" }));

    await waitFor(() =>
      expect(createRecipe).toHaveBeenCalledWith(
        expect.objectContaining({
          name: "Security Sensitive (copy)",
          roles: builtinSecurity.roles,
          maxParallelTasks: 2,
        })
      )
    );
  });

  it("edits and deletes a custom recipe", async () => {
    render(<SettingsView />);

    fireEvent.click(await screen.findByRole("button", { name: "Edit Backend Feature recipe" }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "Renamed" } });
    fireEvent.click(screen.getByRole("button", { name: "Save recipe" }));

    await waitFor(() =>
      expect(updateRecipe).toHaveBeenCalledWith(
        "backend_feature",
        expect.objectContaining({ name: "Renamed" })
      )
    );

    fireEvent.click(screen.getByRole("button", { name: /^Delete$/ }));
    await waitFor(() => expect(deleteRecipe).toHaveBeenCalledWith("backend_feature"));
  });
});
