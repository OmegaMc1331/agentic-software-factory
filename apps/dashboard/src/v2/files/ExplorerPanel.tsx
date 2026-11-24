import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Button, Checkbox, IconButton, Input, Select } from "../design-system/primitives/Controls";
import { Dialog, Menu, type MenuEntry } from "../design-system/primitives/Overlays";
import { InlineAlert } from "../design-system/components/Application";
import { Icon } from "../design-system/icons/Icon";
import type { DockSession, WorkspaceModel } from "../workspace/production/controller";
import type { WorkspaceView } from "../workspace/production/model";
import { useFileRoots } from "./useFileRoots";
import { filesChanged } from "./fileApi";
import type { DirectoryPage, FileEntry, FileRef, Recovery } from "./types";

type NodePage = DirectoryPage & { loading?: boolean; error?: string };
type Operation = { type: "file" | "directory" | "rename" | "move" | "delete"; entry?: FileEntry };
export function ExplorerPanel({
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
  const rootId =
    view.state.rootId ?? catalog.value?.roots.find((root) => root.kind === "repository")?.id ?? "";
  const root = catalog.value?.roots.find((root) => root.id === rootId);
  const expanded = useMemo(() => new Set(view.state.expanded ?? []), [view.state.expanded]);
  const [preview, setPreview] = useState<string>(),
    [purge, setPurge] = useState<Recovery>();
  const [pages, setPages] = useState<Record<string, NodePage>>({}),
    [selected, setSelected] = useState(view.state.selectedPath ?? ""),
    [scroll, setScroll] = useState(0),
    [height, setHeight] = useState(400),
    [menu, setMenu] = useState<{ x: number; y: number; entry: FileEntry }>(),
    [operation, setOperation] = useState<Operation>(),
    [destination, setDestination] = useState(""),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [recoveries, setRecoveries] = useState<Recovery[]>(),
    [recoveryOpen, setRecoveryOpen] = useState(false),
    [deleted, setDeleted] = useState<Recovery>();
  const tree = useRef<HTMLDivElement>(null),
    generation = useRef(0),
    pollOffset = useRef(0),
    pagesRef = useRef(pages),
    inFlight = useRef(new Set<string>());
  pagesRef.current = pages;
  const hidden = !!view.state.includeHidden;
  const reference = useCallback(
    (path: string): FileRef => ({ project_id: view.projectId, root_id: rootId, path }),
    [view.projectId, rootId]
  );
  const load = useCallback(
    async (path: string, more = false) => {
      const stamp = generation.current,
        key = `${stamp}:${path}`;
      if (inFlight.current.has(key)) return;
      if (
        more &&
        Object.values(pagesRef.current).reduce((count, page) => count + page.entries.length, 0) >=
          4000
      ) {
        setError(
          "Explorer has 4,000 cached entries. Collapse other branches before loading more, or search by path."
        );
        return;
      }
      inFlight.current.add(key);
      setPages((current) => ({
        ...current,
        [path]: {
          ...(current[path] ?? {
            entries: [],
            next: null,
            complete: true,
            reason: null,
            git_available: true,
          }),
          loading: true,
          error: "",
        },
      }));
      try {
        const next = await api.tree(
          view.projectId,
          rootId,
          path,
          hidden,
          more ? (pagesRef.current[path]?.next ?? undefined) : undefined
        );
        if (stamp === generation.current)
          setPages((current) => ({
            ...current,
            [path]: {
              ...next,
              entries: more
                ? [...(current[path]?.entries ?? []), ...next.entries].filter(
                    (entry, index, all) =>
                      all.findIndex((other) => other.path === entry.path) === index
                  )
                : next.entries,
            },
          }));
      } catch (reason) {
        if (stamp === generation.current)
          setPages((current) => ({
            ...current,
            [path]: {
              ...(current[path] ?? {
                entries: [],
                next: null,
                complete: false,
                reason: null,
                git_available: false,
              }),
              loading: false,
              error: String(reason),
            },
          }));
      } finally {
        inFlight.current.delete(key);
      }
    },
    [api, view.projectId, rootId, hidden]
  );
  useEffect(() => {
    const next = ++generation.current;
    setPages({});
    setScroll(0);
    if (rootId) void load("");
    return () => {
      generation.current = next + 1;
    };
  }, [rootId, hidden, load]);
  useEffect(() => {
    for (const path of expanded) if (!pages[path] && rootId) void load(path);
  }, [expanded, pages, load, rootId]);
  useEffect(() => {
    setPages((current) => {
      const keep = Object.entries(current).filter(([path]) => !path || expanded.has(path));
      return keep.length === Object.keys(current).length ? current : Object.fromEntries(keep);
    });
  }, [expanded]);
  useEffect(() => {
    const element = tree.current;
    if (!element) return;
    const observer = new ResizeObserver(() => setHeight(element.clientHeight));
    observer.observe(element);
    setHeight(element.clientHeight);
    return () => observer.disconnect();
  }, []);
  const refresh = useCallback(() => {
    void load("");
    for (const path of expanded) void load(path);
  }, [expanded, load]);
  useEffect(() => {
    const changed = (event: Event) => {
      const file = (event as CustomEvent<FileRef>).detail;
      if (file.project_id === view.projectId && file.root_id === rootId) refresh();
    };
    window.addEventListener("factory-files-changed", changed);
    const timer = setInterval(() => {
      if (document.visibilityState === "visible" && tree.current?.getBoundingClientRect().width) {
        void load("");
        const paths = [...expanded];
        for (let index = 0; index < Math.min(8, paths.length); index++)
          void load(paths[(pollOffset.current + index) % paths.length]);
        pollOffset.current = paths.length ? (pollOffset.current + 8) % paths.length : 0;
      }
    }, 5000);
    return () => {
      window.removeEventListener("factory-files-changed", changed);
      clearInterval(timer);
    };
  }, [refresh, view.projectId, rootId, load, expanded]);
  const rows = useMemo(() => {
    const result: Array<{ entry?: FileEntry; path: string; depth: number; page?: NodePage }> = [];
    const visit = (path: string, depth: number) => {
      const page = pages[path];
      for (const entry of page?.entries ?? []) {
        result.push({ entry, path: entry.path, depth });
        if (entry.kind === "directory" && expanded.has(entry.path)) visit(entry.path, depth + 1);
      }
      if (page?.loading || page?.error || page?.next || (page && !page.complete))
        result.push({ path, depth, page });
    };
    visit("", 0);
    return result;
  }, [pages, expanded]);
  const select = (path: string) => {
    setSelected(path);
    model.updateView(view.sessionId, view.id, { selectedPath: path });
  };
  const toggle = (path: string, open = !expanded.has(path)) => {
    const next = new Set(expanded);
    if (open) {
      if (next.size >= 64) {
        setError("Collapse a directory before opening more than 64 branches.");
        return;
      }
      next.add(path);
    } else next.delete(path);
    model.updateView(view.sessionId, view.id, { expanded: [...next] });
  };
  const open = (entry: FileEntry, beside = false) => {
    select(entry.path);
    if (entry.protected) {
      setError("Links and protected entries cannot be opened through the editor.");
      return;
    }
    if (entry.kind === "directory") toggle(entry.path);
    else
      void dock
        .openFile(reference(entry.path), beside ? "right" : "within", undefined, beside)
        .catch((reason) => setError(String(reason)));
  };
  const begin = (type: Operation["type"], entry?: FileEntry) => {
    setOperation({ type, entry });
    setDestination(
      type === "rename"
        ? (entry?.name ?? "")
        : type === "move"
          ? (entry?.path ?? "")
          : entry?.kind === "directory"
            ? `${entry.path}/`
            : selected.includes("/")
              ? `${selected.slice(0, selected.lastIndexOf("/"))}/`
              : ""
    );
    setError("");
  };
  const execute = async () => {
    if (!operation) return;
    setBusy(true);
    setError("");
    try {
      const { type, entry } = operation;
      if (type === "file" || type === "directory") {
        const file = reference(destination);
        await api.create(file, type === "directory");
        filesChanged(file);
        if (type === "file") await dock.openFile(file);
      } else if (entry) {
        const file = reference(entry.path);
        if (type === "delete") {
          const recovery = await api.delete(file, entry.revision);
          setDeleted(recovery);
          await model.documents.refreshFile(file);
          filesChanged(file);
        } else {
          const parent = entry.path.includes("/")
            ? entry.path.slice(0, entry.path.lastIndexOf("/") + 1)
            : "";
          const moved = await api.move(
            file,
            entry.revision,
            type === "rename" ? parent + destination : destination
          );
          await model.documents.moved(moved);
          select(moved.to.path);
          filesChanged(moved.to);
        }
      }
      setOperation(undefined);
      refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };
  const restore = async (item: Recovery) => {
    setBusy(true);
    try {
      const file = await api.restore(view.projectId, item.id);
      filesChanged(file);
      await model.documents.refreshFile(file);
      setDeleted(undefined);
      setRecoveries(await api.recovery(view.projectId));
      refresh();
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };
  const reveal = () => {
    const active = dock.session.views.find((item) => item.id === dock.session.activeViewId)?.file;
    if (!active || active.project_id !== view.projectId) return;
    const segments = active.path.split("/");
    segments.pop();
    const paths = segments.map((_, index) => segments.slice(0, index + 1).join("/"));
    model.updateView(view.sessionId, view.id, {
      rootId: active.root_id,
      expanded: paths,
      selectedPath: active.path,
    });
    select(active.path);
  };
  const menuEntries = (entry: FileEntry): MenuEntry[] => [
    {
      id: "open",
      label: entry.kind === "directory" ? "Expand directory" : "Open file",
      disabled: entry.protected,
      action: () => open(entry),
    },
    {
      id: "beside",
      label: "Open file beside",
      disabled: entry.protected || entry.kind !== "file",
      action: () => open(entry, true),
    },
    {
      id: "copy",
      label: "Copy relative path",
      action: () =>
        void navigator.clipboard.writeText(entry.path).catch((reason) => setError(String(reason))),
    },
    {
      id: "file",
      label: "New file",
      disabled: root?.read_only,
      action: () => begin("file", entry),
    },
    {
      id: "directory",
      label: "New directory",
      disabled: root?.read_only,
      action: () => begin("directory", entry),
    },
    {
      id: "rename",
      label: "Rename",
      disabled: entry.protected || root?.read_only,
      action: () => begin("rename", entry),
    },
    {
      id: "move",
      label: "Move within root",
      disabled: entry.protected || root?.read_only,
      action: () => begin("move", entry),
    },
    {
      id: "delete",
      label: "Delete to ASF recovery",
      danger: true,
      disabled: entry.protected || root?.read_only,
      action: () => begin("delete", entry),
    },
  ];
  const first = Math.max(0, Math.floor(scroll / 24) - 8),
    last = Math.min(rows.length, Math.ceil((scroll + height) / 24) + 8);
  return (
    <div className="files-explorer">
      <div className="files-explorer-tools">
        <Select
          label="Filesystem root"
          value={rootId}
          onChange={(event) =>
            model.updateView(view.sessionId, view.id, { rootId: event.target.value, expanded: [] })
          }
        >
          {catalog.value?.roots.map((root) => (
            <option key={root.id} value={root.id}>
              {root.label}
              {root.read_only ? " · read only" : ""}
            </option>
          ))}
        </Select>
        <div className="files-actions">
          <IconButton label="Refresh explorer" icon="refresh" onClick={refresh} />
          <Button tone="quiet" disabled={!root || root.read_only} onClick={() => begin("file")}>
            New file
          </Button>
          <Button
            tone="quiet"
            disabled={!root || root.read_only}
            onClick={() => begin("directory")}
          >
            New directory
          </Button>
          <Button tone="quiet" onClick={reveal}>
            Reveal active file
          </Button>
        </div>
        <Checkbox
          label="Show hidden and ignored files"
          checked={hidden}
          onChange={(event) =>
            model.updateView(view.sessionId, view.id, { includeHidden: event.target.checked })
          }
        />
      </div>
      {(error || catalog.error) && (
        <InlineAlert title="File operation unavailable" tone="danger">
          {error || catalog.error}
        </InlineAlert>
      )}
      {catalog.value?.warning && (
        <InlineAlert title="Worktree discovery unavailable" tone="warning">
          {catalog.value.warning}
        </InlineAlert>
      )}
      {root?.read_only && (
        <p className="files-hint">
          Agent worktree: inspection only. Manual writes require future ownership coordination.
        </p>
      )}
      {deleted && (
        <div className="files-notice" role="status">
          Moved {deleted.path} to ASF recovery.{" "}
          <Button tone="quiet" disabled={busy} onClick={() => void restore(deleted)}>
            Undo deletion
          </Button>
        </div>
      )}
      <div
        className="files-tree"
        role="tree"
        aria-busy={Object.values(pages).some((page) => page.loading)}
        aria-label="Repository files"
        aria-activedescendant={
          selected && rows.slice(first, last).some((row) => row.entry && row.path === selected)
            ? `node-${view.id}-${encodeURIComponent(selected)}`
            : undefined
        }
        tabIndex={0}
        ref={tree}
        onScroll={(event) => setScroll(event.currentTarget.scrollTop)}
        onKeyDown={(event) => {
          const visible = rows.filter((row) => row.entry),
            index = visible.findIndex((row) => row.path === selected),
            entry = visible[index]?.entry;
          if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
            event.preventDefault();
            const next =
              event.key === "Home"
                ? 0
                : event.key === "End"
                  ? visible.length - 1
                  : Math.max(
                      0,
                      Math.min(visible.length - 1, index + (event.key === "ArrowDown" ? 1 : -1))
                    );
            const row = visible[next];
            if (row) {
              select(row.path);
              tree.current?.scrollTo({
                top: Math.max(
                  0,
                  rows.findIndex((item) => item.path === row.path) * 24 - height / 2
                ),
              });
            }
          } else if (entry && event.key === "ArrowRight" && entry.kind === "directory") {
            event.preventDefault();
            toggle(entry.path, true);
          } else if (entry && event.key === "ArrowLeft") {
            event.preventDefault();
            if (expanded.has(entry.path)) toggle(entry.path, false);
            else select(entry.path.slice(0, Math.max(0, entry.path.lastIndexOf("/"))));
          } else if (entry && event.key === "Enter") {
            event.preventDefault();
            open(entry);
          } else if (
            entry &&
            (event.key === "ContextMenu" || (event.shiftKey && event.key === "F10"))
          ) {
            event.preventDefault();
            const box = tree.current!.getBoundingClientRect();
            setMenu({ entry, x: box.left + 24, y: box.top + 40 });
          }
        }}
      >
        <div style={{ height: rows.length * 24, position: "relative" }}>
          {rows.slice(first, last).map((row, index) =>
            row.entry ? (
              <div
                key={row.path}
                id={`node-${view.id}-${encodeURIComponent(row.path)}`}
                role="treeitem"
                aria-level={row.depth + 1}
                aria-selected={selected === row.path}
                aria-expanded={row.entry.kind === "directory" ? expanded.has(row.path) : undefined}
                className="files-tree-row"
                data-selected={selected === row.path || undefined}
                style={{ top: (first + index) * 24, paddingLeft: 8 + row.depth * 14 }}
                title={`${row.path}${row.entry.git ? ` · Git ${row.entry.git}` : ""}${row.entry.protected ? " · access blocked" : ""}`}
                onClick={() => {
                  select(row.path);
                  if (row.entry?.kind === "directory") toggle(row.path);
                  else if (row.entry) open(row.entry);
                }}
                onContextMenu={(event) => {
                  event.preventDefault();
                  select(row.path);
                  setMenu({ entry: row.entry!, x: event.clientX, y: event.clientY });
                }}
              >
                <span className="files-tree-chevron" aria-hidden="true">
                  {row.entry.kind === "directory" ? (expanded.has(row.path) ? "⌄" : "›") : ""}
                </span>
                <span className="files-file-icon" aria-hidden="true">
                  <Icon
                    name={
                      row.entry.kind === "directory"
                        ? "folder"
                        : row.entry.protected
                          ? "error"
                          : "file"
                    }
                    size={14}
                  />
                </span>
                <span className="files-tree-name">{row.entry.name}</span>
                {row.entry.git && (
                  <small className="files-git-state" aria-label={`Git ${row.entry.git}`}>
                    {row.entry.git.trim()}
                  </small>
                )}
              </div>
            ) : (
              <div
                key={`${row.path}:page`}
                className="files-tree-row files-tree-page"
                style={{ top: (first + index) * 24, paddingLeft: 8 + row.depth * 14 }}
              >
                {row.page?.loading ? (
                  "Loading directory…"
                ) : row.page?.error ? (
                  <button onClick={() => void load(row.path)}>{row.page.error} · Retry</button>
                ) : row.page?.next ? (
                  <button onClick={() => void load(row.path, true)}>
                    Load next directory page
                  </button>
                ) : (
                  <span>Limited: {row.page?.reason ?? "some entries unavailable"}</span>
                )}
              </div>
            )
          )}
        </div>
        {!rows.length && (
          <p className="files-hint">
            {rootId
              ? "No visible entries. Refresh or show hidden files."
              : "Loading authorized roots…"}
          </p>
        )}
      </div>
      <div className="files-explorer-footer">
        <Button
          tone="quiet"
          onClick={() => {
            setRecoveryOpen(true);
            void api
              .recovery(view.projectId)
              .then(setRecoveries)
              .catch((reason) => setError(String(reason)));
          }}
        >
          ASF recovery
        </Button>
        <span>{rows.filter((row) => row.entry).length} loaded entries</span>
      </div>
      {menu && (
        <Menu
          label="File actions"
          anchor={menu}
          entries={menuEntries(menu.entry)}
          onClose={() => setMenu(undefined)}
          returnFocus={tree.current}
        />
      )}
      <Dialog
        open={!!operation}
        onClose={() => !busy && setOperation(undefined)}
        title={
          operation?.type === "delete"
            ? "Delete to ASF recovery?"
            : `${operation?.type ?? "File"} operation`
        }
        footer={
          <>
            <Button disabled={busy} onClick={() => setOperation(undefined)}>
              Cancel
            </Button>
            <Button
              tone={operation?.type === "delete" ? "danger" : "primary"}
              loading={busy}
              onClick={() => void execute()}
            >
              {operation?.type === "delete" ? "Confirm deletion" : "Apply file operation"}
            </Button>
          </>
        }
      >
        {operation?.type === "delete" ? (
          <p>
            {operation.entry?.path} ({operation.entry?.kind}) will be removed from this root and
            retained in the project's ASF recovery directory. Open drafts remain. Only empty
            directories can be deleted. This is not the OS trash.
          </p>
        ) : (
          <Input
            label={operation?.type === "rename" ? "New name" : "Relative destination path"}
            value={destination}
            onChange={(event) => setDestination(event.target.value)}
            autoFocus
          />
        )}
        {error && (
          <InlineAlert tone="danger" title="Operation failed">
            {error}
            <Button
              onClick={() => {
                const entry = operation?.entry;
                if (!entry) return;
                const parent = entry.path.includes("/")
                  ? entry.path.slice(0, entry.path.lastIndexOf("/"))
                  : "";
                void api
                  .tree(view.projectId, rootId, parent, true)
                  .then((page) => {
                    const current = page.entries.find((candidate) => candidate.path === entry.path);
                    if (!current)
                      throw new Error(
                        "The entry is no longer in this directory page. Refresh Explorer and select it again."
                      );
                    setOperation({ ...operation!, entry: current });
                    setError("");
                    refresh();
                  })
                  .catch((reason) => setError(String(reason)));
              }}
            >
              Refresh operation state
            </Button>
          </InlineAlert>
        )}
      </Dialog>
      <Dialog
        open={recoveryOpen}
        onClose={() => setRecoveryOpen(false)}
        title="Project file recovery"
      >
        <p>
          Deleted files are retained locally. Restore never overwrites a current file. Uncertain
          saves retain recovery evidence for inspection.
        </p>
        {error && (
          <InlineAlert tone="danger" title="Recovery operation failed">
            {error}
          </InlineAlert>
        )}
        {recoveries?.length ? (
          recoveries.map((item) => (
            <div className="files-recovery-row" key={item.id}>
              <span>
                {item.path} · {item.kind} · {item.phase}
              </span>
              <Button
                disabled={busy}
                onClick={() =>
                  void api
                    .recoveryContent(view.projectId, item.id)
                    .then((snapshot) =>
                      setPreview(
                        snapshot.content ??
                          `Metadata only: ${snapshot.size} bytes, ${snapshot.encoding}`
                      )
                    )
                    .catch((reason) => setError(String(reason)))
                }
              >
                Inspect retained version
              </Button>
              {((item.kind === "delete" && item.phase === "deleted") ||
                item.kind === "case_rename") && (
                <Button disabled={busy} onClick={() => void restore(item)}>
                  Restore file
                </Button>
              )}
              <Button tone="danger" disabled={busy} onClick={() => setPurge(item)}>
                Permanently discard recovery
              </Button>
            </div>
          ))
        ) : (
          <p>No recoverable file operations.</p>
        )}
      </Dialog>
      <Dialog
        open={preview !== undefined}
        onClose={() => setPreview(undefined)}
        title="Retained file version"
      >
        <p>Read-only recovery content. This version has not been applied to disk.</p>
        <pre className="files-recovery-content">{preview}</pre>
      </Dialog>
      <Dialog
        open={!!purge}
        onClose={() => !busy && setPurge(undefined)}
        title="Permanently discard this recovery?"
        footer={
          <>
            <Button onClick={() => setPurge(undefined)}>Cancel</Button>
            <Button
              tone="danger"
              loading={busy}
              onClick={() => {
                if (!purge) return;
                setBusy(true);
                void api
                  .purgeRecovery(view.projectId, purge.id)
                  .then(() => api.recovery(view.projectId))
                  .then((items) => {
                    setRecoveries(items);
                    setPurge(undefined);
                  })
                  .catch((reason) => setError(String(reason)))
                  .finally(() => setBusy(false));
              }}
            >
              Confirm permanent recovery removal
            </Button>
          </>
        }
      >
        <p>
          The retained backup and prepared temporary version for {purge?.path} will be removed
          permanently. The current repository file and editor draft are not changed.
        </p>
        {error && (
          <InlineAlert tone="danger" title="Recovery removal failed">
            {error}
          </InlineAlert>
        )}
      </Dialog>
    </div>
  );
}
