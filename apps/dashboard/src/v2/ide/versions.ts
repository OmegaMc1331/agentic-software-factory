import { fileKey, type FileRef } from "../files/types";
export class LspVersions {
  private records = new Map<string, { generation: number; version: number }>();
  constructor(private observed: Array<{ file: FileRef; version: number }> = []) {}
  version(document: { id: string; file: FileRef; generation: number }) {
    const previous = this.records.get(document.id);
    if (previous?.generation === document.generation) return previous.version;
    const observed =
      this.observed.find((item) => fileKey(item.file) === fileKey(document.file))?.version ?? 0;
    const version = Math.max(previous?.version ?? 0, observed) + 1;
    this.records.set(document.id, { generation: document.generation, version });
    return version;
  }
  forget(id: string) {
    this.records.delete(id);
  }
}
