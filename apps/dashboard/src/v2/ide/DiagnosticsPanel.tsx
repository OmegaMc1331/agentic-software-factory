import { useSyncExternalStore } from "react";
import { Button } from "../design-system/primitives/Controls";
import type { WorkspaceModel, DockSession } from "../workspace/production/controller";
export function DiagnosticsPanel({
  projectId,
  model,
  dock,
}: {
  projectId: string;
  model: WorkspaceModel;
  dock: DockSession;
}) {
  useSyncExternalStore(model.language.subscribe, model.language.snapshot);
  const diagnostics = [...model.language.connections.values()]
    .filter((connection) => connection.projectId === projectId)
    .flatMap((connection) =>
      [...connection.diagnostics.values()].filter((set) =>
        model.documents
          .all()
          .some(
            (doc) =>
              doc.file.project_id === projectId &&
              doc.file.root_id === set.file.root_id &&
              doc.file.path === set.file.path &&
              doc.generation === set.version
          )
      )
    );
  return (
    <div className="ws-scroll ide-diagnostics">
      <h2>Diagnostics</h2>
      <p>
        Language-server diagnostics for this project. Versioned results must match the current
        buffer; unversioned results are labelled because their freshness cannot be verified.
      </p>
      <ul className="ide-list">
        {diagnostics.flatMap((set) =>
          set.diagnostics.map((diagnostic, i) => (
            <li key={`${set.uri}:${i}`}>
              <Button
                tone="quiet"
                onClick={() =>
                  void dock.openFile(set.file, "within", {
                    line: diagnostic.range.start.line + 1,
                    column: diagnostic.range.start.character + 1,
                  })
                }
              >
                <span>
                  {diagnostic.severity === 1
                    ? "Error"
                    : diagnostic.severity === 2
                      ? "Warning"
                      : "Information"}{" "}
                  · {set.file.path}:{diagnostic.range.start.line + 1}
                </span>
                <span>{diagnostic.message}</span>
                <small>
                  {diagnostic.source ?? "Language server"}
                  {!set.versioned && " · Unversioned"}
                  {set.limited && " · First 1,000 diagnostics only"}
                </small>
              </Button>
            </li>
          ))
        )}
      </ul>
      {diagnostics.every((set) => !set.diagnostics.length) && (
        <p>
          No current diagnostics received. Start the configured language server from a source
          editor.
        </p>
      )}
    </div>
  );
}
