import { memo, useEffect, useMemo, useRef, useState } from "react";
import { FileTreeItem, EmptyState, PanelHeader } from "../../design-system/components/Application";
import { SearchField } from "../../design-system/primitives/Controls";
import { ContextMenu } from "../../design-system/primitives/Overlays";
import { GitModificationIndicator } from "../../design-system/patterns/Code";
import { useWorkspace, useWorkspaceState } from "../shell/context";

type TreeRow = {
  path: string;
  name: string;
  depth: number;
  directory: boolean;
  position?: number;
  total?: number;
};

export const Explorer = memo(function Explorer() {
  const model = useWorkspace();
  const state = useWorkspaceState();
  const [query, setQuery] = useState("");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [scrollTop, setScrollTop] = useState(0);
  const [height, setHeight] = useState(400);
  const [focusIndex, setFocusIndex] = useState(0);
  const host = useRef<HTMLDivElement>(null);
  const [rowHeight, setRowHeight] = useState(30);
  const rows = useMemo(() => {
    const entries = new Map<string, TreeRow>();
    const source =
      state.activity === "git"
        ? model.files.filter((file) => file.added || file.removed)
        : model.files;
    for (const file of source) {
      if (query && !file.path.toLowerCase().includes(query.toLowerCase())) continue;
      const parts = file.path.split("/");
      for (let index = 0; index < parts.length; index++) {
        const path = parts.slice(0, index + 1).join("/");
        entries.set(path, {
          path,
          name: parts[index],
          depth: index,
          directory: index < parts.length - 1,
        });
      }
    }
    const visible = [...entries.values()]
      .sort((a, b) => a.path.localeCompare(b.path))
      .filter((row) => query || ![...collapsed].some((path) => row.path.startsWith(path + "/")));
    const totals = new Map<string, number>();
    const positions = new Map<string, number>();
    for (const row of visible) {
      const parent = row.path.split("/").slice(0, -1).join("/");
      totals.set(parent, (totals.get(parent) ?? 0) + 1);
    }
    return visible.map((row) => {
      const parent = row.path.split("/").slice(0, -1).join("/");
      const position = (positions.get(parent) ?? 0) + 1;
      positions.set(parent, position);
      return { ...row, position, total: totals.get(parent) };
    });
  }, [model.files, state.activity, collapsed, query]);
  useEffect(() => {
    const element = host.current!;
    const measure = () => {
      setHeight(element.clientHeight);
      const measured = parseFloat(getComputedStyle(element).getPropertyValue("--row-height"));
      setRowHeight(measured * parseFloat(getComputedStyle(document.documentElement).fontSize));
    };
    const resize = new ResizeObserver(measure);
    const density = new MutationObserver(measure);
    density.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-density", "style"],
    });
    resize.observe(element);
    return () => {
      resize.disconnect();
      density.disconnect();
    };
  }, []);
  const start = Math.max(0, Math.floor(scrollTop / rowHeight) - 5);
  const end = Math.min(rows.length, start + Math.ceil(height / rowHeight) + 10);
  const activate = (row: TreeRow) => {
    if (row.directory)
      setCollapsed((previous) => {
        const next = new Set(previous);
        if (next.has(row.path)) next.delete(row.path);
        else next.add(row.path);
        return next;
      });
    else model.openFile(row.path);
  };
  const selectedIndex = Math.min(focusIndex, Math.max(rows.length - 1, 0));
  return (
    <section className="v2-explorer">
      <PanelHeader
        title={model.project === "atlas" ? "Atlas" : "Fieldnotes"}
        detail={state.activity === "git" ? "Fixture changes" : "Project fixture"}
      />
      <div className="v2-explorer-search">
        <SearchField
          label="Filter fixture files"
          value={query}
          onChange={(value) => {
            setQuery(value);
            setScrollTop(0);
            setFocusIndex(0);
            if (host.current) host.current.scrollTop = 0;
          }}
          placeholder="Filter files…"
        />
      </div>
      {state.activity === "git" && <div className="v2-list-caption">Changes in this fixture</div>}
      <div
        ref={host}
        className="v2-tree-scroll"
        role="tree"
        aria-label="Fixture project files"
        tabIndex={0}
        aria-activedescendant={
          rows.length && selectedIndex >= start && selectedIndex < end
            ? `tree-row-${selectedIndex}`
            : undefined
        }
        onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
        onKeyDown={(event) => {
          const row = rows[selectedIndex];
          if (!row) return;
          const next =
            event.key === "ArrowDown"
              ? Math.min(rows.length - 1, selectedIndex + 1)
              : event.key === "ArrowUp"
                ? Math.max(0, selectedIndex - 1)
                : event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? rows.length - 1
                    : -1;
          if (next >= 0) {
            event.preventDefault();
            setFocusIndex(next);
            if (host.current) {
              const top = next * rowHeight;
              if (top < host.current.scrollTop || top + rowHeight > host.current.scrollTop + height)
                host.current.scrollTop = Math.max(0, top - height / 2);
            }
          } else if (event.shiftKey && event.key === "F10") {
            event.preventDefault();
            const target = document.getElementById(`tree-row-${selectedIndex}`);
            const rect = target?.getBoundingClientRect();
            if (target && rect)
              target.dispatchEvent(
                new MouseEvent("contextmenu", {
                  bubbles: true,
                  clientX: rect.left + 24,
                  clientY: rect.bottom,
                })
              );
          } else if (event.key === "Enter") {
            event.preventDefault();
            activate(row);
          } else if (event.key === "ArrowRight") {
            event.preventDefault();
            if (row.directory && collapsed.has(row.path)) activate(row);
            else if (row.directory) setFocusIndex(Math.min(rows.length - 1, selectedIndex + 1));
          } else if (event.key === "ArrowLeft") {
            event.preventDefault();
            if (row.directory && !collapsed.has(row.path)) activate(row);
            else {
              const parent = row.path.split("/").slice(0, -1).join("/");
              const index = rows.findIndex((item) => item.path === parent);
              if (index >= 0) setFocusIndex(index);
            }
          }
        }}
      >
        <div style={{ height: rows.length * rowHeight, position: "relative" }} role="group">
          <div style={{ position: "absolute", top: start * rowHeight, width: "100%" }}>
            {rows.slice(start, end).map((row, index) => (
              <ContextMenu
                key={row.path}
                label={`Actions for ${row.name}`}
                entries={
                  row.directory
                    ? [
                        {
                          id: "toggle",
                          label: collapsed.has(row.path) ? "Expand folder" : "Collapse folder",
                          icon: "folder",
                          action: () => activate(row),
                        },
                      ]
                    : [
                        {
                          id: "open",
                          label: "Open fixture buffer",
                          icon: "file",
                          action: () => model.openFile(row.path),
                        },
                        {
                          id: "split",
                          label: "Open in split view",
                          icon: "split",
                          action: () => model.openFile(row.path, true),
                        },
                        {
                          id: "context",
                          label: "Reference in conversation",
                          icon: "link",
                          action: () => {
                            model.showView("conversation");
                            model.draft = `Explain ${row.path}`;
                            model.notify("File reference selected", row.path);
                          },
                        },
                      ]
                }
              >
                <div className="v2-tree-row">
                  <FileTreeItem
                    id={`tree-row-${start + index}`}
                    path={row.path}
                    label={row.name}
                    depth={row.depth}
                    directory={row.directory}
                    expanded={!collapsed.has(row.path)}
                    selected={row.path === state.selectedFile}
                    focused={start + index === selectedIndex}
                    onSelect={() => {
                      setFocusIndex(start + index);
                      activate(row);
                    }}
                    position={row.position}
                    total={row.total}
                  />
                  {state.activity === "git" && !row.directory && (
                    <GitModificationIndicator
                      status="modified"
                      origin={model.files.find((file) => file.path === row.path)?.origin ?? "agent"}
                    />
                  )}
                </div>
              </ContextMenu>
            ))}
          </div>
        </div>
        {!rows.length && (
          <EmptyState
            title="No matching files"
            description="Try a shorter path or clear the filter."
          />
        )}
      </div>
      <div className="v2-list-footer">{model.files.length} fixture files · windowed list</div>
    </section>
  );
});
