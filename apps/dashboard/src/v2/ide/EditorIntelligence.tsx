import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { keymap } from "@codemirror/view";
import type { EditorView } from "@codemirror/view";
import { Button, Input, Select } from "../design-system/primitives/Controls";
import { Dialog, Menu, type MenuEntry } from "../design-system/primitives/Overlays";
import { InlineAlert } from "../design-system/components/Application";
import type { WorkspaceModel, DockSession } from "../workspace/production/controller";
import type { EditorDocument } from "../files/documents";
import { fileKey } from "../files/types";
import type { EditPlan, InstalledExtension } from "./types";
import { useIde } from "./hooks";
import { editorContributions, formatText, formatterApplies } from "./extensions";
import { languageId } from "./client";

interface Location {
  uri?: string;
  targetUri?: string;
  range?: { start: { line: number; character: number } };
  targetSelectionRange?: { start: { line: number; character: number } };
  location?: Location;
  name?: string;
}
interface CodeAction {
  title: string;
  edit?: unknown;
  command?: unknown;
  disabled?: { reason: string };
}
export function EditorIntelligence({
  model,
  dock,
  document: doc,
  editor,
}: {
  model: WorkspaceModel;
  dock: DockSession;
  document: EditorDocument;
  editor: React.MutableRefObject<EditorView | undefined>;
}) {
  useSyncExternalStore(model.language.subscribe, model.language.snapshot);
  const state = useIde(doc.file.project_id);
  const connection = model.language.forDocument(doc);
  const [profile, setProfile] = useState(""),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [rename, setRename] = useState(false),
    [newName, setNewName] = useState(""),
    [plans, setPlans] = useState<EditPlan[]>([]),
    [locations, setLocations] = useState<Location[]>([]),
    [query, setQuery] = useState("");
  const [context, setContext] = useState<{ x: number; y: number } | null>(null);
  const [actions, setActions] = useState<CodeAction[] | null>(null);
  const actionGeneration = useRef(0);
  const commands = useRef<Record<string, () => void>>({});
  const profiles =
    state.value?.profiles.filter(
      (profile) =>
        profile.kind === "lsp" &&
        profile.enabled &&
        profile.languages.includes(languageId(doc.file.path))
    ) ?? [];
  const installedJson = JSON.stringify(state.value?.extensions ?? []);
  const documentId = doc.id,
    documentPath = doc.file.path;
  useEffect(() => {
    let active = true;
    void (async () => {
      const currentDocument = model.documents.get(documentId);
      if (!currentDocument) return;
      const local = await editorContributions(
        JSON.parse(installedJson) as InstalledExtension[],
        documentPath,
        (extension, id) => {
          const command = extension.manifest.contributions.commands?.find((c) => c.id === id);
          if (command?.action === "open-view")
            dock.open("extensionView", {
              agent: `${extension.manifest.id}/${command.target}`,
              title: command.title,
            });
          if (command?.action === "format") {
            const formatter = extension.manifest.contributions.formatters?.find(
              (f) => f.id === command.target
            );
            const current = model.documents.get(documentId);
            if (formatter && current && !current.disk?.read_only)
              model.documents.setText(
                documentId,
                formatText(model.documents.raw(current), formatter.operation)
              );
          }
        }
      );
      const remote = connection ? await connection.attach(currentDocument) : [];
      if (active)
        model.documents.configureIde(documentId, [
          ...remote,
          ...local,
          keymap.of([
            {
              key: "F2",
              run: () => {
                commands.current.rename?.();
                return !!connection?.info.capabilities.renameProvider;
              },
            },
            {
              key: "F12",
              run: () => {
                commands.current.definition?.();
                return !!connection?.info.capabilities.definitionProvider;
              },
            },
            {
              key: "Shift-F12",
              run: () => {
                commands.current.references?.();
                return !!connection?.info.capabilities.referencesProvider;
              },
            },
            {
              key: "Alt-Shift-f",
              run: () => {
                commands.current.format?.();
                return !!connection?.info.capabilities.documentFormattingProvider;
              },
            },
            {
              key: "Shift-F10",
              run: (view) => {
                const box = view.coordsAtPos(view.state.selection.main.head);
                if (box) setContext({ x: box.left, y: box.bottom });
                return true;
              },
            },
          ]),
        ]);
    })().catch((error) => {
      if (active) setError(String(error));
    });
    return () => {
      active = false;
    };
  }, [model, documentId, documentPath, connection, installedJson, dock]);
  useEffect(() => {
    const listener = (event: Event) => {
      const detail = (
        event as CustomEvent<{ documentId: string; projectId: string; x: number; y: number }>
      ).detail;
      if (detail?.documentId === documentId && detail.projectId === doc.file.project_id)
        setContext({ x: detail.x, y: detail.y });
    };
    window.addEventListener("asf-editor-context", listener);
    return () => window.removeEventListener("asf-editor-context", listener);
  }, [documentId, doc.file.project_id]);
  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (error) {
      setError(String(error));
    } finally {
      setBusy(false);
    }
  };
  const requireProjectFocus = () => {
    const session = model.session;
    if (
      session?.views.find((view) => view.id === session.activeViewId)?.projectId !==
      doc.file.project_id
    )
      throw new Error(
        "The active project changed. Return to this editor and retry the language action."
      );
  };
  const request = async (method: string, extra: Record<string, unknown> = {}) => {
    if (!connection) throw new Error("Start an approved language server first.");
    const current = model.documents.get(doc.id)!;
    await connection.synchronize();
    await connection.sync(current);
    connection.client.sync();
    const view = editor.current;
    const offset = view?.state.selection.main.head ?? 0;
    const line = current.text.lineAt(offset);
    const result = await connection.request<unknown>(
      method,
      {
        textDocument: { uri: connection.uris.get(doc.id) },
        position: { line: line.number - 1, character: offset - line.from },
        ...extra,
      },
      method === "textDocument/rename"
    );
    requireProjectFocus();
    return result;
  };
  const navigate = async (location: Location) => {
    const uri = location.uri ?? location.targetUri ?? location.location?.uri;
    if (!uri || !connection) return;
    const position =
      location.targetSelectionRange?.start ??
      location.range?.start ??
      location.location?.range?.start;
    const file = await connection.resolve(uri);
    requireProjectFocus();
    await dock.openFile(
      file,
      "within",
      position ? { line: position.line + 1, column: position.character + 1 } : undefined
    );
  };
  const prepare = async (workspaceEdit: unknown) => {
    if (!connection) return false;
    const object = workspaceEdit as {
      changes?: Record<string, unknown>;
      documentChanges?: Array<{ textDocument?: { uri: string } }>;
    };
    const uris = [
      ...Object.keys(object.changes ?? {}),
      ...(object.documentChanges ?? []).flatMap((change) =>
        change.textDocument ? [change.textDocument.uri] : []
      ),
    ];
    if (uris.length > 32) throw new Error("Workspace edit exceeds the 32-file limit.");
    let opened = false;
    for (const uri of uris) {
      const file = await connection.resolve(uri);
      const existing = model.documents.all().find((doc) => fileKey(doc.file) === fileKey(file));
      if (!existing || !connection.uris.has(existing.id)) {
        requireProjectFocus();
        await dock.openFile(file);
        const current = model.documents.all().find((doc) => fileKey(doc.file) === fileKey(file));
        if (current) await connection.sync(current);
        opened = true;
      }
    }
    if (opened) {
      await dock.openFile(doc.file);
      return false;
    }
    const prepared = await connection.edit(workspaceEdit);
    requireProjectFocus();
    setPlans(prepared);
    return true;
  };
  const edit = async (
    method: string,
    extra: Record<string, unknown> = {},
    attempt = 0
  ): Promise<void> => {
    const generation = model.documents.get(doc.id)!.generation;
    const result = await request(method, extra);
    if (model.documents.get(doc.id)?.generation !== generation)
      throw new Error(
        "The buffer changed during the language request. Retry on the current version."
      );
    if (!connection || !result) return;
    const workspaceEdit = Array.isArray(result)
      ? { changes: { [connection.uris.get(doc.id)!]: result } }
      : result;
    if (!(await prepare(workspaceEdit))) {
      if (attempt > 0)
        throw new Error(
          "The server keeps adding affected files. Open the required sources and retry."
        );
      await edit(method, extra, attempt + 1);
    }
  };
  const apply = async () => {
    const pending = plans;
    for (const plan of pending) {
      const disk = await model.documents.api.read(plan.file);
      const open = model.documents.all().find((doc) => fileKey(doc.file) === fileKey(plan.file));
      if (
        !open ||
        disk.revision !== plan.expected_revision ||
        (open &&
          (model.documents.raw(open) !== plan.before ||
            connection?.version(open) !== plan.document_version))
      )
        throw new Error(
          "An affected document changed. No further edits were applied; review a new proposal."
        );
    }
    for (const plan of pending) {
      const open = model.documents.all().find((doc) => fileKey(doc.file) === fileKey(plan.file));
      if (
        !open ||
        model.documents.raw(open) !== plan.before ||
        connection?.version(open) !== plan.document_version
      )
        throw new Error("An affected buffer changed during preflight. Request a new edit.");
    }
    let count = 0;
    try {
      for (const plan of pending) {
        const open = model.documents.all().find((doc) => fileKey(doc.file) === fileKey(plan.file));
        if (open) {
          if (model.documents.raw(open) !== plan.before)
            throw new Error("Buffer changed during apply.");
          model.documents.setText(open.id, plan.after);
        } else throw new Error("An affected editor was closed before apply.");
        count++;
      }
    } catch (error) {
      throw new Error(
        `${count} of ${pending.length} file edits applied. ${String(error)} No automatic retry.`
      );
    }
    setPlans([]);
    model.announce(`${count} language edits applied. Open buffers remain unsaved until Save.`);
  };
  const supported = (key: string) => !!connection?.info.capabilities[key];
  commands.current = {
    rename: () => {
      if (supported("renameProvider")) setRename(true);
    },
    format: () => {
      if (supported("documentFormattingProvider"))
        void run(() =>
          edit("textDocument/formatting", { options: { tabSize: 2, insertSpaces: true } })
        );
    },
    definition: () => {
      if (supported("definitionProvider"))
        void run(async () => {
          const result = await request("textDocument/definition");
          const locations = Array.isArray(result) ? result : result ? [result] : [];
          if (locations[0]) await navigate(locations[0] as Location);
        });
    },
    references: () => {
      if (supported("referencesProvider"))
        void run(async () =>
          setLocations(
            ((await request("textDocument/references", {
              context: { includeDeclaration: true },
            })) as Location[]) ?? []
          )
        );
    },
  };
  const contextEntries: MenuEntry[] = [
    {
      id: "definition",
      label: "Go to definition",
      shortcut: "F12",
      disabled: !supported("definitionProvider"),
      action: commands.current.definition,
    },
    {
      id: "references",
      label: "Find references",
      shortcut: "Shift+F12",
      disabled: !supported("referencesProvider"),
      action: commands.current.references,
    },
    {
      id: "rename",
      label: "Rename symbol",
      shortcut: "F2",
      disabled: !supported("renameProvider"),
      action: commands.current.rename,
    },
    {
      id: "format",
      label: "Format document",
      shortcut: "Alt+Shift+F",
      disabled: !supported("documentFormattingProvider"),
      action: commands.current.format,
    },
    ...(state.value?.extensions ?? [])
      .filter((extension) => extension.enabled && extension.status === "active")
      .flatMap((extension) =>
        (extension.manifest.contributions.commands ?? [])
          .filter((command) => command.context_menu)
          .map((command) => ({
            id: `${extension.manifest.id}:${command.id}`,
            label: command.title,
            action: () => {
              if (command.action === "open-view")
                dock.open("extensionView", {
                  agent: `${extension.manifest.id}/${command.target}`,
                  title: command.title,
                });
              else {
                const formatter = extension.manifest.contributions.formatters?.find(
                  (formatter) => formatter.id === command.target
                );
                const current = model.documents.get(doc.id);
                if (formatter && current)
                  model.documents.setText(
                    current.id,
                    formatText(model.documents.raw(current), formatter.operation)
                  );
              }
            },
          }))
      ),
  ];
  return (
    <section className="ide-intelligence" aria-label="Language intelligence">
      {context && (
        <Menu
          label="Source editor actions"
          entries={contextEntries}
          anchor={context}
          onClose={() => setContext(null)}
          returnFocus={editor.current?.contentDOM}
        />
      )}
      <div className="files-toolbar">
        <span className="ide-status">
          {connection ? (connection.error ? "LSP error" : connection.info.profile) : "Plain editor"}
        </span>
        {!connection ? (
          <>
            <Select
              label="Language server profile"
              value={profile || profiles[0]?.id || ""}
              onChange={(e) => setProfile(e.target.value)}
            >
              <option value="" disabled>
                Select language server
              </option>
              {profiles.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </Select>
            <Button
              disabled={busy || !profiles.length || !!doc.disk?.read_only}
              onClick={() =>
                void run(async () => {
                  const selected = profiles.find((p) => p.id === (profile || profiles[0]?.id));
                  if (selected)
                    await model.language.connect(
                      doc.file.project_id,
                      doc.file.root_id,
                      selected.id,
                      selected.languages
                    );
                })
              }
            >
              Start language server
            </Button>
          </>
        ) : (
          <Button
            tone="quiet"
            disabled={busy}
            onClick={() => void run(() => model.language.stop(connection))}
          >
            Stop LSP
          </Button>
        )}
        <Button tone="quiet" onClick={() => dock.open("settings")}>
          Tool settings
        </Button>
        <Button
          tone="quiet"
          disabled={busy || !supported("definitionProvider")}
          onClick={() =>
            void run(async () => {
              const result = await request("textDocument/definition");
              const locations = Array.isArray(result) ? result : result ? [result] : [];
              if (locations[0]) await navigate(locations[0] as Location);
              else model.announce("No definition returned by this language server.");
            })
          }
        >
          Definition
        </Button>
        <Button
          tone="quiet"
          disabled={busy || !supported("referencesProvider")}
          onClick={() =>
            void run(async () =>
              setLocations(
                ((await request("textDocument/references", {
                  context: { includeDeclaration: true },
                })) as Location[]) ?? []
              )
            )
          }
        >
          References
        </Button>
        <Button
          tone="quiet"
          disabled={busy || !supported("renameProvider")}
          onClick={() => setRename(true)}
        >
          Rename symbol
        </Button>
        <Button
          tone="quiet"
          disabled={busy || !supported("documentFormattingProvider") || !!doc.disk?.read_only}
          onClick={() =>
            void run(() =>
              edit("textDocument/formatting", { options: { tabSize: 2, insertSpaces: true } })
            )
          }
        >
          Format
        </Button>
        <Button tone="quiet" disabled={!connection} onClick={() => dock.open("diagnostics")}>
          Diagnostics
        </Button>
        <Button
          tone="quiet"
          onClick={() => {
            const current = model.documents.get(doc.id);
            if (current)
              model.debug.toggle(
                doc.file,
                current.text.lineAt(editor.current?.state.selection.main.head ?? 0).number
              );
          }}
        >
          Toggle breakpoint
        </Button>
        <Button tone="quiet" onClick={() => dock.open("debug")}>
          Debug
        </Button>
      </div>
      {connection?.error && (
        <InlineAlert tone="warning" title="Language service unavailable">
          {connection.error}
        </InlineAlert>
      )}
      {error && (
        <InlineAlert tone="danger" title="Language operation failed">
          {error}
        </InlineAlert>
      )}
      <details>
        <summary>Symbols, code actions and extension commands</summary>
        <div className="files-toolbar">
          <Input
            label="Workspace symbol query"
            placeholder="Symbol name"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <Button
            disabled={busy || !supported("workspaceSymbolProvider")}
            onClick={() =>
              void run(async () =>
                setLocations(
                  ((await connection?.request("workspace/symbol", { query })) as Location[]) ?? []
                )
              )
            }
          >
            Workspace symbols
          </Button>
          <Button
            disabled={busy || !supported("documentSymbolProvider")}
            onClick={() =>
              void run(async () => {
                const symbols = (await request("textDocument/documentSymbol")) as Array<
                  Location & { selectionRange?: Location["range"] }
                >;
                setLocations(
                  (symbols ?? []).map((symbol) => ({
                    ...symbol,
                    uri: connection?.uris.get(doc.id),
                    range: symbol.selectionRange ?? symbol.range,
                  }))
                );
              })
            }
          >
            Document symbols
          </Button>
          <Button
            disabled={busy || !supported("codeActionProvider")}
            onClick={() =>
              void run(async () => {
                const current = model.documents.get(doc.id)!;
                const offset = editor.current?.state.selection.main.head ?? 0;
                const line = current.text.lineAt(offset);
                const position = { line: line.number - 1, character: offset - line.from };
                const actions = (await request("textDocument/codeAction", {
                  range: { start: position, end: position },
                  context: {
                    diagnostics:
                      connection?.diagnostics.get(connection.uris.get(doc.id) ?? "")?.diagnostics ??
                      [],
                  },
                })) as CodeAction[];
                if (model.documents.get(doc.id)?.generation !== current.generation)
                  throw new Error("The buffer changed. Request code actions again.");
                actionGeneration.current = current.generation;
                setActions((actions ?? []).slice(0, 100));
              })
            }
          >
            Code actions
          </Button>
          {state.value?.extensions
            .filter((e) => e.enabled && e.status === "active")
            .flatMap((extension) =>
              (extension.manifest.contributions.formatters ?? [])
                .filter((formatter) => formatterApplies(formatter.languages, doc.file.path))
                .map((formatter) => (
                  <Button
                    key={`${extension.manifest.id}:${formatter.id}`}
                    disabled={!!doc.disk?.read_only}
                    onClick={() =>
                      model.documents.setText(
                        doc.id,
                        formatText(
                          model.documents.raw(model.documents.get(doc.id)!),
                          formatter.operation
                        )
                      )
                    }
                  >
                    {formatter.title}
                  </Button>
                ))
            )}
        </div>
        {locations.length > 0 && (
          <ul className="ide-list">
            {locations.slice(0, 100).map((location, i) => (
              <li key={i}>
                <Button tone="quiet" onClick={() => void run(() => navigate(location))}>
                  {location.name ?? location.uri ?? location.targetUri ?? `Location ${i + 1}`}
                </Button>
              </li>
            ))}
          </ul>
        )}
      </details>
      <Dialog
        open={actions !== null}
        onClose={() => setActions(null)}
        title="Language code actions"
        description="Choose an edit to review. Server commands are not executed."
      >
        {actions?.length === 0 && <p>No code actions returned for this location.</p>}
        <ul className="ide-list">
          {actions?.map((action, index) => (
            <li key={index}>
              <Button
                tone="quiet"
                disabled={
                  busy ||
                  !!action.disabled ||
                  (!action.edit &&
                    !(
                      connection?.info.capabilities.codeActionProvider as {
                        resolveProvider?: boolean;
                      }
                    )?.resolveProvider)
                }
                onClick={() => {
                  setActions(null);
                  void run(async () => {
                    if (model.documents.get(doc.id)?.generation !== actionGeneration.current)
                      throw new Error("The buffer changed. Request code actions again.");
                    const resolved = action.edit
                      ? action
                      : await connection?.request<CodeAction>("codeAction/resolve", action);
                    if (!resolved?.edit)
                      throw new Error(
                        "This action supplies a server command without a supported text edit."
                      );
                    if (!(await prepare(resolved.edit)))
                      model.announce(
                        "Affected sources were loaded. Request this code action again for current versions."
                      );
                  });
                }}
              >
                {action.title}
              </Button>
              {action.disabled && <small>{action.disabled.reason}</small>}
              {!action.edit && !!action.command && <small>Server command is unsupported</small>}
            </li>
          ))}
        </ul>
      </Dialog>
      <Dialog
        open={rename}
        onClose={() => setRename(false)}
        title="Rename symbol"
        description="The language server proposes edits. Review them before applying."
        footer={
          <>
            <Button tone="quiet" onClick={() => setRename(false)}>
              Cancel
            </Button>
            <Button
              disabled={!newName.trim() || busy}
              onClick={() => {
                setRename(false);
                void run(() => edit("textDocument/rename", { newName }));
              }}
            >
              Request rename
            </Button>
          </>
        }
      >
        <Input
          label="New symbol name"
          value={newName}
          onChange={(e) => setNewName(e.target.value)}
        />
      </Dialog>
      <Dialog
        open={plans.length > 0}
        onClose={() => setPlans([])}
        title="Review language workspace edit"
        description="All affected files are opened and synchronized before requesting the edit. Applying changes creates unsaved buffers. Save or Save All uses P05 revision-aware writes; no file is automatically overwritten."
        footer={
          <>
            <Button tone="quiet" onClick={() => setPlans([])}>
              Cancel
            </Button>
            <Button disabled={busy} onClick={() => void run(apply)}>
              Apply reviewed edits
            </Button>
          </>
        }
      >
        <div className="ide-edit-preview">
          {plans.map((plan) => (
            <section key={fileKey(plan.file)}>
              <h4>{plan.file.path}</h4>
              <pre>{plan.after.slice(0, 12000)}</pre>
              <p>
                {plan.after.length > 12000 ? "Preview limited to 12,000 characters. " : ""}Revision{" "}
                {plan.expected_revision.slice(0, 18)}
              </p>
            </section>
          ))}
        </div>
      </Dialog>
    </section>
  );
}
