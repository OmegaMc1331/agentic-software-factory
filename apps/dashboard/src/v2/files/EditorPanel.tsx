import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { EditorState, Prec, type Extension } from "@codemirror/state";
import {
  EditorView,
  lineNumbers,
  keymap,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  rectangularSelection,
  crosshairCursor,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  bracketMatching,
  foldGutter,
  foldKeymap,
  indentOnInput,
  indentUnit,
} from "@codemirror/language";
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { searchKeymap, highlightSelectionMatches, openSearchPanel } from "@codemirror/search";
import { Button } from "../design-system/primitives/Controls";
import { Dialog } from "../design-system/primitives/Overlays";
import { InlineAlert, LoadingIndicator } from "../design-system/components/Application";
import { FileBreadcrumb } from "../design-system/patterns/Code";
import type { WorkspaceView } from "../workspace/production/model";
import type { DockSession, WorkspaceModel } from "../workspace/production/controller";
import { editorTheme } from "./editorTheme";
import { language, languageName } from "./languages";
import type { EditorDocument } from "./documents";
import { fileTitle } from "./types";
import { EditorIntelligence } from "../ide/EditorIntelligence";
import { debugExtension } from "../ide/debug";
import {
  usePreferences,
  defaultPreferences,
  type ApplicationPreferences,
} from "../product/preferences";
const editorPreferences = (value: ApplicationPreferences): Extension[] => [
  Prec.high(EditorView.theme({ "&": { fontSize: `${value.editor_font_size}px` } })),
  EditorState.tabSize.of(value.editor_tab_size),
  indentUnit.of(" ".repeat(value.editor_tab_size)),
  ...(value.editor_word_wrap ? [EditorView.lineWrapping] : []),
];

export function EditorPanel({
  view,
  model,
  dock,
}: {
  view: WorkspaceView;
  model: WorkspaceModel;
  dock: DockSession;
}) {
  const manager = model.documents;
  const snapshot = useSyncExternalStore(
    manager.subscribe,
    useCallback(
      () => (view.documentId ? manager.get(view.documentId, view.file) : undefined),
      [manager, view.documentId, view.file]
    )
  );
  const [failure, setFailure] = useState(""),
    [confirm, setConfirm] = useState<"reload" | "resolve" | null>(null);
  const editor = useRef<EditorView>();
  useEffect(() => {
    if (!view.file || !view.documentId) return;
    let active = true;
    void manager.open(view.file, view.documentId).catch((error) => {
      if (active) setFailure(String(error));
    });
    return () => {
      active = false;
    };
  }, [manager, view.documentId, view.file]);
  if (!view.file || !view.documentId)
    return (
      <div className="ws-scroll">
        <h2>Open a source file</h2>
        <p>Select a repository file in Explorer or search for its path.</p>
        <Button onClick={() => dock.open("explorer")}>Open Explorer</Button>
      </div>
    );
  if (failure)
    return (
      <InlineAlert title="File could not open" tone="danger">
        {failure}
      </InlineAlert>
    );
  if (!snapshot || snapshot.status === "loading")
    return <LoadingIndicator label={`Opening ${view.file.path}`} />;
  const document = snapshot;
  const incomplete = document.disk !== null && document.disk.mode !== "text";
  const compare = (comparison: "base" | "disk" | "head") =>
    dock.open("diff", {
      file: document.file,
      projectId: document.file.project_id,
      documentId: document.id,
      comparison,
      title: `Diff · ${fileTitle(document.file.path, document.rootLabel)}`,
    });
  return (
    <div
      className="files-editor"
      data-root-id={document.file.root_id}
      data-document-id={document.id}
    >
      <div
        className="files-toolbar"
        role="toolbar"
        aria-label={`File actions ${document.file.path}`}
      >
        <Button
          disabled={
            !document.dirty ||
            document.saving ||
            !document.disk ||
            document.disk.read_only ||
            document.conflict ||
            document.draftConflict
          }
          onClick={() => void manager.save(document.id)}
        >
          {document.saving ? "Saving…" : "Save"}
        </Button>
        <Button
          tone="quiet"
          onClick={() =>
            void manager.saveAll().then((results) => {
              const failed = results.filter((result) => !result.saved);
              if (failed.length)
                model.setIssue(`${failed.length} file saves failed; their drafts are retained.`);
              else model.announce(`${results.length} files saved.`);
            })
          }
        >
          Save all
        </Button>
        <Button
          tone="quiet"
          disabled={document.disk?.content === null}
          onClick={() => {
            if (editor.current) openSearchPanel(editor.current);
          }}
        >
          Find / replace
        </Button>
        <Button
          tone="quiet"
          disabled={document.disk?.content == null || incomplete}
          onClick={() => compare("disk")}
        >
          Compare disk
        </Button>
        <Button tone="quiet" disabled={incomplete} onClick={() => compare("base")}>
          Compare base
        </Button>
        <Button tone="quiet" disabled={incomplete} onClick={() => compare("head")}>
          Compare HEAD
        </Button>
        <span className="files-toolbar-spacer" />
        <span className="files-root-label" title={document.file.root_id}>
          {document.rootLabel}
        </span>
      </div>
      <div className="files-breadcrumb">
        <FileBreadcrumb path={document.file.path} />
      </div>
      {!incomplete && !document.disk?.read_only && (
        <EditorIntelligence model={model} dock={dock} document={document} editor={editor} />
      )}
      {document.recovered && (
        <InlineAlert title="Recovered unsaved draft" tone="info">
          This buffer came from recovery storage. It has not been written to the repository.
        </InlineAlert>
      )}
      {document.disk?.read_only && (
        <InlineAlert title="Read-only file" tone="info">
          {document.disk.reason === "agent_worktree"
            ? "Managed roots are available for inspection. Manual writes are disabled to preserve execution and review evidence."
            : document.disk.reason === "large_file_preview"
              ? `Only a bounded preview is loaded (${document.disk.size.toLocaleString()} bytes on disk). It cannot be saved.`
              : document.disk.reason === "mixed_line_endings"
                ? "This file contains mixed line endings. Inspection is read only so they cannot be normalized silently."
                : "Binary, unsupported encoding, or filesystem permissions prevent safe text editing."}
        </InlineAlert>
      )}
      {document.error && (
        <InlineAlert
          title={
            document.status === "missing"
              ? "File missing"
              : document.status === "unavailable"
                ? "File unavailable"
                : "File operation failed"
          }
          tone="danger"
        >
          {document.error}
          <Button tone="quiet" onClick={() => void manager.refresh(document.id)}>
            Retry file
          </Button>
        </InlineAlert>
      )}
      {document.recoveryError && (
        <InlineAlert title="Recovery needs attention" tone="warning">
          {document.recoveryError}
          <Button tone="quiet" onClick={() => void manager.persist(document.id)}>
            Retry recovery save
          </Button>
          {document.draftConflict && (
            <Button tone="quiet" onClick={() => void manager.fork(document.id)}>
              Save independent draft
            </Button>
          )}
        </InlineAlert>
      )}
      {document.conflict && (
        <div className="files-conflict" role="alert">
          <strong>Disk changed while this buffer has unsaved edits.</strong>
          <span>The loaded base, current disk and local draft remain separate.</span>
          <div className="files-actions">
            <Button onClick={() => compare("disk")}>Review differences</Button>
            <Button onClick={() => setConfirm("reload")}>Reload disk</Button>
            <Button
              disabled={!document.disk || document.disk.read_only || document.draftConflict}
              onClick={() => setConfirm("resolve")}
            >
              Save resolved version
            </Button>
            <Button tone="quiet" onClick={() => void manager.persist(document.id)}>
              Keep local draft
            </Button>
          </div>
        </div>
      )}
      {document.disk?.content === null ? (
        <div className="ws-scroll">
          <h2>File metadata</h2>
          <dl>
            <dt>Encoding</dt>
            <dd>{document.disk.encoding}</dd>
            <dt>Size</dt>
            <dd>{document.disk.size.toLocaleString()} bytes</dd>
            <dt>Revision</dt>
            <dd className="files-revision">{document.disk.revision}</dd>
          </dl>
        </div>
      ) : (
        <CodeSurface view={view} document={document} model={model} editorRef={editor} />
      )}
      <footer className="files-status" role="status">
        <span>{languageName(document.file.path)}</span>
        <span>
          {document.disk?.encoding ?? "UTF-8"}
          {document.bom ? " BOM" : ""}
        </span>
        <span>{document.newline.toUpperCase()}</span>
        <span>{document.text.length.toLocaleString()} characters</span>
        <span>
          {document.dirty
            ? document.recoverySaved
              ? "Draft backed up · file not saved"
              : "Unsaved changes"
            : document.disk
              ? "Matches loaded disk revision"
              : "Disk unavailable"}
        </span>
      </footer>
      <Dialog
        open={confirm !== null}
        onClose={() => setConfirm(null)}
        title={
          confirm === "reload"
            ? "Discard local changes and reload disk?"
            : "Save this manually resolved version?"
        }
        description={
          confirm === "reload"
            ? "All panes sharing this document will use the current disk version. Your local unsaved buffer will be replaced."
            : "Your current buffer will replace the disk version shown in the comparison. The backend will reject the write if that disk revision changes again."
        }
        footer={
          <>
            <Button tone="quiet" onClick={() => setConfirm(null)}>
              Cancel
            </Button>
            <Button
              onClick={() => {
                if (confirm === "reload") void manager.reloadDisk(document.id);
                else if (document.disk)
                  void manager.saveResolved(document.id, document.disk.revision);
                setConfirm(null);
              }}
            >
              {confirm === "reload" ? "Discard and reload" : "Save resolved version"}
            </Button>
          </>
        }
      />
    </div>
  );
}

function CodeSurface({
  view,
  document,
  model,
  editorRef,
}: {
  view: WorkspaceView;
  document: EditorDocument;
  model: WorkspaceModel;
  editorRef: React.MutableRefObject<EditorView | undefined>;
}) {
  const host = useRef<HTMLDivElement>(null),
    initial = useRef({ view, document }),
    instance = useRef<EditorView>();
  initial.current = { view, document };
  const manager = model.documents;
  const [mounted, setMounted] = useState(0);
  const preferences = usePreferences();
  const preferenceRef = useRef(preferences.value?.preferences ?? defaultPreferences);
  preferenceRef.current = preferences.value?.preferences ?? defaultPreferences;
  useEffect(() => {
    const editor = instance.current;
    if (editor) {
      const pane = manager.pane(document.id, view.id, () => [], initial.current.view.state);
      editor.dispatch({
        effects: pane.preferences.reconfigure(editorPreferences(preferenceRef.current)),
      });
    }
  }, [preferences.value, manager, document.id, view.id, mounted]);
  useEffect(() => {
    const find = (event: Event) => {
      const detail = (event as CustomEvent<{ viewId: string; projectId: string }>).detail;
      if (
        detail?.viewId === view.id &&
        detail.projectId === document.file.project_id &&
        instance.current
      ) {
        openSearchPanel(instance.current);
      }
    };
    window.addEventListener("factory-editor-find", find);
    return () => window.removeEventListener("factory-editor-find", find);
  }, [view.id, document.file.project_id]);
  useEffect(() => {
    const element = host.current!;
    let hide: ReturnType<typeof setTimeout> | undefined,
      scroll: ReturnType<typeof setTimeout> | undefined;
    let disposed = false;
    const create = () => {
      if (instance.current || disposed || element.clientWidth === 0 || element.clientHeight === 0)
        return;
      const doc = manager.get(initial.current.document.id)!;
      const pane = manager.pane(
        doc.id,
        initial.current.view.id,
        (controls) =>
          [
            ...editorTheme,
            lineNumbers(),
            debugExtension(model.debug, doc.file),
            highlightActiveLineGutter(),
            drawSelection(),
            highlightActiveLine(),
            rectangularSelection(),
            crosshairCursor(),
            EditorState.allowMultipleSelections.of(true),
            history({ minDepth: 30 }),
            bracketMatching(),
            foldGutter(),
            indentOnInput(),
            closeBrackets(),
            highlightSelectionMatches(),
            controls.preferences.of(editorPreferences(preferenceRef.current)),
            controls.lineEnding.of(
              EditorState.lineSeparator.of(doc.newline === "crlf" ? "\r\n" : "\n")
            ),
            controls.editable.of([
              EditorState.readOnly.of(!!doc.disk?.read_only),
              EditorView.editable.of(!doc.disk?.read_only),
              EditorView.contentAttributes.of({
                "aria-label": `Source editor ${doc.file.path}`,
                tabindex: "0",
                spellcheck: "false",
              }),
            ]),
            controls.language.of([]),
            controls.ide.of(manager.ideExtensions(doc.id)),
            keymap.of([
              {
                key: "Mod-s",
                run: () => {
                  void manager.save(doc.id);
                  return true;
                },
              },
              {
                key: "Mod-Shift-s",
                run: () => {
                  void manager.saveAll();
                  return true;
                },
              },
              ...defaultKeymap,
              ...historyKeymap,
              ...searchKeymap,
              ...foldKeymap,
              ...closeBracketsKeymap,
              indentWithTab,
            ]),
          ] as Extension[],
        initial.current.view.state
      );
      const editor = new EditorView({
        parent: element,
        state: pane.state,
        dispatchTransactions: (transactions, current) => {
          manager.dispatch(
            initial.current.document.id,
            initial.current.view.id,
            transactions,
            current
          );
          if (transactions.some((transaction) => transaction.docChanged || transaction.selection))
            model.updateView(initial.current.view.sessionId, initial.current.view.id, {
              anchor: current.state.selection.main.anchor,
              head: current.state.selection.main.head,
            });
        },
      });
      instance.current = editor;
      editorRef.current = editor;
      manager.attach(doc.id, initial.current.view.id, editor);
      editor.scrollDOM.scrollTop = initial.current.view.state.scroll ?? 0;
      const saveScroll = () => {
        clearTimeout(scroll);
        scroll = setTimeout(
          () =>
            model.updateView(initial.current.view.sessionId, initial.current.view.id, {
              scroll: editor.scrollDOM.scrollTop,
            }),
          120
        );
      };
      editor.scrollDOM.addEventListener("scroll", saveScroll);
      void language(doc.file.path)
        .then((extension) => {
          if (instance.current === editor)
            editor.dispatch({ effects: pane.language.reconfigure(extension) });
        })
        .catch((error) => model.setIssue(`Syntax support could not load: ${String(error)}`));
      setMounted((value) => value + 1);
    };
    const destroy = () => {
      clearTimeout(scroll);
      const editor = instance.current;
      if (editor) {
        manager.detach(initial.current.document.id, initial.current.view.id);
        editor.destroy();
        instance.current = undefined;
        editorRef.current = undefined;
      }
    };
    const observer = new ResizeObserver(() => {
      if (element.clientWidth && element.clientHeight) {
        clearTimeout(hide);
        create();
      } else {
        clearTimeout(hide);
        hide = setTimeout(destroy, 150);
      }
    });
    observer.observe(element);
    create();
    return () => {
      disposed = true;
      clearTimeout(hide);
      observer.disconnect();
      destroy();
    };
  }, [manager, model, view.id, view.documentId, editorRef]);
  useEffect(() => {
    const editor = instance.current;
    if (!editor) return;
    const pane = manager.pane(document.id, view.id, () => [], initial.current.view.state);
    editor.dispatch({
      effects: [
        pane.editable.reconfigure([
          EditorState.readOnly.of(!!document.disk?.read_only),
          EditorView.editable.of(!document.disk?.read_only),
          EditorView.contentAttributes.of({
            "aria-label": `Source editor ${document.file.path}`,
            tabindex: "0",
            spellcheck: "false",
          }),
        ]),
        pane.lineEnding.reconfigure(
          EditorState.lineSeparator.of(document.newline === "crlf" ? "\r\n" : "\n")
        ),
      ],
    });
  }, [
    document.disk?.read_only,
    document.newline,
    document.id,
    document.file.path,
    manager,
    view.id,
    mounted,
  ]);
  useEffect(() => {
    let active = true;
    const editor = instance.current;
    if (editor) {
      const pane = manager.pane(document.id, view.id, () => [], initial.current.view.state);
      void language(document.file.path).then((extension) => {
        if (active && instance.current === editor)
          editor.dispatch({ effects: pane.language.reconfigure(extension) });
      });
    }
    return () => {
      active = false;
    };
  }, [manager, document.id, document.file.path, view.id, mounted]);
  useEffect(() => {
    const editor = instance.current;
    if (!editor || !view.state.line) return;
    const line = editor.state.doc.line(
      Math.max(1, Math.min(view.state.line, editor.state.doc.lines))
    );
    const anchor =
      line.from +
      [...line.text].slice(0, Math.max(0, (view.state.column ?? 1) - 1)).join("").length;
    editor.dispatch({ selection: { anchor }, scrollIntoView: true });
    editor.focus();
  }, [view.state.jump, view.state.line, view.state.column, mounted]);
  return (
    <div
      className="files-code-surface"
      ref={host}
      onContextMenu={(event) => {
        if (document.disk?.read_only) return;
        event.preventDefault();
        window.dispatchEvent(
          new CustomEvent("asf-editor-context", {
            detail: {
              documentId: document.id,
              projectId: document.file.project_id,
              x: event.clientX,
              y: event.clientY,
            },
          })
        );
      }}
    />
  );
}
