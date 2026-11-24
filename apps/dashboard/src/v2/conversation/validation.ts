import type { Plan } from "./types";
export function validateGraph(plan: Plan): string {
  if (!plan.tasks.length) return "Add at least one task.";
  const ids = new Set(plan.tasks.map((t) => t.id));
  if (ids.size !== plan.tasks.length) return "Task IDs must be unique.";
  const visiting = new Set<string>(),
    done = new Set<string>();
  const visit = (id: string): boolean => {
    if (visiting.has(id)) return false;
    if (done.has(id)) return true;
    visiting.add(id);
    const task = plan.tasks.find((t) => t.id === id);
    if (!task || task.dependencies.some((d) => !ids.has(d) || !visit(d))) return false;
    visiting.delete(id);
    done.add(id);
    return true;
  };
  if (plan.tasks.some((t) => !visit(t.id))) return "Dependencies contain a cycle or unknown task.";
  if (
    plan.tasks.some(
      (t) => !t.title.trim() || !t.objective.trim() || !t.acceptanceCriteria.some((c) => c.trim())
    )
  )
    return "Every task needs a title, objective and acceptance criterion.";
  return "";
}
