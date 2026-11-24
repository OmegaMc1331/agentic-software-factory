import { useEffect, useRef } from "react";
import { EditorState, Compartment } from "@codemirror/state";
import { EditorView, lineNumbers, keymap } from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { useTheme } from "../../design-system/themes/theme";
import { Button } from "../../design-system/primitives/Controls";
import type { WorkspaceView } from "./model";
import type { WorkspaceModel } from "./controller";
export default function ScratchNote({
  view,
  model,
}: {
  view: WorkspaceView;
  model: WorkspaceModel;
}) {
  const host = useRef<HTMLDivElement>(null),
    editor = useRef<EditorView>(),
    appearance = useRef(new Compartment());
  const initial = useRef(view),
    { resolved } = useTheme();
  useEffect(() => {
    const state = initial.current.state,
      text = state.draft ?? "";
    const instance = new EditorView({
      parent: host.current!,
      state: EditorState.create({
        doc: text,
        selection: {
          anchor: Math.min(state.anchor ?? 0, text.length),
          head: Math.min(state.head ?? 0, text.length),
        },
        extensions: [
          EditorState.changeFilter.of((transaction) => {
            if (transaction.newDoc.length <= 65536) return true;
            queueMicrotask(() =>
              model.setIssue(
                "Workspace notes are limited to 65,536 characters. The insertion was refused; the existing draft is unchanged."
              )
            );
            return false;
          }),
          history(),
          keymap.of([...defaultKeymap, ...historyKeymap]),
          lineNumbers(),
          appearance.current.of(
            EditorView.theme({}, { dark: document.documentElement.dataset.asfTheme === "dark" })
          ),
          EditorView.theme({
            "&": {
              height: "100%",
              backgroundColor: "var(--code-bg)",
              color: "var(--color-text)",
              fontSize: "var(--font-base)",
            },
            ".cm-scroller": { fontFamily: "var(--font-code)", overflow: "auto" },
            ".cm-gutters": {
              backgroundColor: "var(--code-gutter)",
              color: "var(--color-text-muted)",
              borderColor: "var(--color-border)",
            },
            ".cm-content": { caretColor: "var(--color-accent)" },
            ".cm-cursor": { borderLeftColor: "var(--color-accent)" },
            ".cm-selectionBackground": { backgroundColor: "var(--color-selection)" },
          }),
          EditorView.contentAttributes.of({
            "aria-label": "Workspace note; stored in workspace, never written to repository",
          }),
          EditorView.updateListener.of((update) => {
            if (update.docChanged || update.selectionSet) {
              const draft = update.state.doc.toString();
              if (draft.length > 65536) {
                model.setIssue(
                  "A workspace note is limited to 65,536 characters. The previous recovery snapshot is retained."
                );
                return;
              }
              model.updateView(initial.current.sessionId, initial.current.id, {
                draft,
                anchor: update.state.selection.main.anchor,
                head: update.state.selection.main.head,
                ...(update.docChanged ? { dirty: true } : {}),
              });
            }
          }),
        ],
      }),
    });
    editor.current = instance;
    instance.scrollDOM.scrollTop = state.scroll ?? 0;
    const scroll = () =>
      model.updateView(initial.current.sessionId, initial.current.id, {
        scroll: instance.scrollDOM.scrollTop,
      });
    instance.scrollDOM.addEventListener("scroll", scroll);
    return () => {
      instance.scrollDOM.removeEventListener("scroll", scroll);
      instance.destroy();
      editor.current = undefined;
    };
  }, [model]);
  useEffect(() => {
    editor.current?.dispatch({
      effects: appearance.current.reconfigure(EditorView.theme({}, { dark: resolved === "dark" })),
    });
  }, [resolved]);
  return (
    <div className="ws-note">
      <div className="ws-note-toolbar">
        <span>Workspace note · no repository file access</span>
        <Button
          onClick={() => {
            model.updateView(view.sessionId, view.id, { dirty: false });
            void model.save();
          }}
        >
          Keep draft in workspace
        </Button>
      </div>
      <div ref={host} className="ws-note-editor" />
    </div>
  );
}
