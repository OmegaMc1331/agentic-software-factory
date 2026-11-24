import {
  Compartment,
  EditorState,
  Text,
  Transaction,
  type Extension,
  type ChangeSet,
} from "@codemirror/state";
import type { EditorView } from "@codemirror/view";
import { FactoryApiError } from "../../api";
import { FileApi, filesChanged } from "./fileApi";
import { BrowserDraftJournal, type DraftJournal, type LocalDraft } from "./journal";
import {
  fileKey,
  token,
  validDraft,
  validFile,
  type Draft,
  type DraftEnvelope,
  type FileRef,
  type FileSnapshot,
  type MoveResult,
  type RootCatalog,
} from "./types";

export interface EditorDocument {
  id: string;
  file: FileRef;
  status: "loading" | "ready" | "missing" | "unavailable";
  text: Text;
  baseText: Text;
  baseRevision: string;
  newline: string;
  bom: boolean;
  disk: FileSnapshot | null;
  rootLabel: string;
  dirty: boolean;
  conflict: boolean;
  draftConflict: boolean;
  saving: boolean;
  draftSaving: boolean;
  recoverySaved: boolean;
  recovered: boolean;
  error: string;
  recoveryError: string;
  generation: number;
}
export interface Pane {
  state: EditorState;
  view?: EditorView;
  language: Compartment;
  editable: Compartment;
  lineEnding: Compartment;
  ide: Compartment;
  preferences: Compartment;
}
interface RecordState {
  contextPane?: string;
  snapshot: EditorDocument;
  panes: Map<string, Pane>;
  listeners: Set<() => void>;
  draftRevision: number;
  readEpoch: number;
  timer?: ReturnType<typeof setTimeout>;
  draftPromise?: Promise<void>;
  savePromise?: Promise<boolean>;
  local: Promise<void>;
  loaded?: Promise<string>;
  ideExtensions?: Extension[];
}
export interface DocumentHost {
  changed(document: EditorDocument, previousId?: string, previousFile?: FileRef): void;
  issue(message: string): void;
}
const textFrom = (content: string, newline: string) =>
  Text.of(content.split(newline === "crlf" ? "\r\n" : "\n"));
const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const isMissing = (error: unknown) => error instanceof FactoryApiError && error.status === 404;

export class DocumentManager {
  private records = new Map<string, RecordState>();
  private byFile = new Map<string, string>();
  private aliases = new Map<string, string>();
  private listeners = new Set<() => void>();
  private catalogs = new Map<string, Promise<RootCatalog>>();
  private pending: Array<() => void> = [];
  private active = 0;
  private stopped = false;
  private polling = false;
  private pollOffset = 0;
  private interval?: ReturnType<typeof setInterval>;
  private owner: string;
  readonly journal: DraftJournal;
  constructor(
    private host: DocumentHost,
    readonly api = new FileApi(),
    journal?: DraftJournal
  ) {
    try {
      this.owner = sessionStorage.getItem("asf.v2.editor.window") ?? token("window");
      sessionStorage.setItem("asf.v2.editor.window", this.owner);
    } catch {
      this.owner = token("window");
    }
    this.journal = journal ?? new BrowserDraftJournal(this.owner);
  }
  async clientIdentity() {
    return (await this.journal.identity?.()) ?? this.owner;
  }
  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };
  private resolve(id: string) {
    let next = id;
    for (let i = 0; i < 128 && this.aliases.has(next); i++) next = this.aliases.get(next)!;
    return next;
  }
  private matches(record: RecordState, file: FileRef) {
    const current = record.snapshot.file;
    return (
      current.project_id === file.project_id &&
      current.root_id === file.root_id &&
      (current.path === file.path ||
        this.resolve(this.byFile.get(fileKey(file)) ?? "") === record.snapshot.id)
    );
  }
  private forgetFileAliases(id: string) {
    for (const [key, value] of this.byFile) if (this.resolve(value) === id) this.byFile.delete(key);
  }
  get(id: string, file?: FileRef) {
    const record = this.records.get(this.resolve(id));
    return record && (!file || this.matches(record, file)) ? record.snapshot : undefined;
  }
  contextSelection(id: string) {
    const record = this.records.get(this.resolve(id));
    const panes = [...(record?.panes.values() ?? [])];
    return (
      panes.find((pane) => pane.view?.hasFocus) ??
      record?.panes.get(record.contextPane ?? "") ??
      panes[0]
    )?.state.selection.main;
  }
  all() {
    return [...this.records.values()].map((record) => record.snapshot);
  }
  subscribeDocument(id: string, listener: () => void) {
    const record = this.records.get(this.resolve(id));
    if (!record) return () => {};
    record.listeners.add(listener);
    return () => record.listeners.delete(listener);
  }
  private publish(record: RecordState, patch: Partial<EditorDocument>, previousId?: string) {
    if (this.stopped) return;
    const before = record.snapshot;
    record.snapshot = { ...before, ...patch };
    for (const listener of record.listeners) listener();
    for (const listener of this.listeners) listener();
    if (
      before.dirty !== record.snapshot.dirty ||
      fileKey(before.file) !== fileKey(record.snapshot.file) ||
      previousId
    )
      this.host.changed(
        record.snapshot,
        previousId,
        fileKey(before.file) === fileKey(record.snapshot.file) ? undefined : before.file
      );
  }
  private async io<T>(operation: () => Promise<T>): Promise<T> {
    if (this.stopped) throw new Error("Editor document service was closed.");
    if (this.active >= 2) await new Promise<void>((resolve) => this.pending.push(resolve));
    else this.active++;
    try {
      if (this.stopped) throw new Error("Editor document service was closed.");
      return await operation();
    } finally {
      const next = this.pending.shift();
      if (next) next();
      else this.active--;
    }
  }
  private catalog(projectId: string) {
    let promise = this.catalogs.get(projectId);
    if (!promise) {
      promise = this.io(() => this.api.roots(projectId));
      this.catalogs.set(projectId, promise);
      promise.catch(() => this.catalogs.delete(projectId));
    }
    return promise;
  }
  async open(file: FileRef, id?: string): Promise<string> {
    if (!validFile(file)) throw new Error("Invalid editor file identity.");
    const existing = this.records.get(this.resolve(id ?? this.byFile.get(fileKey(file)) ?? ""));
    if (existing) {
      if (!this.matches(existing, file))
        throw new Error(
          "This editor document identity belongs to a different file. Its existing buffer was retained."
        );
      return existing.loaded ?? existing.snapshot.id;
    }
    if (this.records.size >= 64)
      throw new Error(
        "This window already holds 64 documents. Retain dirty drafts and close clean documents before opening more."
      );
    const documentId = id ?? this.byFile.get(fileKey(file)) ?? token();
    if (!/^d_[a-f0-9]{32}$/.test(documentId)) throw new Error("Invalid editor document identity.");
    const empty = Text.empty;
    const record: RecordState = {
      snapshot: {
        id: documentId,
        file,
        status: "loading",
        text: empty,
        baseText: empty,
        baseRevision: "",
        newline: "lf",
        bom: false,
        disk: null,
        rootLabel: "Repository",
        dirty: false,
        conflict: false,
        draftConflict: false,
        saving: false,
        draftSaving: false,
        recoverySaved: false,
        recovered: false,
        error: "",
        recoveryError: "",
        generation: 0,
      },
      panes: new Map(),
      listeners: new Set(),
      draftRevision: 0,
      readEpoch: 0,
      local: Promise.resolve(),
    };
    this.records.set(documentId, record);
    this.byFile.set(fileKey(file), documentId);
    if (!this.interval) this.interval = setInterval(() => void this.poll(), 2000);
    record.loaded = this.load(record).finally(() => {
      record.loaded = undefined;
    });
    return record.loaded;
  }
  private async load(record: RecordState) {
    const file = record.snapshot.file,
      id = record.snapshot.id;
    let local: LocalDraft | undefined,
      server: DraftEnvelope | undefined,
      disk: FileSnapshot | undefined,
      recoveryError = "",
      error = "",
      rootLabel = "Repository";
    try {
      local = await this.journal.read(id);
    } catch (reason) {
      recoveryError = errorText(reason);
    }
    try {
      const catalog = await this.catalog(file.project_id);
      rootLabel =
        catalog.roots.find((root) => root.id === file.root_id)?.label ?? "Unavailable root";
    } catch (reason) {
      if (!isMissing(reason)) recoveryError = errorText(reason);
    }
    try {
      server = await this.io(() => this.api.draft(file, id));
    } catch (reason) {
      if (!isMissing(reason)) recoveryError = errorText(reason);
    }
    try {
      disk = await this.io(() => this.api.read(file));
    } catch (reason) {
      error = errorText(reason);
    }
    const draftConflict = !!(
      local &&
      server &&
      local.revision !== server.revision &&
      local.document.content !== server.document.content
    );
    const candidate =
      local && (draftConflict || !server || local.revision === server.revision) ? local : server;
    const draft = candidate?.document;
    if (
      draft &&
      (!validDraft(draft) ||
        draft.file.project_id !== file.project_id ||
        draft.file.root_id !== file.root_id ||
        draft.file.path !== file.path)
    ) {
      this.publish(record, {
        status: "unavailable",
        error:
          "The recovery record belongs to a different file or is invalid. It was retained without being applied.",
      });
      return id;
    }
    const recovered = draft?.dirty === true;
    const newline = recovered ? draft.newline : (disk?.newline ?? "lf");
    const content = recovered ? draft.content : (disk?.content ?? "");
    const baseContent = recovered ? draft.base_content : (disk?.content ?? "");
    const canonical = disk?.file ?? file;
    const canonicalId = this.byFile.get(fileKey(canonical));
    if (canonicalId && canonicalId !== id && !recovered) {
      const other = this.records.get(canonicalId);
      if (other) {
        if (other.loaded) await other.loaded;
        this.records.delete(id);
        this.aliases.set(id, canonicalId);
        this.byFile.set(fileKey(file), canonicalId);
        this.host.changed(other.snapshot, id);
        for (const listener of this.listeners) listener();
        return canonicalId;
      }
    }
    this.byFile.set(fileKey(file), id);
    this.byFile.set(fileKey(canonical), id);
    record.draftRevision = server?.revision ?? 0;
    this.publish(record, {
      file: canonical,
      status: disk ? "ready" : recovered ? "missing" : "unavailable",
      text: textFrom(content, newline),
      baseText: textFrom(baseContent, newline),
      baseRevision: recovered ? draft.base_revision : (disk?.revision ?? ""),
      newline,
      bom: recovered ? draft.bom : (disk?.bom ?? false),
      disk: disk ?? null,
      rootLabel,
      dirty: recovered,
      conflict: recovered && !!disk && draft.base_revision !== disk.revision,
      draftConflict,
      recovered,
      error,
      recoveryError: draftConflict
        ? "Another window saved a different recovery revision. Save an independent draft before editing or saving."
        : recoveryError,
      recoverySaved:
        !!server && !!draft && !draftConflict && server.document.content === draft.content,
    });
    this.host.changed(record.snapshot);
    return id;
  }
  raw(document: EditorDocument) {
    return document.text.sliceString(
      0,
      document.text.length,
      document.newline === "crlf" ? "\r\n" : "\n"
    );
  }
  private draft(record: RecordState): Draft {
    const document = record.snapshot;
    return {
      schema: 1,
      id: document.id,
      file: document.file,
      base_revision: document.baseRevision,
      base_content: document.dirty
        ? document.baseText.sliceString(
            0,
            document.baseText.length,
            document.newline === "crlf" ? "\r\n" : "\n"
          )
        : "",
      content: document.dirty ? this.raw(document) : "",
      newline: document.newline,
      bom: document.bom,
      dirty: document.dirty,
    };
  }
  pane(
    id: string,
    viewId: string,
    extensions: (
      controls: Pick<Pane, "language" | "editable" | "lineEnding" | "ide" | "preferences">
    ) => Extension[],
    selection: { anchor?: number; head?: number }
  ) {
    const record = this.records.get(this.resolve(id));
    if (!record) throw new Error("Document has not loaded.");
    let pane = record.panes.get(viewId);
    if (!pane) {
      const document = record.snapshot;
      const controls = {
        language: new Compartment(),
        editable: new Compartment(),
        lineEnding: new Compartment(),
        ide: new Compartment(),
        preferences: new Compartment(),
      };
      pane = {
        ...controls,
        state: EditorState.create({
          doc: document.text,
          selection: {
            anchor: Math.min(selection.anchor ?? 0, document.text.length),
            head: Math.min(selection.head ?? selection.anchor ?? 0, document.text.length),
          },
          extensions: extensions(controls),
        }),
      };
      record.panes.set(viewId, pane);
    }
    return pane;
  }
  attach(id: string, viewId: string, view: EditorView) {
    const record = this.records.get(this.resolve(id));
    const pane = record?.panes.get(viewId);
    if (pane) pane.view = view;
  }
  views(id: string) {
    return [...(this.records.get(this.resolve(id))?.panes.values() ?? [])].flatMap((pane) =>
      pane.view ? [pane.view] : []
    );
  }
  configureIde(id: string, extensions: Extension[]) {
    const record = this.records.get(this.resolve(id));
    if (!record) return;
    record.ideExtensions = extensions;
    for (const pane of record.panes.values()) {
      const transaction = pane.state.update({ effects: pane.ide.reconfigure(extensions) });
      if (pane.view) pane.view.dispatch(transaction);
      else pane.state = transaction.state;
    }
  }
  ideExtensions(id: string) {
    return this.records.get(this.resolve(id))?.ideExtensions ?? [];
  }
  detach(id: string, viewId: string) {
    const pane = this.records.get(this.resolve(id))?.panes.get(viewId);
    if (pane?.view) {
      pane.state = pane.view.state;
      pane.view = undefined;
    }
  }
  dispatch(id: string, viewId: string, transactions: readonly Transaction[], view: EditorView) {
    const record = this.records.get(this.resolve(id));
    if (!record) return;
    if (view.hasFocus) record.contextPane = viewId;
    const document = record.snapshot;
    if (transactions.some((transaction) => transaction.docChanged) && document.disk?.read_only) {
      this.host.issue("This file is read only. Its buffer was not changed.");
      return;
    }
    const final = transactions.at(-1)?.state;
    if (final && final.doc.length > 1_048_576) {
      this.host.issue(
        "The editor size limit was reached. This insertion was refused; the previous draft remains."
      );
      return;
    }
    view.update(transactions);
    const origin = record.panes.get(viewId);
    if (origin) origin.state = view.state;
    for (const transaction of transactions)
      if (transaction.docChanged) this.broadcast(record, transaction.changes, viewId);
    if (transactions.some((transaction) => transaction.docChanged)) {
      const text = view.state.doc,
        dirty = !text.eq(record.snapshot.baseText);
      this.publish(record, {
        text,
        dirty,
        generation: document.generation + 1,
        recoverySaved: false,
        error: "",
      });
      this.schedule(record);
    }
  }
  private broadcast(
    record: RecordState,
    changes: ChangeSet,
    origin?: string,
    preserveSelection = false
  ) {
    for (const [id, pane] of record.panes)
      if (id !== origin) {
        const transaction = pane.state.update({
          changes,
          ...(preserveSelection
            ? {
                selection: {
                  anchor: Math.min(pane.state.selection.main.anchor, changes.newLength),
                  head: Math.min(pane.state.selection.main.head, changes.newLength),
                },
              }
            : {}),
          annotations: [Transaction.addToHistory.of(false), Transaction.remote.of(true)],
        });
        if (pane.view) pane.view.update([transaction]);
        pane.state = transaction.state;
      }
  }
  private replace(record: RecordState, text: Text) {
    const current = record.snapshot.text;
    const changes = EditorState.create({ doc: current }).changes({
      from: 0,
      to: current.length,
      insert: text,
    });
    this.broadcast(record, changes, undefined, true);
  }
  setText(id: string, content: string) {
    const record = this.records.get(this.resolve(id));
    if (!record) return;
    if (record.snapshot.disk?.read_only) {
      this.host.issue("This document is read only.");
      return;
    }
    if (content.length > 1_048_576) {
      this.host.issue("The editor size limit was reached.");
      return;
    }
    const text = textFrom(content, record.snapshot.newline);
    this.replace(record, text);
    this.publish(record, {
      text,
      dirty: !text.eq(record.snapshot.baseText),
      generation: record.snapshot.generation + 1,
      recoverySaved: false,
    });
    this.schedule(record);
  }
  private schedule(record: RecordState) {
    this.persistLocal(record);
    clearTimeout(record.timer);
    record.timer = setTimeout(() => void this.flush(record), 300);
  }
  private persistLocal(record: RecordState) {
    const generation = record.snapshot.generation,
      document = this.draft(record),
      revision = record.draftRevision;
    record.local = record.local
      .catch(() => {})
      .then(() =>
        this.journal.write({ owner: this.owner, updated: Date.now(), document, revision })
      )
      .then(() => {
        if (record.snapshot.generation === generation)
          this.publish(record, {
            recoveryError: record.snapshot.draftConflict ? record.snapshot.recoveryError : "",
          });
      })
      .catch((error) =>
        this.publish(record, { recoveryError: errorText(error), recoverySaved: false })
      );
  }
  private async flush(record: RecordState) {
    if (record.draftPromise) return record.draftPromise;
    if (record.snapshot.draftConflict || this.stopped) return;
    const generation = record.snapshot.generation,
      document = this.draft(record);
    this.publish(record, { draftSaving: true });
    record.draftPromise = (async () => {
      try {
        const saved = await this.io(() => this.api.saveDraft(document, record.draftRevision));
        record.draftRevision = saved.revision;
        const current = record.snapshot.generation === generation;
        this.publish(record, { draftSaving: false, recoverySaved: current, recoveryError: "" });
        this.persistLocal(record);
        if (!current) record.timer = setTimeout(() => void this.flush(record), 300);
      } catch (error) {
        this.publish(record, {
          draftSaving: false,
          recoverySaved: false,
          draftConflict: error instanceof FactoryApiError && error.status === 409,
          recoveryError: errorText(error),
        });
      } finally {
        record.draftPromise = undefined;
      }
    })();
    return record.draftPromise;
  }
  async persist(id: string) {
    const record = this.records.get(this.resolve(id));
    if (record) {
      clearTimeout(record.timer);
      await this.flush(record);
      await record.local;
    }
  }
  async save(id: string, file?: FileRef): Promise<boolean> {
    const record = this.records.get(this.resolve(id));
    if (!record) return false;
    if (file && !this.matches(record, file)) return false;
    if (record.savePromise) return record.savePromise;
    const document = record.snapshot;
    if (!document.dirty) return true;
    if (document.conflict || document.draftConflict || !document.disk || document.disk.read_only) {
      this.publish(record, {
        error: "Resolve the disk or recovery conflict before saving. The local draft is retained.",
      });
      return false;
    }
    const content = this.raw(document),
      generation = document.generation;
    this.publish(record, { saving: true, error: "" });
    record.savePromise = (async () => {
      try {
        await this.persist(document.id);
        if (record.snapshot.draftConflict) return false;
        const saved = await this.io(() =>
          this.api.save(document.file, document.baseRevision, content)
        );
        const baseText = textFrom(saved.content ?? "", saved.newline),
          dirty = !record.snapshot.text.eq(baseText);
        this.publish(record, {
          saving: false,
          disk: saved,
          baseRevision: saved.revision,
          baseText,
          bom: saved.bom,
          dirty,
          conflict: false,
          recovered: false,
          error: "",
          generation: Math.max(generation, record.snapshot.generation) + 1,
          recoverySaved: false,
        });
        this.schedule(record);
        filesChanged(saved.file);
        return true;
      } catch (error) {
        this.publish(record, {
          saving: false,
          error: errorText(error),
          conflict: error instanceof FactoryApiError && error.status === 409,
        });
        await this.refresh(document.id);
        return false;
      } finally {
        record.savePromise = undefined;
        if (record.snapshot.saving) this.publish(record, { saving: false });
      }
    })();
    return record.savePromise;
  }
  async saveAll() {
    await Promise.allSettled([...this.records.values()].map((record) => record.loaded));
    const documents = this.all().filter((document) => document.dirty);
    const results = [];
    for (const document of documents)
      results.push({ file: document.file, saved: await this.save(document.id) });
    return results;
  }
  async refreshFile(file: FileRef) {
    const id = this.byFile.get(fileKey(file));
    if (id) await this.refresh(id);
  }
  async refresh(id: string) {
    const record = this.records.get(this.resolve(id));
    if (!record) return;
    const epoch = ++record.readEpoch,
      file = record.snapshot.file,
      baseRevision = record.snapshot.baseRevision;
    const current = () =>
      record.readEpoch === epoch &&
      record.snapshot.baseRevision === baseRevision &&
      fileKey(record.snapshot.file) === fileKey(file);
    try {
      const disk = await this.io(() => this.api.read(file));
      if (!current()) return;
      if (record.snapshot.dirty) {
        this.publish(record, {
          disk,
          status: "ready",
          conflict: record.snapshot.baseRevision !== disk.revision,
        });
      } else {
        const text = textFrom(disk.content ?? "", disk.newline);
        this.replace(record, text);
        this.publish(record, {
          disk,
          status: "ready",
          text,
          baseText: text,
          baseRevision: disk.revision,
          newline: disk.newline,
          bom: disk.bom,
          error: "",
          conflict: false,
        });
      }
    } catch (error) {
      if (!current()) return;
      this.publish(record, {
        status: isMissing(error) ? "missing" : "unavailable",
        error: errorText(error),
      });
    }
  }
  async reloadDisk(id: string) {
    const record = this.records.get(this.resolve(id));
    if (!record) return;
    const generation = record.snapshot.generation,
      epoch = ++record.readEpoch,
      file = record.snapshot.file;
    await record.draftPromise;
    try {
      const disk = await this.io(() => this.api.read(file));
      if (record.readEpoch !== epoch) return;
      if (
        record.snapshot.generation !== generation ||
        fileKey(record.snapshot.file) !== fileKey(file)
      ) {
        this.publish(record, {
          error:
            "The buffer changed during reload. No local changes were discarded; confirm reload again if intended.",
        });
        return;
      }
      const text = textFrom(disk.content ?? "", disk.newline);
      this.replace(record, text);
      this.publish(record, {
        disk,
        text,
        baseText: text,
        baseRevision: disk.revision,
        newline: disk.newline,
        bom: disk.bom,
        dirty: false,
        conflict: false,
        draftConflict: false,
        recovered: false,
        error: "",
        generation: record.snapshot.generation + 1,
        recoverySaved: false,
        status: "ready",
      });
      this.schedule(record);
    } catch (error) {
      if (record.readEpoch !== epoch) return;
      this.publish(record, { error: errorText(error) });
    }
  }
  async saveResolved(id: string, reviewedRevision: string) {
    const record = this.records.get(this.resolve(id));
    if (!record) return false;
    try {
      const disk = await this.io(() => this.api.read(record.snapshot.file));
      if (disk.revision !== reviewedRevision) {
        this.publish(record, {
          disk,
          conflict: true,
          error: "Disk changed again while resolving. Review the newer version.",
        });
        return false;
      }
      this.publish(record, {
        disk,
        baseText: textFrom(disk.content ?? "", disk.newline),
        baseRevision: disk.revision,
        conflict: false,
        generation: record.snapshot.generation + 1,
      });
      return this.save(id);
    } catch (error) {
      this.publish(record, { error: errorText(error) });
      return false;
    }
  }
  async fork(id: string) {
    const previous = this.resolve(id),
      record = this.records.get(previous);
    if (!record) return previous;
    await record.draftPromise;
    await record.savePromise;
    const next = token();
    this.records.delete(previous);
    this.records.set(next, record);
    this.aliases.set(previous, next);
    this.byFile.set(fileKey(record.snapshot.file), next);
    record.draftRevision = 0;
    this.publish(
      record,
      {
        id: next,
        draftConflict: false,
        recoveryError: "",
        generation: record.snapshot.generation + 1,
        recoverySaved: false,
      },
      previous
    );
    this.schedule(record);
    await this.persist(next);
    if (record.snapshot.recoverySaved) await this.journal.remove(previous);
    return next;
  }
  async moved(result: MoveResult) {
    for (const record of this.records.values()) {
      const file = record.snapshot.file;
      if (
        file.project_id !== result.from.project_id ||
        file.root_id !== result.from.root_id ||
        !(
          file.path === result.from.path ||
          (result.kind === "directory" && file.path.startsWith(`${result.from.path}/`))
        )
      )
        continue;
      const updated = { ...file, path: result.to.path + file.path.slice(result.from.path.length) };
      this.forgetFileAliases(record.snapshot.id);
      this.byFile.set(fileKey(updated), record.snapshot.id);
      this.publish(record, { file: updated });
      try {
        const disk = await this.io(() => this.api.read(updated));
        const sameBase =
          (disk.content ?? "") ===
          record.snapshot.baseText.sliceString(
            0,
            record.snapshot.baseText.length,
            record.snapshot.newline === "crlf" ? "\r\n" : "\n"
          );
        this.publish(record, {
          disk,
          baseRevision: sameBase ? disk.revision : record.snapshot.baseRevision,
          conflict: !sameBase,
          generation: record.snapshot.generation + 1,
        });
        this.schedule(record);
      } catch (error) {
        this.publish(record, { error: errorText(error), status: "missing" });
      }
    }
    filesChanged(result.to);
  }
  private async poll() {
    if (this.polling || this.stopped) return;
    this.polling = true;
    try {
      const documents = this.all().filter((document) => document.status !== "loading");
      if (!documents.length) return;
      const selected = Array.from(
        { length: Math.min(8, documents.length) },
        (_, index) => documents[(this.pollOffset + index) % documents.length]
      );
      this.pollOffset = (this.pollOffset + selected.length) % documents.length;
      const groups = new Map<string, EditorDocument[]>();
      for (const document of selected)
        groups.set(document.file.project_id, [
          ...(groups.get(document.file.project_id) ?? []),
          document,
        ]);
      for (const [projectId, items] of groups) {
        try {
          const result = await this.io(() =>
            this.api.changes(
              projectId,
              items.map((document) => ({
                file: document.file,
                revision: document.disk?.revision ?? document.baseRevision,
              }))
            )
          );
          for (const change of result.changes)
            if (change.status !== "unchanged") {
              const id = this.byFile.get(fileKey(change.file));
              if (id) await this.refresh(id);
            }
        } catch (error) {
          for (const document of items) {
            const record = this.records.get(document.id);
            if (record) this.publish(record, { error: errorText(error) });
          }
        }
      }
    } finally {
      this.polling = false;
    }
  }
  async discard(id: string) {
    const record = this.records.get(this.resolve(id));
    if (!record) return;
    clearTimeout(record.timer);
    await record.savePromise;
    await record.draftPromise;
    clearTimeout(record.timer);
    await record.local;
    if (record.draftRevision)
      await this.io(() =>
        this.api.discardDraft(record.snapshot.file, record.snapshot.id, record.draftRevision)
      );
    record.draftRevision = 0;
    await this.journal.remove(record.snapshot.id);
    const disk = record.snapshot.disk,
      complete = disk?.mode === "text" && disk.content !== null,
      text = complete ? textFrom(disk.content!, disk.newline) : record.snapshot.baseText;
    this.replace(record, text);
    this.publish(record, {
      text,
      baseText: text,
      baseRevision: complete ? disk.revision : record.snapshot.baseRevision,
      newline: complete ? disk.newline : record.snapshot.newline,
      bom: complete ? disk.bom : record.snapshot.bom,
      dirty: false,
      conflict: false,
      recovered: false,
      recoverySaved: false,
      draftConflict: false,
      recoveryError: "",
      generation: record.snapshot.generation + 1,
    });
  }
  needsUnloadWarning() {
    return this.all().some((document) => document.dirty && !document.recoverySaved);
  }
  releaseUnused(referenced: Set<string>) {
    let removed = false;
    for (const [id, record] of this.records)
      if (
        !referenced.has(id) &&
        !record.loaded &&
        !record.snapshot.dirty &&
        !record.draftPromise &&
        !record.savePromise
      ) {
        clearTimeout(record.timer);
        this.forgetFileAliases(id);
        const aliases = [...this.aliases.keys()].filter((alias) => this.resolve(alias) === id);
        for (const alias of aliases) this.aliases.delete(alias);
        this.records.delete(id);
        removed = true;
      }
    if (removed) for (const listener of this.listeners) listener();
  }
  dispose() {
    this.stopped = true;
    this.journal.close?.();
    this.api.dispose?.();
    clearInterval(this.interval);
    for (const record of this.records.values()) clearTimeout(record.timer);
  }
}
