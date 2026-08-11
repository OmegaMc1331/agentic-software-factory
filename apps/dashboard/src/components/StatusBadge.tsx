import type { TaskState } from "../types";
import { STATE_META } from "../state";

export function StatusBadge({ state }: { state: TaskState }) {
  const meta = STATE_META[state];
  return (
    <span className={`status status-${state}`} title={meta.label}>
      <span className="status-dot" style={{ backgroundColor: meta.color }} />
      {meta.label}
    </span>
  );
}
