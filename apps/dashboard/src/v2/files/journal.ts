import { validDraft, type DraftEnvelope } from "./types";
export interface LocalDraft extends DraftEnvelope {
  owner: string;
  updated: number;
}
export interface DraftJournal {
  identity?(): Promise<string>;
  read(id: string): Promise<LocalDraft | undefined>;
  write(record: LocalDraft): Promise<void>;
  remove(id: string): Promise<void>;
  close?(): void;
}
export class BrowserDraftJournal implements DraftJournal {
  private database?: Promise<IDBDatabase>;
  private channel?: BroadcastChannel;
  private ownership?: Promise<void>;
  constructor(private owner: string) {}
  async identity() {
    await this.ensureOwner();
    return this.owner;
  }
  private ensureOwner() {
    return (this.ownership ??= new Promise<void>((resolve) => {
      if (typeof BroadcastChannel === "undefined") {
        this.owner = crypto.randomUUID();
        resolve();
        return;
      }
      const nonce = crypto.randomUUID();
      let claimed = false;
      this.channel = new BroadcastChannel("asf-editor-draft-owners");
      this.channel.onmessage = (event) => {
        const message = event.data;
        if (message?.owner !== this.owner || message.nonce === nonce) return;
        if (message.type === "claim")
          this.channel?.postMessage({
            type: "occupied",
            owner: this.owner,
            nonce,
            target: message.nonce,
          });
        else if (message.type === "occupied" && message.target === nonce && !claimed) {
          this.owner = crypto.randomUUID();
          try {
            sessionStorage.setItem("asf.v2.editor.window", this.owner);
          } catch {
            this.channel?.close();
            this.channel = undefined;
          }
        }
      };
      this.channel.postMessage({ type: "claim", owner: this.owner, nonce });
      setTimeout(() => {
        claimed = true;
        resolve();
      }, 60);
    }));
  }
  private open() {
    return (this.database ??= this.ensureOwner().then(
      () =>
        new Promise<IDBDatabase>((resolve, reject) => {
          if (!globalThis.indexedDB) {
            reject(new Error("Browser draft storage is unavailable."));
            return;
          }
          const request = indexedDB.open("asf-editor-drafts", 1);
          request.onupgradeneeded = () => {
            request.result.createObjectStore("drafts", { keyPath: "key" });
          };
          request.onsuccess = () => resolve(request.result);
          request.onerror = () => reject(new Error("Browser draft storage could not open."));
        })
    ));
  }
  async read(id: string) {
    const db = await this.open();
    const values = await new Promise<Array<LocalDraft & { key: string }>>((resolve, reject) => {
      const request = db.transaction("drafts").objectStore("drafts").getAll();
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(new Error("Could not read local drafts."));
    });
    const matches = values.filter((value) => value.document?.id === id);
    const candidate =
      matches.find((value) => value.owner === this.owner) ??
      matches.sort((a, b) => b.updated - a.updated)[0];
    if (candidate && (!validDraft(candidate.document) || !Number.isSafeInteger(candidate.revision)))
      throw new Error("A local draft record is corrupt. Its stored copy was retained.");
    if (
      matches.length > 1 &&
      !matches.some((value) => value.owner === this.owner) &&
      new Set(matches.map((value) => value.document.content)).size > 1
    )
      throw new Error(
        "Several windows retained different local drafts. Reopen the original window to choose its version; all records are retained."
      );
    return candidate;
  }
  async write(record: LocalDraft) {
    if (!validDraft(record.document))
      throw new Error("Invalid draft record; previous recovery data retained.");
    const db = await this.open();
    return new Promise<void>((resolve, reject) => {
      const transaction = db.transaction("drafts", "readwrite"),
        store = transaction.objectStore("drafts"),
        read = store.getAll();
      read.onsuccess = () => {
        const key = `${this.owner}:${record.document.id}`;
        let values = read.result.filter((value) => value.key !== key);
        let bytes =
          values.reduce((total, value) => total + (value.bytes ?? 0), 0) +
          new TextEncoder().encode(JSON.stringify(record)).length;
        const clean = values
          .filter((value) => !value.document?.dirty)
          .sort((a, b) => a.updated - b.updated || a.key.localeCompare(b.key));
        while ((values.length >= 128 || bytes > 33_554_432) && clean.length) {
          const previous = clean.shift()!;
          store.delete(previous.key);
          values = values.filter((value) => value.key !== previous.key);
          bytes -= previous.bytes ?? 0;
        }
        if (values.length >= 128 || bytes > 33_554_432) {
          transaction.abort();
          return;
        }
        store.put({
          ...record,
          owner: this.owner,
          key,
          bytes: new TextEncoder().encode(JSON.stringify(record)).length,
        });
      };
      transaction.oncomplete = () => resolve();
      transaction.onerror = transaction.onabort = () =>
        reject(
          new Error(
            "Browser draft storage is full or unavailable. Keep this window open until recovery is saved."
          )
        );
    });
  }
  async remove(id: string) {
    const db = await this.open();
    await new Promise<void>((resolve, reject) => {
      const transaction = db.transaction("drafts", "readwrite");
      transaction.objectStore("drafts").delete(`${this.owner}:${id}`);
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(new Error("Local draft cleanup failed."));
    });
  }
  close() {
    this.channel?.close();
    void this.database?.then((database) => database.close()).catch(() => {});
  }
}
