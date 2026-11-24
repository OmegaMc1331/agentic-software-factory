import { EditorView, keymap, lineNumbers } from "@codemirror/view";
import { Compartment, EditorSelection, EditorState } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import { javascript } from "@codemirror/lang-javascript";
import { python } from "@codemirror/lang-python";
import { syntaxTree, syntaxHighlighting, defaultHighlightStyle } from "@codemirror/language";
import { diagnosticCount } from "@codemirror/lint";
import { LSPClient, serverDiagnostics, serverCompletion, hoverTooltips } from "@codemirror/lsp-client";
import { MergeView } from "@codemirror/merge";
import type { Adapter, Connection, Document, Project } from "../shared/bridge";

const language = new Compartment();
const protocol = new Compartment();
const views = new Map<EditorView, Document>();
let client: LSPClient | undefined;
let yaml: import("@codemirror/language").LanguageSupport | undefined;
const theme = EditorView.theme({ "&": { height: "100%", backgroundColor: "#161a20", color: "#e0e4eb", fontSize: "13px" }, ".cm-scroller": { overflow: "auto" }, ".cm-gutters": { backgroundColor: "#161a20", color: "#8992a0" }, ".cm-content": { fontFamily: "Consolas, monospace" } }, { dark: true });
function syntax(doc: Document) { return doc.path.endsWith(".ts") ? javascript({ typescript: true }) : doc.path.endsWith(".py") ? python() : doc.path.endsWith(".yaml") && yaml ? yaml : []; }

export const adapter: Adapter = {
  mount(element, doc, changed) {
    const view = new EditorView({ parent: element, state: EditorState.create({ doc: doc.content, extensions: [theme, lineNumbers(), history(), keymap.of([...defaultKeymap, ...historyKeymap]), syntaxHighlighting(defaultHighlightStyle), language.of(syntax(doc)), protocol.of(client && doc.path.endsWith(".ts") ? client.plugin(doc.uri, "typescript") : []), EditorView.contentAttributes.of({ "aria-label": `Code ${doc.path}` }), EditorView.updateListener.of(update => { if (update.docChanged) changed(update.state.doc.toString()); })] }) });
    views.set(view, doc);
    return {
      text: () => view.state.doc.toString(), replace: value => view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: value } }), focus: () => view.focus(),
      state: () => ({ selection: view.state.selection.toJSON(), scrollTop: view.scrollDOM.scrollTop }),
      restore: value => { const state = value as any; if (state?.selection) view.dispatch({ selection: EditorSelection.fromJSON(state.selection) }); if (state?.scrollTop) view.scrollDOM.scrollTop = state.scrollTop; },
      diagnostics: () => Array.from({ length: diagnosticCount(view.state) }, () => ({ source: "real-lsp" })),
      hover: async () => { client?.sync(); return client?.request("textDocument/hover", { textDocument: { uri: doc.uri }, position: { line: 0, character: 17 } }); },
      destroy: () => { views.delete(view); view.destroy(); },
    };
  },
  diff(element, original, modified) {
    const view = new MergeView({ parent: element, a: { doc: original, extensions: [theme, EditorState.readOnly.of(true), EditorView.editable.of(false)] }, b: { doc: modified, extensions: [theme, EditorState.readOnly.of(true), EditorView.editable.of(false)] }, highlightChanges: true, gutter: true });
    return { changes: () => view.chunks.length, snapshot: () => ({ original: view.a.state.doc.toString(), modified: view.b.state.doc.toString() }), focusModified: () => view.b.focus(), destroy: () => view.destroy() };
  },
  async lsp(project: Project, connection: Connection) {
    const handlers = new Set<(message: string) => void>();
    connection.socket.addEventListener("message", event => { const parsed = JSON.parse(event.data); if (parsed.jsonrpc) for (const handler of handlers) handler(event.data); });
    client = new LSPClient({ rootUri: project.uri, timeout: 20000, sanitizeHTML: value => { const node = document.createElement("div"); node.textContent = value; return node.innerHTML; }, extensions: [serverDiagnostics(), serverCompletion(), hoverTooltips()] });
    client.connect({ send: message => connection.socket.send(message), subscribe: handler => handlers.add(handler), unsubscribe: handler => handlers.delete(handler) });
    await client.initializing;
    for (const [view, doc] of views) if (doc.path.endsWith(".ts")) view.dispatch({ effects: protocol.reconfigure(client.plugin(doc.uri, "typescript")) });
  },
  async plugin(active) {
    yaml = active ? (await import("@codemirror/lang-yaml")).yaml() : undefined;
    for (const [view, doc] of views) if (doc.path.endsWith(".yaml")) view.dispatch({ effects: language.reconfigure(syntax(doc)) });
    return { package: "@codemirror/lang-yaml", active: !!yaml };
  },
  pluginEvidence() { return { active: !!yaml, syntaxTrees: [...views].filter(([, doc]) => doc.path.endsWith(".yaml")).map(([view]) => syntaxTree(view.state).toString()) }; },
};

export function scopeProbe(uri: string) { const host = document.createElement("div"); const view = new EditorView({ parent: host, state: EditorState.create({ doc: "export const otherProjectSecret = 1;", extensions: [javascript({ typescript: true })] }) }); return () => { void uri; view.destroy(); }; }
