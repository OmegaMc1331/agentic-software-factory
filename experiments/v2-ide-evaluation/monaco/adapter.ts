import * as monaco from "monaco-editor";
import EditorWorker from "monaco-editor/editor/editor.worker?worker";
import YamlWorker from "monaco-yaml/yaml.worker?worker";
import type { Adapter, Connection, Document, Project } from "../shared/bridge";
import { waitFor } from "../shared/bridge";

(self as any).MonacoEnvironment = { getWorker(_id: string, label: string) { return label === "yaml" ? new YamlWorker() : new EditorWorker(); } };
monaco.editor.setTheme("vs-dark");
const externalLanguageServices = { completionItems: false, hovers: false, documentSymbols: false, definitions: false, references: false, documentHighlights: false, rename: false, diagnostics: false, documentRangeFormattingEdits: false, signatureHelp: false, onTypeFormattingEdits: false, codeActions: false, inlayHints: false };
monaco.typescript.typescriptDefaults.setModeConfiguration(externalLanguageServices);
monaco.typescript.javascriptDefaults.setModeConfiguration(externalLanguageServices);
let yaml: { dispose(): void } | undefined;
const editors = new Set<monaco.editor.IStandaloneCodeEditor>();
const connections: object[] = [];

export const adapter: Adapter = {
  mount(element, doc, changed) {
    const uri = monaco.Uri.parse(doc.uri);
    let model = monaco.editor.getModel(uri);
    if (!model) model = monaco.editor.createModel(doc.content, doc.path.endsWith(".ts") ? "typescript" : doc.path.endsWith(".py") ? "python" : doc.path.endsWith(".yaml") ? "yaml" : "plaintext", uri);
    const editor = monaco.editor.create(element, { model, automaticLayout: true, minimap: { enabled: false }, fontSize: 13, scrollBeyondLastLine: false, wordWrap: "off", ariaLabel: `Code ${doc.path}`, quickSuggestions: false });
    editors.add(editor);
    const listener = model.onDidChangeContent(() => changed(model!.getValue()));
    return {
      text: () => model!.getValue(), replace: value => model!.setValue(value), focus: () => editor.focus(),
      state: () => editor.saveViewState(), restore: value => editor.restoreViewState(value as monaco.editor.ICodeEditorViewState),
      diagnostics: () => monaco.editor.getModelMarkers({ resource: uri }),
      hover: async () => { editor.setPosition({ lineNumber: 1, column: 17 }); await editor.getAction("editor.action.showHover")?.run(); return waitFor(() => element.querySelector(".monaco-hover")?.textContent); },
      destroy: () => { listener.dispose(); editors.delete(editor); editor.dispose(); },
    };
  },
  diff(element, original, modified) {
    const left = monaco.editor.createModel(original, "typescript");
    const right = monaco.editor.createModel(modified, "typescript");
    const editor = monaco.editor.createDiffEditor(element, { automaticLayout: true, readOnly: true, originalEditable: false, renderSideBySide: true, minimap: { enabled: false } });
    editor.setModel({ original: left, modified: right });
    return { changes: () => editor.getLineChanges()?.length ?? 0, snapshot: () => ({ original: left.getValue(), modified: right.getValue() }), focusModified: () => editor.getModifiedEditor().focus(), destroy: () => { editor.dispose(); left.dispose(); right.dispose(); } };
  },
  async lsp(_project: Project, connection: Connection) {
    const transport = monaco.lsp.WebSocketTransport.fromWebSocket(connection.socket);
    const client = new monaco.lsp.MonacoLspClient(transport);
    connections.push(client, transport);
    await waitFor(() => connection.messages.find(m => m.result?.capabilities));
  },
  async plugin(active) {
    yaml?.dispose(); yaml = undefined;
    if (active) {
      const { configureMonacoYaml } = await import("monaco-yaml");
      yaml = configureMonacoYaml(monaco, { enableSchemaRequest: false, schemas: [], validate: true, hover: true, completion: true }) as unknown as { dispose(): void };
    }
    return { package: "monaco-yaml", active: !!yaml };
  },
  pluginEvidence() {
    return { active: !!yaml, tokens: monaco.editor.tokenize("project: ASF\n", "yaml").flat().map(token => token.type), markers: monaco.editor.getModels().filter(m => m.getLanguageId() === "yaml").flatMap(m => monaco.editor.getModelMarkers({ resource: m.uri })), models: monaco.editor.getModels().length };
  },
};

export function scopeProbe(uri: string) { const model = monaco.editor.createModel("export const otherProjectSecret = 1;", "typescript", monaco.Uri.parse(uri)); return () => model.dispose(); }
