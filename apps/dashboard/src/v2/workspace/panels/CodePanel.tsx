import { useEffect, useMemo, useRef, useState } from "react";
import { Compartment, EditorState, Transaction } from "@codemirror/state";
import {
  drawSelection,
  EditorView,
  highlightActiveLine,
  keymap,
  lineNumbers,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { search, searchKeymap } from "@codemirror/search";
import { bracketMatching, HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { javascript } from "@codemirror/lang-javascript";
import { python } from "@codemirror/lang-python";
import { rust } from "@codemirror/lang-rust";
import { goToNextChunk, goToPreviousChunk, MergeView, unifiedMergeView } from "@codemirror/merge";
import { tags } from "@lezer/highlight";
import {
  CodeEditorContainer,
  DiffToolbar,
  FileBreadcrumb,
  LanguageLabel,
} from "../../design-system/patterns/Code";
import { Button } from "../../design-system/primitives/Controls";
import { PanelToolbar } from "../../design-system/components/Application";
import { useTheme } from "../../design-system/themes/theme";
import type { WorkspaceController } from "../controller";
import type { FixtureFile } from "../../prototypes/fixtures/data";

const editorTheme = EditorView.theme({
  "&": {
    height: "100%",
    backgroundColor: "var(--code-bg)",
    color: "var(--color-text)",
    fontSize: "var(--font-base)",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-content": {
    fontFamily: "var(--font-code)",
    caretColor: "var(--color-accent)",
    padding: "12px 0",
  },
  ".cm-scroller": { fontFamily: "var(--font-code)", lineHeight: "1.75", overflow: "auto" },
  ".cm-gutters": {
    backgroundColor: "var(--code-gutter)",
    color: "var(--color-text-muted)",
    borderColor: "var(--color-border)",
    fontFamily: "var(--font-code)",
  },
  ".cm-activeLine, .cm-activeLineGutter": { backgroundColor: "var(--color-panel-muted)" },
  ".cm-cursor": { borderLeftColor: "var(--color-accent)" },
  "&.cm-focused .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--color-selection)",
  },
  ".cm-panels": {
    backgroundColor: "var(--color-panel-muted)",
    color: "var(--color-text)",
    borderColor: "var(--color-border)",
  },
  ".cm-textfield, .cm-button": {
    backgroundColor: "var(--color-panel)",
    color: "var(--color-text)",
    border: "1px solid var(--color-control-border)",
    backgroundImage: "none",
  },
  ".cm-searchMatch": {
    backgroundColor: "var(--color-warning-bg)",
    outline: "1px solid var(--color-warning)",
  },
});
const highlighting = HighlightStyle.define([
  { tag: [tags.keyword, tags.controlKeyword, tags.operatorKeyword], color: "var(--code-keyword)" },
  { tag: [tags.string, tags.character], color: "var(--code-string)" },
  { tag: [tags.number, tags.bool], color: "var(--code-number)" },
  { tag: [tags.typeName, tags.className], color: "var(--code-type)" },
  { tag: tags.comment, color: "var(--code-comment)" },
]);
function language(file: FixtureFile) {
  return file.language === "Rust"
    ? rust()
    : file.language === "Python"
      ? python()
      : file.language === "TypeScript"
        ? javascript({ typescript: true })
        : [];
}
function common(file: FixtureFile) {
  return [
    editorTheme,
    lineNumbers(),
    syntaxHighlighting(highlighting),
    language(file),
    bracketMatching(),
  ];
}

export function EditorPanel({
  model,
  path,
  panelId,
}: {
  model: WorkspaceController;
  path: string;
  panelId: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const editor = useRef<EditorView>();
  const { resolved } = useTheme();
  const appearance = useMemo(() => new Compartment(), []);
  const file = model.files.find((item) => item.path === path);
  if (!file) throw new Error("The fixture document is unavailable. Reopen a file from Explorer.");
  useEffect(() => {
    const buffer = model.buffer(path);
    const view = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: buffer.text,
        selection: {
          anchor: Math.min(buffer.anchor, buffer.text.length),
          head: Math.min(buffer.head, buffer.text.length),
        },
        extensions: [
          common(file),
          appearance.of([]),
          history(),
          drawSelection(),
          highlightActiveLine(),
          keymap.of([...defaultKeymap, ...historyKeymap, ...searchKeymap]),
          search({ top: true }),
          EditorView.contentAttributes.of({
            "aria-label": `Fixture code: ${path}`,
            "aria-description":
              "Local preview buffer. No filesystem or language server is connected.",
          }),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) model.updateBuffer(path, update.state.doc.toString(), panelId);
            if (update.selectionSet) {
              buffer.anchor = update.state.selection.main.anchor;
              buffer.head = update.state.selection.main.head;
              model.schedule();
            }
          }),
        ],
      }),
    });
    editor.current = view;
    view.scrollDOM.scrollTop = buffer.scrollTop;
    const unsubscribe = model.subscribeBuffer(path, panelId, (text) => {
      if (text !== view.state.doc.toString())
        view.dispatch({
          changes: { from: 0, to: view.state.doc.length, insert: text },
          annotations: Transaction.addToHistory.of(false),
        });
    });
    const scroll = () => {
      buffer.scrollTop = view.scrollDOM.scrollTop;
      model.schedule();
    };
    view.scrollDOM.addEventListener("scroll", scroll, { passive: true });
    return () => {
      unsubscribe();
      view.scrollDOM.removeEventListener("scroll", scroll);
      view.destroy();
      editor.current = undefined;
    };
  }, [model, path, panelId, appearance, file]);
  useEffect(() => {
    editor.current?.dispatch({
      effects: appearance.reconfigure(EditorView.theme({}, { dark: resolved === "dark" })),
    });
  }, [resolved, appearance]);
  return (
    <CodeEditorContainer label={`Editor ${path}`}>
      <FileBreadcrumb path={path} />
      <div className="v2-editor-meta">
        <LanguageLabel language={file.language} />
        <span>Local fixture buffer</span>
        <Button
          tone="quiet"
          onClick={() => {
            model.persist();
            model.dirty.delete(path);
            model.notify("Preview buffer saved", "Only browser preview state was updated.");
          }}
        >
          Save preview
        </Button>
      </div>
      <div
        className="v2-editor-body"
        ref={host}
        onKeyDown={(event) => {
          if ((event.ctrlKey || event.metaKey) && event.key === "s") {
            event.preventDefault();
            model.persist();
            model.notify("Preview buffer saved", "No project file was written.");
          }
        }}
      />
    </CodeEditorContainer>
  );
}

export function DiffPanel({ model, path }: { model: WorkspaceController; path: string }) {
  const host = useRef<HTMLDivElement>(null);
  const [mode, setMode] = useState<"split" | "unified">("split");
  const { resolved } = useTheme();
  const appearance = useMemo(() => new Compartment(), []);
  const views = useRef<EditorView[]>([]);
  const file = model.files.find((item) => item.path === path);
  if (!file) throw new Error("The fixture diff is unavailable.");
  useEffect(() => {
    const readonly = [
      common(file),
      appearance.of([]),
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
    ];
    if (mode === "split") {
      const merge = new MergeView({
        parent: host.current!,
        a: {
          doc: file.before,
          extensions: [
            ...readonly,
            EditorView.contentAttributes.of({
              "aria-label": "Original fixture version",
              tabindex: "0",
            }),
          ],
        },
        b: {
          doc: model.buffer(path).text,
          extensions: [
            ...readonly,
            EditorView.contentAttributes.of({
              "aria-label": "Modified fixture version",
              tabindex: "0",
            }),
          ],
        },
        gutter: true,
        highlightChanges: true,
      });
      views.current = [merge.a, merge.b];
      return () => {
        merge.destroy();
        views.current = [];
      };
    }
    const view = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: model.buffer(path).text,
        extensions: [
          ...readonly,
          unifiedMergeView({ original: file.before, mergeControls: false }),
          EditorView.contentAttributes.of({ "aria-label": "Unified fixture diff", tabindex: "0" }),
        ],
      }),
    });
    views.current = [view];
    return () => {
      view.destroy();
      views.current = [];
    };
  }, [model, path, mode, file, appearance]);
  useEffect(() => {
    for (const view of views.current)
      view.dispatch({
        effects: appearance.reconfigure(EditorView.theme({}, { dark: resolved === "dark" })),
      });
  }, [resolved, appearance, mode]);
  return (
    <CodeEditorContainer label={`Diff ${path}`}>
      <FileBreadcrumb path={path} />
      <DiffToolbar
        provenance="fixture"
        mode={mode}
        onModeChange={setMode}
        onPrevious={() => {
          const view = views.current.at(-1);
          if (view) {
            view.focus();
            goToPreviousChunk(view);
          }
        }}
        onNext={() => {
          const view = views.current.at(-1);
          if (view) {
            view.focus();
            goToNextChunk(view);
          }
        }}
      />
      <PanelToolbar label="Diff versions">
        <span className="v2-diff-version">Original · fixture</span>
        <span className="v2-diff-version">Modified · fixture</span>
      </PanelToolbar>
      <div className="v2-editor-body v2-diff-body" ref={host} />
    </CodeEditorContainer>
  );
}
