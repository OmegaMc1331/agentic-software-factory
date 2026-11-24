import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags } from "@lezer/highlight";
export const editorTheme = [
  EditorView.theme({
    "&": {
      height: "100%",
      fontSize: "var(--font-base)",
      backgroundColor: "var(--code-bg)",
      color: "var(--color-text)",
    },
    ".cm-scroller": { fontFamily: "var(--font-code)", overflow: "auto", lineHeight: "1.65" },
    ".cm-content": { caretColor: "var(--color-accent)", paddingBlock: "8px" },
    ".cm-cursor,.cm-dropCursor": { borderLeftColor: "var(--color-accent)" },
    ".cm-gutters": {
      backgroundColor: "var(--code-gutter)",
      color: "var(--color-text-muted)",
      borderColor: "var(--color-border)",
    },
    ".cm-activeLine,.cm-activeLineGutter": { backgroundColor: "var(--color-surface-hover)" },
    ".cm-selectionBackground,&.cm-focused .cm-selectionBackground": {
      backgroundColor: "var(--color-selection) !important",
    },
    ".cm-matchingBracket": {
      backgroundColor: "var(--color-selection)",
      outline: "1px solid var(--color-accent)",
    },
    ".cm-searchMatch": {
      backgroundColor: "var(--color-warning-bg)",
      outline: "1px solid var(--color-warning)",
    },
    ".cm-searchMatch-selected": {
      backgroundColor: "var(--color-selection)",
      outline: "1px solid var(--color-focus)",
    },
    ".cm-panels,.cm-tooltip": {
      backgroundColor: "var(--color-panel-muted)",
      color: "var(--color-text)",
      borderColor: "var(--color-border)",
    },
    ".cm-panel input,.cm-panel button": {
      font: "inherit",
      color: "var(--color-text)",
      backgroundColor: "var(--color-panel)",
      border: "1px solid var(--color-control-border)",
    },
    ".cm-panel .cm-button": {
      backgroundImage: "none",
      borderRadius: "var(--radius-control)",
      minHeight: "26px",
      padding: "2px 8px",
    },
    ".cm-panel.cm-search": { padding: "6px 36px 6px 6px" },
    '.cm-panel button[name="close"]': {
      minWidth: "26px",
      minHeight: "26px",
      top: "4px",
      right: "4px",
    },
    ".cm-panel button:hover": { backgroundColor: "var(--color-surface-hover)" },
    ".cm-panel button:focus-visible,.cm-panel input:focus-visible": {
      outline: "2px solid var(--color-focus)",
      outlineOffset: "2px",
    },
  }),
  syntaxHighlighting(
    HighlightStyle.define([
      { tag: [tags.keyword, tags.modifier, tags.operatorKeyword], color: "var(--code-keyword)" },
      { tag: [tags.comment, tags.meta], color: "var(--code-comment)" },
      { tag: [tags.string, tags.regexp], color: "var(--code-string)" },
      { tag: [tags.number, tags.bool, tags.null], color: "var(--code-number)" },
      {
        tag: [tags.function(tags.variableName), tags.typeName, tags.tagName],
        color: "var(--code-type)",
      },
    ])
  ),
];
