import { useMemo } from "react";
import type { ArtifactKind, RoleArtifact } from "./types";

export const ARTIFACT_LABELS: Record<ArtifactKind, string> = {
  research: "Research findings",
  architecture: "Architecture",
  analysis: "Analysis",
  review: "Specialized review",
  verification: "Verification report",
  documentation_context: "Documentation context",
};

export function artifactTitle(artifact: RoleArtifact): string {
  return (
    ARTIFACT_LABELS[artifact.kind] ??
    artifact.kind.replaceAll("_", " ").replace(/^\w/, (c) => c.toUpperCase())
  );
}

export function formatArtifactContent(content: string): string {
  const trimmed = content.trim();
  if (!trimmed.startsWith("{")) return content;
  try {
    return JSON.stringify(JSON.parse(trimmed), null, 2);
  } catch {
    return content;
  }
}

/** The artifact entries a task produced and the ones it may consume from its
 * direct dependencies, derived from one run's artifact set. */
export function artifactsForTask(
  artifacts: RoleArtifact[],
  taskId: number,
  dependencies: number[]
): { produced: RoleArtifact[]; consumed: RoleArtifact[] } {
  const produced = artifacts
    .filter((artifact) => artifact.taskId === taskId)
    .sort((a, b) => a.id - b.id);
  const consumed = artifacts
    .filter(
      (artifact) =>
        artifact.taskId !== null &&
        artifact.taskId !== taskId &&
        dependencies.includes(artifact.taskId)
    )
    .sort((a, b) => a.id - b.id);
  return { produced, consumed };
}

export function useRunArtifactsForTask(
  artifacts: RoleArtifact[] | null,
  taskId: number | null,
  dependencies: number[]
): { produced: RoleArtifact[]; consumed: RoleArtifact[] } {
  return useMemo(() => {
    if (!artifacts || taskId === null) return { produced: [], consumed: [] };
    return artifactsForTask(artifacts, taskId, dependencies);
  }, [artifacts, taskId, dependencies]);
}
