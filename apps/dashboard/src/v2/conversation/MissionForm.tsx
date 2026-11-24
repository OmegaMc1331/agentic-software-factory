import { useState } from "react";
import { ReactFlow, Background, useNodesState, type Node, type Edge } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { Button, Input, Textarea, Select } from "../design-system/primitives/Controls";
import { InlineAlert } from "../design-system/components/Application";
import { useData } from "../workspace/production/data";
import type { MissionRequest, PlannedTask } from "./types";
import { validateGraph } from "./validation";

function PlanGraph({ tasks, onSelect }: { tasks: PlannedTask[]; onSelect(id: string): void }) {
  const initial: Node[] = tasks.map((task, index) => ({
    id: task.id,
    data: { label: `${task.id} · ${task.title || "Untitled task"}` },
    position: { x: (index % 2) * 240, y: Math.floor(index / 2) * 100 },
    style: {
      background: "var(--color-panel)",
      color: "var(--color-text)",
      borderColor: "var(--color-border)",
      borderRadius: 3,
    },
  }));
  const [nodes, setNodes, onNodesChange] = useNodesState(initial);
  const ids = tasks.map((t) => `${t.id}:${t.title}`).join("|");
  const [previous, setPrevious] = useState(ids);
  if (previous !== ids) {
    setPrevious(ids);
    setNodes(initial);
  }
  const edges: Edge[] = tasks.flatMap((task) =>
    task.dependencies.map((dep) => ({
      id: `${dep}:${task.id}`,
      source: dep,
      target: task.id,
      style: { stroke: "var(--color-accent)" },
    }))
  );
  return (
    <div className="chat-plan-graph" aria-label="Executable task graph">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onNodeClick={(_, node) => onSelect(node.id)}
        fitView
        minZoom={0.3}
        maxZoom={1.5}
        nodesConnectable={false}
        deleteKeyCode={null}
        proOptions={{ hideAttribution: true }}
      >
        <Background color="var(--color-border)" />
      </ReactFlow>
    </div>
  );
}
export function MissionForm({
  project,
  mode,
  onSubmit,
  onCancel,
}: {
  project: string;
  mode: "form" | "visual";
  onSubmit(mission: MissionRequest): Promise<void>;
  onCancel(): void;
}) {
  const [objective, setObjective] = useState("");
  const [scope, setScope] = useState("");
  const [constraints, setConstraints] = useState("");
  const [criteria, setCriteria] = useState("");
  const [recipe, setRecipe] = useState("");
  const [tasks, setTasks] = useState<PlannedTask[]>([]);
  const [selected, setSelected] = useState("");
  const [preferred, setPreferred] = useState<Record<string, string>>({});
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const roles = useData<
    Array<{ id: string; name: string; assignments?: Array<{ agent: string; preferred: boolean }> }>
  >(project, "roles");
  const recipes = useData<Array<{ id: string; name: string }>>(project, "recipes");
  const update = (id: string, patch: Partial<PlannedTask>) =>
    setTasks((current) => current.map((task) => (task.id === id ? { ...task, ...patch } : task)));
  return (
    <form
      className="chat-mission-form"
      onSubmit={(event) => {
        event.preventDefault();
        const plan = { objective, tasks };
        const invalid = mode === "visual" ? validateGraph(plan) : "";
        if (invalid) {
          setError(invalid);
          return;
        }
        setBusy(true);
        setError("");
        void onSubmit({
          objective,
          scope,
          constraints,
          acceptance_criteria: criteria.split("\n").filter(Boolean),
          ...(recipe ? { recipe } : {}),
          ...(Object.values(preferred).some(Boolean)
            ? {
                team: {
                  planner:
                    preferred.planner ||
                    roles.value
                      ?.find((role) => role.id === "planner")
                      ?.assignments?.find((a) => a.preferred)?.agent ||
                    roles.value?.find((role) => role.id === "planner")?.assignments?.[0]?.agent ||
                    "",
                  workers: preferred.worker
                    ? [preferred.worker]
                    : roles.value
                        ?.find((role) => role.id === "worker")
                        ?.assignments?.map((a) => a.agent) || [],
                  reviewers: preferred.reviewer
                    ? [preferred.reviewer]
                    : roles.value
                        ?.find((role) => role.id === "reviewer")
                        ?.assignments?.map((a) => a.agent) || [],
                  additional: Object.fromEntries(
                    (roles.value ?? [])
                      .filter(
                        (role) =>
                          !["planner", "worker", "reviewer"].includes(role.id) &&
                          role.assignments?.length
                      )
                      .map((role) => [role.id, role.assignments?.map((a) => a.agent) || []])
                  ),
                },
              }
            : {}),
          ...(mode === "visual" ? { plan } : {}),
        })
          .catch((error) => setError(String(error)))
          .finally(() => setBusy(false));
      }}
    >
      <header>
        <h2>{mode === "visual" ? "Executable visual plan" : "New mission"}</h2>
        <p>
          The existing engine validates and starts the submitted mission under project policies.
          External publication remains a separate authorized action.
        </p>
      </header>
      <Textarea
        label="Mission objective"
        value={objective}
        onChange={(e) => setObjective(e.target.value)}
        required
        maxLength={16000}
      />
      <Input
        label="Scope"
        value={scope}
        onChange={(e) => setScope(e.target.value)}
        maxLength={4000}
      />
      <Textarea
        label="Constraints"
        value={constraints}
        onChange={(e) => setConstraints(e.target.value)}
        maxLength={4000}
      />
      <Textarea
        label="Acceptance criteria, one per line"
        value={criteria}
        onChange={(e) => setCriteria(e.target.value)}
        maxLength={16000}
      />
      <Select label="Existing recipe" value={recipe} onChange={(e) => setRecipe(e.target.value)}>
        <option value="">Configured project team and policies</option>
        {recipes.value?.map((recipe) => (
          <option key={recipe.id} value={recipe.id}>
            {recipe.name}
          </option>
        ))}
      </Select>
      <details>
        <summary>Preferred configured agents</summary>
        {["planner", "worker", "reviewer"].map((role) => (
          <Select
            key={role}
            label={`Preferred ${role}`}
            value={preferred[role] ?? ""}
            onChange={(event) => setPreferred({ ...preferred, [role]: event.target.value })}
          >
            <option value="">Project default</option>
            {roles.value
              ?.find((item) => item.id === role)
              ?.assignments?.map((assignment) => (
                <option key={assignment.agent} value={assignment.agent}>
                  {assignment.agent}
                </option>
              ))}
          </Select>
        ))}
      </details>
      {mode === "visual" && (
        <section aria-label="Plan tasks">
          <div className="chat-actions">
            <Button
              type="button"
              disabled={tasks.length >= 64}
              onClick={() => {
                const id = `T${Math.max(0, ...tasks.map((t) => Number(t.id.slice(1)) || 0)) + 1}`;
                setTasks([
                  ...tasks,
                  {
                    id,
                    title: "",
                    objective: "",
                    dependencies: [],
                    acceptanceCriteria: [],
                    role: "worker",
                  },
                ]);
                setSelected(id);
              }}
            >
              Add task
            </Button>
            <span>{tasks.length} tasks · dependencies are edited below</span>
          </div>
          <PlanGraph tasks={tasks} onSelect={setSelected} />
          {tasks.map((task) => (
            <fieldset
              key={task.id}
              className="chat-task"
              data-selected={task.id === selected}
              tabIndex={-1}
            >
              <legend>{task.id}</legend>
              <Input
                label={`Title ${task.id}`}
                value={task.title}
                onChange={(e) => update(task.id, { title: e.target.value })}
                required
              />
              <Textarea
                label={`Objective ${task.id}`}
                value={task.objective}
                onChange={(e) => update(task.id, { objective: e.target.value })}
                required
              />
              <Input
                label={`Dependencies ${task.id}`}
                hint="Comma-separated task IDs"
                value={task.dependencies.join(",")}
                onChange={(e) =>
                  update(task.id, {
                    dependencies: e.target.value
                      .split(",")
                      .map((v) => v.trim())
                      .filter(Boolean),
                  })
                }
              />
              <Input
                label={`Acceptance ${task.id}`}
                value={task.acceptanceCriteria.join("; ")}
                onChange={(e) =>
                  update(task.id, {
                    acceptanceCriteria: e.target.value
                      .split(";")
                      .map((v) => v.trim())
                      .filter(Boolean),
                  })
                }
                required
              />
              <Select
                label={`Role ${task.id}`}
                value={task.role}
                onChange={(e) => update(task.id, { role: e.target.value, operation: undefined })}
              >
                {(roles.value ?? [])
                  .filter((role) => role.id !== "planner")
                  .map((role) => (
                    <option key={role.id} value={role.id}>
                      {role.name}
                    </option>
                  ))}
              </Select>
              <Select
                label={`Operation ${task.id}`}
                value={task.operation ?? ""}
                onChange={(e) => update(task.id, { operation: e.target.value || undefined })}
              >
                <option value="">Derive from selected role</option>
                {["planning", "advisory", "implement", "verify", "review", "post_process"].map(
                  (op) => (
                    <option key={op}>{op}</option>
                  )
                )}
              </Select>
              <Button
                type="button"
                tone="danger"
                onClick={() => setTasks(tasks.filter((t) => t.id !== task.id))}
              >
                Remove {task.id}
              </Button>
            </fieldset>
          ))}
        </section>
      )}
      {error && (
        <InlineAlert tone="danger" title="Mission not submitted">
          {error}
        </InlineAlert>
      )}
      {(roles.error || recipes.error) && <p role="alert">{roles.error || recipes.error}</p>}
      <footer className="chat-actions">
        <Button type="button" onClick={onCancel}>
          Back to conversation
        </Button>
        <Button tone="primary" loading={busy} type="submit" disabled={!objective.trim() || busy}>
          {mode === "visual" ? "Validate and launch plan" : "Plan and start mission"}
        </Button>
      </footer>
    </form>
  );
}
