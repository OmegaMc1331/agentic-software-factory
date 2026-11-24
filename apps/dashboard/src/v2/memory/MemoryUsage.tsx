import { useState } from "react";
import { useMemoryData } from "./client";
import type { Usage } from "./types";
import "./memory.css";

export function MemoryUsage({ projectId, sessionId }: { projectId: string; sessionId: number }) {
  const [open, setOpen] = useState(false);
  return (
    <details className="asf-memory-usage" onToggle={(e) => setOpen(e.currentTarget.open)}>
      <summary>Project memory supplied to this invocation</summary>
      {open ? <UsageRecords projectId={projectId} sessionId={sessionId} /> : null}
    </details>
  );
}
function UsageRecords({ projectId, sessionId }: { projectId: string; sessionId: number }) {
  const data = useMemoryData<Usage[]>(projectId, `/usage?sessionId=${sessionId}`);
  return (
    <>
      {data.error ? (
        <p role="alert">{data.error}</p>
      ) : data.value?.length === 0 ? (
        <p>No memory audit was recorded for this invocation.</p>
      ) : !data.value ? (
        <p role="status">Loading recorded memory references…</p>
      ) : (
        data.value.map((u) => (
          <div key={u.id}>
            <p>
              {u.usedAt} · {u.role}. Supply is recorded; model reliance is not inferred.
            </p>
            {u.memories.length ? (
              <ul>
                {u.memories.map((m) => (
                  <li key={m.memoryId}>
                    {m.memoryId} · revision {m.revision}
                    <small>Content hash: {m.contentHash}</small>
                  </li>
                ))}
              </ul>
            ) : (
              <p>No saved memory was included.</p>
            )}
            <small>Context hash: {u.contextHash}</small>
          </div>
        ))
      )}
    </>
  );
}
