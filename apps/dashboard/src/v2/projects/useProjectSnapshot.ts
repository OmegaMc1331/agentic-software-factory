import { useEffect, useRef, useState } from "react";
import { projectSnapshot, type ProjectSnapshot } from "./projectApi";
export function useProjectSnapshot(projectId: string, revision: number) {
  const [snapshot, setSnapshot] = useState<ProjectSnapshot>();
  const [error, setError] = useState<{ projectId: string; message: string }>();
  const sequence = useRef(0);
  useEffect(() => {
    const current = ++sequence.current;
    const controller = new AbortController();
    setSnapshot(undefined);
    setError(undefined);
    if (projectId)
      void projectSnapshot(projectId, { signal: controller.signal })
        .then((value) => {
          if (current === sequence.current && !controller.signal.aborted) setSnapshot(value);
        })
        .catch((failure: unknown) => {
          if (current === sequence.current && !controller.signal.aborted)
            setError({
              projectId,
              message: failure instanceof Error ? failure.message : "Project could not be loaded.",
            });
        });
    return () => {
      controller.abort();
    };
  }, [projectId, revision]);
  return {
    snapshot: snapshot?.projectId === projectId ? snapshot : undefined,
    error: error?.projectId === projectId ? error.message : "",
  };
}
