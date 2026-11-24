import { useEffect, useId, useMemo, useRef, useState } from "react";
import { Dialog } from "../design-system/primitives/Overlays";
import { Input } from "../design-system/primitives/Controls";
import { commandMatches, type WorkspaceCommand } from "./commands";
import "./product.css";

export function CommandPalette({
  open,
  onClose,
  onError,
  commands,
}: {
  open: boolean;
  onClose: () => void;
  onError: (error: unknown) => void;
  commands: WorkspaceCommand[];
}) {
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState(0);
  const search = useRef<HTMLInputElement>(null);
  const id = useId();
  const matches = useMemo(
    () => commands.filter((c) => commandMatches(c, query)).slice(0, 100),
    [commands, query]
  );
  const index = Math.min(selected, Math.max(0, matches.length - 1));
  useEffect(() => {
    if (!open) return;
    const frame = requestAnimationFrame(() => search.current?.focus());
    return () => cancelAnimationFrame(frame);
  }, [open]);
  useEffect(() => {
    document.getElementById(`${id}-${index}`)?.scrollIntoView?.({ block: "nearest" });
  }, [id, index]);
  const execute = (command: WorkspaceCommand) => {
    if (command.disabled) return;
    onClose();
    requestAnimationFrame(() => {
      void Promise.resolve()
        .then(() => command.run())
        .catch(onError);
    });
  };
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Command palette"
      description="Search commands and shortcuts. Disabled commands explain their requirements."
    >
      <Input
        ref={search}
        label="Search commands"
        value={query}
        role="combobox"
        aria-expanded="true"
        aria-controls={id}
        aria-activedescendant={matches.length ? `${id}-${index}` : undefined}
        onChange={(e) => {
          setQuery(e.target.value);
          setSelected(0);
        }}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown" || e.key === "ArrowUp") {
            e.preventDefault();
            setSelected(
              (v) =>
                (v + (e.key === "ArrowDown" ? 1 : -1) + matches.length) %
                Math.max(1, matches.length)
            );
          }
          if (e.key === "Home") {
            e.preventDefault();
            setSelected(0);
          }
          if (e.key === "End") {
            e.preventDefault();
            setSelected(matches.length - 1);
          }
          if (e.key === "Enter" && matches[index]) {
            e.preventDefault();
            execute(matches[index]);
          }
        }}
      />
      <div id={id} role="listbox" aria-label="Workspace commands" className="product-command-list">
        {matches.map((command, i) => (
          <div
            key={command.id}
            role="option"
            id={`${id}-${i}`}
            aria-selected={index === i}
            aria-disabled={!!command.disabled}
            className="product-command"
            onMouseMove={() => setSelected(i)}
            onClick={() => execute(command)}
          >
            <span>
              <strong>{command.title}</strong>
              <small>
                {command.category} · {command.disabled ?? command.description}
              </small>
            </span>
            {command.shortcut ? (
              <kbd>
                {command.shortcut.replace("Mod", navigator.platform.includes("Mac") ? "⌘" : "Ctrl")}
              </kbd>
            ) : null}
          </div>
        ))}
        {!matches.length ? (
          <p role="status">No matching commands. Try a view name or action.</p>
        ) : null}
      </div>
    </Dialog>
  );
}
