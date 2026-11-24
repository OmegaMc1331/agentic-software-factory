import { useEffect, useRef, useState } from "react";
import { Button, Checkbox, Input, Select } from "../design-system/primitives/Controls";
import { InlineAlert } from "../design-system/components/Application";
import { useFileRoots } from "./useFileRoots";
import type { WorkspaceView } from "../workspace/production/model";
import type { DockSession, WorkspaceModel } from "../workspace/production/controller";
import { token, type SearchResponse } from "./types";

export function FileSearchPanel({
  view,
  model,
  dock,
}: {
  view: WorkspaceView;
  model: WorkspaceModel;
  dock: DockSession;
}) {
  const api = model.documents.api,
    catalog = useFileRoots(api, view.projectId);
  const [root, setRoot] = useState(view.state.rootId ?? ""),
    [query, setQuery] = useState(view.state.query ?? ""),
    [filter, setFilter] = useState(""),
    [content, setContent] = useState(view.state.searchContent ?? false),
    [regex, setRegex] = useState(false),
    [sensitive, setSensitive] = useState(false),
    [hidden, setHidden] = useState(false),
    [result, setResult] = useState<SearchResponse>(),
    [error, setError] = useState(""),
    [searching, setSearching] = useState(false);
  useEffect(() => {
    if (view.state.searchContent !== undefined) setContent(view.state.searchContent);
  }, [view.state.searchContent]);
  const active = useRef<{ id: string; controller: AbortController }>(),
    generation = useRef(0);
  const rootId = root || catalog.value?.roots.find((root) => root.kind === "repository")?.id || "";
  const cancel = () => {
    const current = active.current;
    if (current) {
      generation.current++;
      current.controller.abort();
      void api.cancel(view.projectId, current.id).catch(() => {});
      active.current = undefined;
    }
    setSearching(false);
  };
  useEffect(
    () => () => {
      const current = active.current;
      if (current) {
        current.controller.abort();
        void api.cancel(view.projectId, current.id).catch(() => {});
      }
    },
    [api, view.projectId]
  );
  const search = async () => {
    cancel();
    const ticket = ++generation.current,
      id = token("q"),
      controller = new AbortController();
    active.current = { id, controller };
    setSearching(true);
    setError("");
    setResult(undefined);
    model.updateView(view.sessionId, view.id, { query, rootId, searchContent: content });
    try {
      const response = await api.search(
        view.projectId,
        {
          id,
          root_id: rootId,
          query,
          content,
          regex,
          case_sensitive: sensitive,
          include_hidden: hidden,
          path_filter: filter,
        },
        controller.signal
      );
      if (ticket === generation.current) setResult(response);
    } catch (reason) {
      if (ticket === generation.current && !controller.signal.aborted) setError(String(reason));
    } finally {
      if (ticket === generation.current) {
        active.current = undefined;
        setSearching(false);
      }
    }
  };
  return (
    <div className="files-search">
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void search();
        }}
        className="files-search-form"
      >
        <Select
          label="Search root"
          value={rootId}
          onChange={(event) => {
            cancel();
            setRoot(event.target.value);
            setResult(undefined);
          }}
        >
          {catalog.value?.roots.map((root) => (
            <option key={root.id} value={root.id}>
              {root.label}
              {root.read_only ? " · read only" : ""}
            </option>
          ))}
        </Select>
        <Select
          label="Search mode"
          value={content ? "content" : "files"}
          onChange={(event) => {
            cancel();
            setContent(event.target.value === "content");
            setResult(undefined);
          }}
        >
          <option value="files">File names and paths</option>
          <option value="content">Repository content</option>
        </Select>
        <Input
          label={content ? "Search text" : "Find file"}
          type="search"
          value={query}
          maxLength={256}
          onChange={(event) => {
            cancel();
            setQuery(event.target.value);
          }}
        />
        <Input
          label="Path contains"
          value={filter}
          maxLength={256}
          onChange={(event) => setFilter(event.target.value)}
        />
        <div className="files-actions">
          <Checkbox
            label="Match case"
            checked={sensitive}
            onChange={(event) => setSensitive(event.target.checked)}
          />
          {content && (
            <Checkbox
              label="Regex"
              checked={regex}
              onChange={(event) => setRegex(event.target.checked)}
            />
          )}
          <Checkbox
            label="Include hidden and ignored"
            checked={hidden}
            onChange={(event) => setHidden(event.target.checked)}
          />
        </div>
        <div className="files-actions">
          <Button type="submit" disabled={!rootId || !query || searching}>
            {searching ? "Searching…" : "Search"}
          </Button>
          <Button disabled={!searching} tone="quiet" onClick={cancel}>
            Cancel search
          </Button>
        </div>
      </form>
      {catalog.error && (
        <InlineAlert title="Roots unavailable" tone="danger">
          {catalog.error}
        </InlineAlert>
      )}
      {error && (
        <InlineAlert title="Search failed" tone="danger">
          {error}
        </InlineAlert>
      )}
      {result && (
        <>
          <p className="files-search-summary" role="status">
            {result.hits.length} results ·{" "}
            {result.complete ? "Complete within the selected scope" : `Limited: ${result.reason}`} ·{" "}
            {result.scanned.toLocaleString()} entries · {result.bytes_read.toLocaleString()} bytes
            read
          </p>
          <div className="files-search-results" aria-label="Repository search results">
            {result.hits.map((hit, index) => (
              <button
                key={`${hit.file.path}:${hit.line}:${index}`}
                className="files-search-hit"
                onClick={() =>
                  void dock
                    .openFile(
                      hit.file,
                      "within",
                      hit.line ? { line: hit.line, column: hit.column ?? 1 } : undefined
                    )
                    .catch((error) => model.setIssue(String(error)))
                }
              >
                <span className="files-hit-path">
                  {hit.file.path}
                  {hit.line ? `:${hit.line}:${hit.column}` : ""}
                </span>
                {hit.preview && <code>{hit.preview}</code>}
              </button>
            ))}
          </div>
        </>
      )}
      <p className="files-hint">
        Search is scoped to this root. Git/ASF internals, links, binary content and oversized files
        are excluded. Content search also excludes common secret-file names.
      </p>
    </div>
  );
}
