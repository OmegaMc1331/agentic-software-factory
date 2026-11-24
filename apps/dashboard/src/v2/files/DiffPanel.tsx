import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { Select, Button } from "../design-system/primitives/Controls";
import { InlineAlert, LoadingIndicator } from "../design-system/components/Application";
import type { WorkspaceView } from "../workspace/production/model";
import type { DockSession, WorkspaceModel } from "../workspace/production/controller";
import { ComparisonView } from "./ComparisonView";
import type { FileSnapshot } from "./types";

export function DiffPanel({
  view,
  model,
  dock,
}: {
  view: WorkspaceView;
  model: WorkspaceModel;
  dock: DockSession;
}) {
  const manager = model.documents;
  const document = useSyncExternalStore(
    manager.subscribe,
    useCallback(
      () => (view.documentId ? manager.get(view.documentId, view.file) : undefined),
      [manager, view.documentId, view.file]
    )
  );
  const [source, setSource] = useState(view.comparison ?? "disk"),
    [head, setHead] = useState<FileSnapshot>(),
    [error, setError] = useState("");
  useEffect(() => {
    if (view.file && view.documentId)
      void manager.open(view.file, view.documentId).catch((error) => setError(String(error)));
  }, [manager, view.file, view.documentId]);
  useEffect(() => {
    if (source !== "head" || !view.file) return;
    let active = true;
    setHead(undefined);
    setError("");
    void manager.api
      .head(view.file)
      .then((value) => {
        if (active) setHead(value);
      })
      .catch((error) => {
        if (active) setError(String(error));
      });
    return () => {
      active = false;
    };
  }, [source, manager, view.file]);
  const incomplete =
    (!!document?.disk && document.disk.mode !== "text") ||
    (source === "head" && !!head && head.mode !== "text");
  const left =
    document && !incomplete
      ? source === "base"
        ? document.baseText.toString()
        : source === "head"
          ? head?.content
          : document.disk?.content
      : undefined;
  const right = document?.text.toString();
  if (!view.file || !view.documentId)
    return (
      <div className="ws-scroll">
        <h2>Select a file to compare</h2>
        <p>Open a source file and choose Compare disk, base or HEAD.</p>
        <Button onClick={() => dock.open("explorer")}>Open Explorer</Button>
      </div>
    );
  if (!document)
    return error ? (
      <InlineAlert tone="danger" title="Version unavailable">
        {error}
      </InlineAlert>
    ) : (
      <LoadingIndicator label="Loading file versions" />
    );
  const revision =
    source === "base"
      ? document.baseRevision
      : source === "head"
        ? head?.revision
        : document.disk?.revision;
  return (
    <div className="files-diff">
      <div className="files-toolbar">
        <Select
          label="Compare left version"
          value={source}
          onChange={(event) => {
            const value = event.target.value as typeof source;
            setSource(value);
            model.updateDocument((workspace) => ({
              ...workspace,
              sessions: workspace.sessions.map((session) => ({
                ...session,
                views: session.views.map((item) =>
                  item.id === view.id ? { ...item, comparison: value } : item
                ),
              })),
            }));
          }}
        >
          <option value="disk">Current disk</option>
          <option value="base">Loaded base</option>
          <option value="head">Git HEAD</option>
        </Select>
        <span>Both sides read only</span>
        <Button tone="quiet" onClick={() => void manager.refresh(document.id)}>
          Refresh disk
        </Button>
      </div>
      <div className="files-diff-labels">
        <span title={revision}>
          {source} · {revision ?? "unavailable"}
        </span>
        <span>
          Local buffer · {document.file.path} · edit {document.generation}
        </span>
      </div>
      {error && (
        <InlineAlert tone="danger" title="Version unavailable">
          {error}
        </InlineAlert>
      )}
      {(left === null || incomplete) && (
        <InlineAlert tone="warning" title="Text comparison unavailable">
          Binary, unsupported and preview-only files cannot be compared as complete UTF-8 text.
        </InlineAlert>
      )}
      {left !== undefined && left !== null && right !== undefined ? (
        <ComparisonView before={left} after={right} />
      ) : (
        <div className="files-merge" />
      )}
      <p className="files-diff-note">
        Comparing text lines; encoding, BOM and line-ending metadata remain visible in the editor.
        No Git staging or integration is performed.
      </p>
    </div>
  );
}
