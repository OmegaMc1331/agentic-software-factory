import { ChangeSet, type Extension } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import {
  LSPClient,
  LSPPlugin,
  Workspace,
  type WorkspaceFile,
  type Transport,
  serverCompletion,
  hoverTooltips,
  serverDiagnostics,
  signatureHelp,
} from "@codemirror/lsp-client";
import type { DocumentManager, EditorDocument } from "../files/documents";
import { FactoryApiError } from "../../api";
import type { FileRef } from "../files/types";
import type { WorkspaceModel } from "../workspace/production/controller";
import type { ServerInfo, EditPlan, EventPage } from "./types";
import { ideRequest } from "./api";
import { LspVersions } from "./versions";
import { sanitizeDocumentation } from "./documentation";
import { diagnosticFits } from "./diagnostics";

interface SyncedFile extends WorkspaceFile {
  documentId: string;
  file: FileRef;
}
interface WorkspaceFileUpdate {
  file: WorkspaceFile;
  prevDoc: WorkspaceFile["doc"];
  changes: ChangeSet;
}
export interface DiagnosticSet {
  file: FileRef;
  uri: string;
  version: number;
  versioned: boolean;
  limited: boolean;
  diagnostics: Array<{
    range: { start: { line: number; character: number }; end: { line: number; character: number } };
    message: string;
    severity?: number;
    source?: string;
  }>;
}

export class SharedWorkspace extends Workspace {
  declare files: SyncedFile[];
  constructor(
    client: LSPClient,
    private connection: LanguageConnection
  ) {
    super(client);
    this.files = [];
  }
  openFile(uri: string, languageId: string, view: EditorView) {
    if (this.getFile(uri)) return;
    const document = this.connection.document(uri);
    if (!document) throw new Error("Language document is not owned by this workspace.");
    const file: SyncedFile = {
      uri,
      languageId,
      documentId: document.id,
      file: document.file,
      version: this.connection.version(document),
      doc: document.text,
      getView: () => this.connection.documents.views(document.id)[0] ?? null,
    };
    this.files.push(file);
    this.client.didOpen(file);
    void view;
  }
  closeFile() {}
  syncFiles(): WorkspaceFileUpdate[] {
    const updates: WorkspaceFileUpdate[] = [];
    for (const file of this.files) {
      const document = this.connection.documents.get(file.documentId);
      if (!document || file.doc.eq(document.text)) continue;
      const prevDoc = file.doc;
      const plugins = this.connection.documents
        .views(document.id)
        .map((view) => LSPPlugin.get(view))
        .filter((plugin) => plugin !== null);
      const current = plugins.find(
        (plugin) =>
          plugin.syncedDoc.eq(prevDoc) && plugin.unsyncedChanges.apply(prevDoc).eq(document.text)
      );
      const changes =
        current?.unsyncedChanges ??
        ChangeSet.of({ from: 0, to: prevDoc.length, insert: document.text }, prevDoc.length);
      file.doc = document.text;
      file.version = this.connection.version(document);
      for (const plugin of plugins) plugin.clear();
      updates.push({ file, prevDoc, changes });
    }
    return updates;
  }
  async requestFile(uri: string) {
    const file = await this.connection.resolve(uri);
    const id = await this.connection.documents.open(file);
    const doc = this.connection.documents.get(id)!;
    let existing = this.getFile(uri);
    if (!existing) {
      existing = {
        uri,
        languageId: "plaintext",
        version: this.connection.version(doc),
        doc: doc.text,
        documentId: id,
        file: doc.file,
        getView: () => this.connection.documents.views(id)[0] ?? null,
      } as SyncedFile;
      this.files.push(existing as SyncedFile);
    }
    return existing;
  }
  updateFile() {
    throw new Error("Workspace edits require ASF revision preflight and explicit review.");
  }
  async displayFile(uri: string) {
    const file = await this.connection.resolve(uri);
    await this.connection.navigate(file);
    return this.connection.documents.views(this.connection.document(uri)?.id ?? "")[0] ?? null;
  }
  prune() {
    for (const file of [...this.files])
      if (
        !this.connection.documents.get(file.documentId) ||
        this.connection.uris.get(file.documentId) !== file.uri
      ) {
        this.files.splice(this.files.indexOf(file), 1);
        this.client.didClose(file.uri);
        void this.connection.release(file.documentId, file.file, file.uri).catch((error) => {
          this.connection.error = String(error);
        });
      }
  }
}

export class LanguageConnection {
  readonly client: LSPClient;
  readonly uris = new Map<string, string>();
  readonly diagnostics = new Map<string, DiagnosticSet>();
  private listeners = new Set<(message: string) => void>();
  private cursor = 0;
  private stopped = false;
  private timer?: ReturnType<typeof setTimeout>;
  private synchronized = new Map<string, string>();
  private versions: LspVersions;
  private syncQueues = new Map<string, Promise<void>>();
  private diskRevisions = new Map<string, string>();
  private queue: Promise<unknown> = Promise.resolve();
  private cancelled = new Set<string>();
  error = "";
  constructor(
    readonly info: ServerInfo,
    readonly projectId: string,
    readonly documents: DocumentManager,
    readonly navigate: (file: FileRef, line?: number, column?: number) => Promise<void>,
    private changed: () => void
  ) {
    this.versions = new LspVersions(info.documents);
    this.client = new LSPClient({
      workspace: (client) => new SharedWorkspace(client, this),
      sanitizeHTML: sanitizeDocumentation,
      timeout: 120_000,
      extensions: [serverCompletion(), hoverTooltips(), serverDiagnostics(), signatureHelp()],
    });
    const transport: Transport = {
      send: (message) => this.send(JSON.parse(message) as Record<string, unknown>),
      subscribe: (handler) => this.listeners.add(handler),
      unsubscribe: (handler) => this.listeners.delete(handler),
    };
    this.client.connect(transport);
    this.poll();
  }
  document(uri: string) {
    const id = [...this.uris].find(([, value]) => value === uri)?.[0];
    return id ? this.documents.get(id) : undefined;
  }
  private deliver(message: unknown) {
    const text = JSON.stringify(message);
    for (const listener of this.listeners) listener(text);
  }
  private send(message: Record<string, unknown>) {
    const method = String(message.method ?? "");
    if (method === "$/cancelRequest") {
      const id = String((message.params as { id: unknown }).id);
      this.cancelled.add(id);
      void ideRequest(this.projectId, `lsp/${this.info.id}/cancel`, { request_id: id }).catch(
        () => {}
      );
      return;
    }
    if (method === "initialize") {
      queueMicrotask(() =>
        this.deliver({
          jsonrpc: "2.0",
          id: message.id,
          result: { capabilities: this.info.capabilities },
        })
      );
      return;
    }
    if (
      [
        "initialized",
        "textDocument/didOpen",
        "textDocument/didChange",
        "textDocument/didClose",
      ].includes(method)
    )
      return;
    if (message.id === undefined) return;
    this.queue = this.queue.catch(() => {}).then(() => this.synchronize());
    void this.queue.then(async () => {
      try {
        if (this.cancelled.has(String(message.id))) throw new Error("Request cancelled.");
        const result = await this.request(method, message.params, false, String(message.id));
        this.deliver({ jsonrpc: "2.0", id: message.id, result });
      } catch (error) {
        this.deliver({
          jsonrpc: "2.0",
          id: message.id,
          error: { code: -32603, message: String(error) },
        });
      } finally {
        this.cancelled.delete(String(message.id));
      }
    });
  }
  version(document: EditorDocument) {
    return this.versions.version(document);
  }
  sync(document: EditorDocument, saved = false): Promise<void> {
    const queue = (this.syncQueues.get(document.id) ?? Promise.resolve())
      .catch(() => {})
      .then(async () => {
        const current = this.documents.get(document.id);
        if (current) await this.syncDocument(current, saved);
      });
    this.syncQueues.set(document.id, queue);
    return queue;
  }
  private async syncDocument(document: EditorDocument, saved = false) {
    if (
      document.file.project_id !== this.projectId ||
      document.file.root_id !== this.info.root_id ||
      document.disk?.read_only ||
      document.status !== "ready"
    )
      return;
    const revision = `${document.generation}:${document.baseRevision}`;
    if (!saved && this.synchronized.get(document.id) === revision) return;
    saved =
      saved ||
      (!document.dirty &&
        this.diskRevisions.has(document.id) &&
        this.diskRevisions.get(document.id) !== document.baseRevision);
    const result = await ideRequest<{ uri: string }>(this.projectId, `lsp/${this.info.id}/sync`, {
      document: {
        file: document.file,
        base_revision: document.baseRevision,
        version: this.version(document),
        text: this.documents.raw(document),
        language_id: languageId(document.file.path),
      },
      saved,
    });
    this.uris.set(document.id, result.uri);
    this.synchronized.set(document.id, revision);
    this.diskRevisions.set(document.id, document.baseRevision);
  }
  async synchronize() {
    for (const document of this.documents.all())
      if (this.uris.has(document.id)) await this.sync(document);
    (this.client.workspace as SharedWorkspace).prune();
  }
  async release(id: string, file: FileRef, uri: string) {
    await ideRequest(this.projectId, `lsp/${this.info.id}/close`, { file });
    if (this.uris.get(id) === uri) {
      this.uris.delete(id);
      this.synchronized.delete(id);
      this.syncQueues.delete(id);
      this.diskRevisions.delete(id);
      this.versions.forget(id);
    }
    this.diagnostics.delete(uri);
  }
  async attach(document: EditorDocument): Promise<Extension[]> {
    await this.sync(document);
    const uri = this.uris.get(document.id);
    return uri ? [this.client.plugin(uri, languageId(document.file.path))] : [];
  }
  async request<T = unknown>(
    method: string,
    params: unknown,
    confirmed = false,
    requestId?: string
  ): Promise<T> {
    return ideRequest<T>(this.projectId, `lsp/${this.info.id}/request`, {
      method,
      params,
      confirmed,
      request_id: requestId,
    });
  }
  async resolve(uri: string) {
    return ideRequest<FileRef>(this.projectId, `lsp/${this.info.id}/resolve`, { uri });
  }
  async edit(edit: unknown) {
    return ideRequest<EditPlan[]>(this.projectId, `lsp/${this.info.id}/prepare-edit`, { edit });
  }
  private poll() {
    if (this.stopped) return;
    void ideRequest<EventPage>(this.projectId, `lsp/${this.info.id}/events?cursor=${this.cursor}`)
      .then((page) => {
        if (this.stopped) return;
        this.cursor = page.cursor;
        if (page.gap) {
          this.error = "Language events were truncated. Restart the server to refresh diagnostics.";
          this.diagnostics.clear();
        }
        for (const event of page.events) {
          const message = event.message;
          if (message.method === "textDocument/publishDiagnostics") {
            const params = message.params as {
              uri: string;
              version?: number;
              diagnostics: DiagnosticSet["diagnostics"];
            };
            const document = this.document(params.uri);
            if (
              !document ||
              !Array.isArray(params.diagnostics) ||
              params.diagnostics.some((diagnostic) => !diagnosticFits(document.text, diagnostic)) ||
              (params.version !== undefined && params.version !== this.version(document))
            )
              continue;
            const limited = params.diagnostics.length > 1000;
            params.diagnostics = params.diagnostics.slice(0, 1000);
            const versioned = params.version !== undefined;
            if (!versioned)
              params.diagnostics = params.diagnostics.map((diagnostic) => ({
                ...diagnostic,
                message: `${diagnostic.message}\nServer omitted the document version; freshness cannot be verified.`,
              }));
            this.diagnostics.set(params.uri, {
              file: document.file,
              uri: params.uri,
              version: document.generation,
              versioned,
              limited,
              diagnostics: params.diagnostics,
            });
          }
          this.deliver(message);
        }
        if (!page.alive) {
          this.error = "Language server exited. Text editing and saving remain available.";
          this.stopped = true;
          this.client.disconnect();
        }
        if (page.events.length || page.gap || !page.alive) this.changed();
      })
      .catch((error) => {
        if (error instanceof FactoryApiError && error.status === 404) {
          this.stopped = true;
          this.client.disconnect();
        }
        if (this.error !== String(error)) {
          this.error = String(error);
          this.changed();
        }
      })
      .finally(() => {
        if (!this.stopped) this.timer = setTimeout(() => this.poll(), 350);
      });
  }
  dispose() {
    this.stopped = true;
    clearTimeout(this.timer);
    this.client.disconnect();
    this.listeners.clear();
  }
}

export function languageId(path: string) {
  const extension = path.split(".").at(-1)?.toLowerCase();
  return (
    (
      {
        ts: "typescript",
        tsx: "typescriptreact",
        js: "javascript",
        jsx: "javascriptreact",
        py: "python",
        rs: "rust",
        json: "json",
        html: "html",
        css: "css",
        md: "markdown",
        toml: "toml",
        yaml: "yaml",
        yml: "yaml",
      } as Record<string, string>
    )[extension ?? ""] ?? "plaintext"
  );
}

export class LanguageManager {
  connections = new Map<string, LanguageConnection>();
  private listeners = new Set<() => void>();
  private revision = 0;
  private timer?: ReturnType<typeof setTimeout>;
  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  readonly snapshot = () => this.revision;
  private changed = () => {
    this.revision++;
    for (const listener of this.listeners) listener();
  };
  constructor(private model: WorkspaceModel) {
    model.documents.subscribe(() => {
      clearTimeout(this.timer);
      this.timer = setTimeout(() => {
        for (const connection of this.connections.values()) {
          connection.client.sync();
          void connection.synchronize().catch((error) => {
            connection.error = String(error);
            this.changed();
          });
        }
      }, 180);
    });
  }
  forDocument(document: EditorDocument) {
    return [...this.connections.values()].find(
      (connection) =>
        connection.projectId === document.file.project_id &&
        connection.info.root_id === document.file.root_id &&
        this.profileLanguages
          .get(`${connection.projectId}:${connection.info.profile}`)
          ?.includes(languageId(document.file.path))
    );
  }
  private profileLanguages = new Map<string, string[]>();
  async connect(project: string, root: string, profile: string, languages: string[]) {
    const info = await ideRequest<ServerInfo>(project, "lsp/start", {
      root_id: root,
      profile_id: profile,
      client_id: `${this.model.state.document.id}.${await this.model.documents.clientIdentity()}`,
      confirmed: true,
    });
    const key = `${project}:${info.id}`;
    const existing = this.connections.get(key);
    if (existing && !existing.error) return existing;
    existing?.dispose();
    this.profileLanguages.set(`${project}:${profile}`, languages);
    const connection = new LanguageConnection(
      info,
      project,
      this.model.documents,
      async (file, line, column) => {
        const session = this.model.session;
        const visible = session?.views.find((view) => view.id === session.activeViewId);
        if (visible?.projectId !== project) return;
        await this.model.navigateFile?.(file, line, column);
      },
      this.changed
    );
    this.connections.set(key, connection);
    this.changed();
    return connection;
  }
  async stop(connection: LanguageConnection) {
    try {
      await ideRequest(connection.projectId, `lsp/${connection.info.id}/stop`, {});
    } catch (error) {
      if (!(error instanceof FactoryApiError && error.status === 404)) throw error;
    }
    connection.dispose();
    this.connections.delete(`${connection.projectId}:${connection.info.id}`);
    this.changed();
  }
  dispose() {
    clearTimeout(this.timer);
    for (const connection of this.connections.values()) connection.dispose();
    this.connections.clear();
  }
}
