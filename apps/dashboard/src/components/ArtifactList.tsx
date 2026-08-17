import { artifactTitle, formatArtifactContent } from "../artifactHelpers";
import type { RoleArtifact } from "../types";

export function ArtifactList({
  artifacts,
  empty = "No artifacts.",
}: {
  artifacts: RoleArtifact[];
  empty?: string;
}) {
  if (artifacts.length === 0) return <p className="inspector-hint">{empty}</p>;
  return (
    <ul className="artifact-list">
      {artifacts.map((artifact) => (
        <li key={artifact.id} className="artifact-item">
          <span className="artifact-kicker">
            {artifactTitle(artifact)}
            {artifact.taskId !== null ? ` · task #${artifact.taskId}` : ""}
            <em>{artifact.role}</em>
          </span>
          <details>
            <summary>Inspect content</summary>
            <pre className="artifact-content">{formatArtifactContent(artifact.content)}</pre>
          </details>
        </li>
      ))}
    </ul>
  );
}
