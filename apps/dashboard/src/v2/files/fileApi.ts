import { request, FactoryApiError, type ApiRequestOptions } from "../../api";
import { projectPath } from "../projects/projectApi";
import { ResourceCache } from "../workspace/production/data";
import type {
  DirectoryPage,
  Draft,
  DraftEnvelope,
  FileRef,
  FileSnapshot,
  MoveResult,
  Recovery,
  RootCatalog,
  SearchRequest,
  SearchResponse,
} from "./types";

export class FileApi {
  readonly catalogs = new ResourceCache((projectId, signal) => this.roots(projectId, signal));
  private active = 0;
  private pending: Array<{ priority: boolean; start: () => void }> = [];
  constructor(private call: typeof request = request) {}
  private async send<T>(
    url: string,
    init?: RequestInit,
    options?: ApiRequestOptions,
    priority = true
  ): Promise<T> {
    if (this.active >= 2) {
      if (this.pending.length >= 128)
        throw new FactoryApiError(
          "The file request queue is full. Wait for pending work and retry.",
          "http",
          { status: 429, code: "file_resource_busy" }
        );
      await new Promise<void>((start) => this.pending.push({ priority, start }));
    } else this.active++;
    try {
      if (options?.signal?.aborted) throw new DOMException("File request cancelled.", "AbortError");
      return await this.call<T>(url, init, options);
    } finally {
      const priorityIndex = this.pending.findIndex((job) => job.priority);
      const next = this.pending.splice(priorityIndex < 0 ? 0 : priorityIndex, 1)[0];
      if (next) next.start();
      else this.active--;
    }
  }
  read(file: FileRef, signal?: AbortSignal) {
    return this.send<FileSnapshot>(
      projectPath(
        file.project_id,
        `files/content?${new URLSearchParams({ root_id: file.root_id, path: file.path })}`
      ),
      undefined,
      { signal }
    );
  }
  head(file: FileRef) {
    return this.send<FileSnapshot>(
      projectPath(
        file.project_id,
        `files/git-head?${new URLSearchParams({ root_id: file.root_id, path: file.path })}`
      )
    );
  }
  roots(projectId: string, signal?: AbortSignal) {
    return this.send<RootCatalog>(projectPath(projectId, "files/roots"), undefined, { signal });
  }
  tree(
    projectId: string,
    rootId: string,
    path: string,
    hidden: boolean,
    after?: string,
    signal?: AbortSignal
  ) {
    return this.send<DirectoryPage>(
      projectPath(
        projectId,
        `files/tree?${new URLSearchParams({ root_id: rootId, path, hidden: String(hidden), ...(after ? { after } : {}) })}`
      ),
      undefined,
      { signal },
      false
    );
  }
  save(file: FileRef, expected: string, content: string) {
    return this.mutate<FileSnapshot>(file.project_id, "files/content", "PUT", {
      file,
      expected_revision: expected,
      content,
    });
  }
  create(file: FileRef, directory: boolean) {
    return this.mutate(file.project_id, "files/entries", "POST", { file, directory });
  }
  move(file: FileRef, expected: string, destination: string) {
    return this.mutate<MoveResult>(file.project_id, "files/move", "POST", {
      file,
      expected_revision: expected,
      destination,
    });
  }
  delete(file: FileRef, expected: string) {
    return this.mutate<Recovery>(file.project_id, "files/entries", "DELETE", {
      file,
      expected_revision: expected,
      confirmed: true,
    });
  }
  recovery(projectId: string) {
    return this.send<Recovery[]>(projectPath(projectId, "files/recovery"));
  }
  recoveryContent(projectId: string, id: string) {
    return this.send<FileSnapshot>(projectPath(projectId, `files/recovery/${id}/content`));
  }
  purgeRecovery(projectId: string, id: string) {
    return this.mutate(projectId, `files/recovery/${id}/purge`, "POST", { confirmed: true });
  }
  restore(projectId: string, id: string) {
    return this.mutate<FileRef>(projectId, `files/recovery/${id}/restore`, "POST", {});
  }
  draft(file: FileRef, id: string) {
    return this.send<DraftEnvelope>(projectPath(file.project_id, `editor/drafts/${id}`));
  }
  saveDraft(document: Draft, revision: number) {
    return this.mutate<DraftEnvelope>(
      document.file.project_id,
      `editor/drafts/${document.id}`,
      "PUT",
      { base_revision: revision, document }
    );
  }
  discardDraft(file: FileRef, id: string, revision: number) {
    return this.mutate(file.project_id, `editor/drafts/${id}`, "DELETE", {
      revision,
      confirmed: true,
    });
  }
  changes(projectId: string, items: Array<{ file: FileRef; revision: string }>) {
    return this.mutate<{ changes: Array<{ file: FileRef; status: string; revision?: string }> }>(
      projectId,
      "files/changes",
      "POST",
      { items },
      { effect: "read" }
    );
  }
  search(projectId: string, body: SearchRequest, signal?: AbortSignal) {
    return this.mutate<SearchResponse>(projectId, "files/search", "POST", body, {
      effect: "read",
      signal,
    });
  }
  cancel(projectId: string, id: string) {
    return this.mutate(projectId, `files/search/${id}/cancel`, "POST", {}, undefined, true);
  }
  dispose() {
    this.catalogs.dispose();
  }
  private mutate<T = unknown>(
    projectId: string,
    path: string,
    method: string,
    body: unknown,
    options?: ApiRequestOptions,
    bypass = false
  ) {
    const url = projectPath(projectId, path),
      init = {
        method,
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      };
    return bypass ? this.call<T>(url, init, options) : this.send<T>(url, init, options);
  }
}
export const filesChanged = (file: FileRef) =>
  window.dispatchEvent(new CustomEvent("factory-files-changed", { detail: file }));
