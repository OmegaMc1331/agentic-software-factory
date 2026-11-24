import { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
import { EditorView, lineNumbers } from "@codemirror/view";
import { MergeView } from "@codemirror/merge";
import { editorTheme } from "./editorTheme";

export function ComparisonView({
  before,
  after,
  line,
  leftLabel = "Compared source version",
  rightLabel = "Local buffer comparison",
}: {
  before: string;
  after: string;
  line?: number;
  leftLabel?: string;
  rightLabel?: string;
}) {
  const host = useRef<HTMLDivElement>(null);
  const merge = useRef<MergeView>();
  useEffect(() => {
    if (!host.current) return;
    const extensions = [
      ...editorTheme,
      lineNumbers(),
      EditorState.readOnly.of(true),
      EditorView.editable.of(false),
    ];
    if (!merge.current) {
      merge.current = new MergeView({
        parent: host.current,
        a: {
          doc: before,
          extensions: [
            ...extensions,
            EditorView.contentAttributes.of({ tabindex: "0", "aria-label": leftLabel }),
          ],
        },
        b: {
          doc: after,
          extensions: [
            ...extensions,
            EditorView.contentAttributes.of({ tabindex: "0", "aria-label": rightLabel }),
          ],
        },
        highlightChanges: true,
        gutter: true,
        diffConfig: { scanLimit: 1000, timeout: 200 },
      });
    } else {
      for (const [editor, text] of [
        [merge.current.a, before],
        [merge.current.b, after],
      ] as const) {
        if (editor.state.doc.toString() !== text)
          editor.dispatch({ changes: { from: 0, to: editor.state.doc.length, insert: text } });
      }
    }
  }, [before, after, leftLabel, rightLabel]);
  useEffect(() => {
    const editor = merge.current?.b;
    if (editor && line)
      editor.dispatch({
        effects: EditorView.scrollIntoView(
          editor.state.doc.line(Math.min(Math.max(line, 1), editor.state.doc.lines)).from,
          { y: "center" }
        ),
      });
  }, [line]);
  useEffect(
    () => () => {
      merge.current?.destroy();
      merge.current = undefined;
    },
    []
  );
  return <div className="files-merge" ref={host} />;
}
