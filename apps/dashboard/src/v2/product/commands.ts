export type CommandScope = "Application" | "Project" | "Editor" | "Workspace";
export interface WorkspaceCommand {
  id: string;
  title: string;
  category: CommandScope;
  description: string;
  shortcut?: string;
  disabled?: string;
  terminal?: boolean;
  run: () => void | Promise<void>;
}
export function workspaceHasKeyboardFocus(workspace: HTMLElement | null) {
  if (!workspace || workspace.ownerDocument.querySelector("dialog[open]")) return false;
  const focused = workspace.ownerDocument.activeElement;
  return focused === workspace.ownerDocument.body || workspace.contains(focused);
}
export function shortcutKey(shortcut: string) {
  return shortcut
    .toLowerCase()
    .split("+")
    .map((s) => s.trim())
    .sort()
    .join("+");
}
export function shortcutConflicts(commands: WorkspaceCommand[]) {
  const owners = new Map<string, string[]>();
  for (const command of commands) {
    if (!command.shortcut) continue;
    const key = shortcutKey(command.shortcut);
    owners.set(key, [...(owners.get(key) ?? []), command.id]);
  }
  return [...owners].filter(([, ids]) => ids.length > 1);
}
export function commandMatches(command: WorkspaceCommand, query: string) {
  const text =
    `${command.title} ${command.category} ${command.description} ${command.shortcut ?? ""}`.toLowerCase();
  return query
    .toLowerCase()
    .replace(/\b(?:ctrl|control|cmd|command|meta)\s*\+/g, "mod+")
    .trim()
    .split(/\s+/)
    .every((word) => text.includes(word));
}
export function matchesShortcut(event: KeyboardEvent, shortcut: string) {
  const parts = shortcut.toLowerCase().split("+");
  const key = parts.at(-1);
  return (
    (parts.includes("mod") ? event.ctrlKey || event.metaKey : !event.ctrlKey && !event.metaKey) &&
    event.altKey === parts.includes("alt") &&
    event.shiftKey === parts.includes("shift") &&
    (event.key.toLowerCase() === key ||
      event.code === `Digit${key}` ||
      (key === "backquote" && event.code === "Backquote"))
  );
}
export function eligibleShortcut(
  event: KeyboardEvent,
  commands: WorkspaceCommand[],
  terminal: boolean
) {
  if (event.defaultPrevented || event.isComposing) return;
  const conflicts = new Set(shortcutConflicts(commands).map(([key]) => key));
  return commands.find(
    (command) =>
      command.shortcut &&
      !command.disabled &&
      (!terminal || command.terminal) &&
      !conflicts.has(shortcutKey(command.shortcut)) &&
      matchesShortcut(event, command.shortcut)
  );
}
