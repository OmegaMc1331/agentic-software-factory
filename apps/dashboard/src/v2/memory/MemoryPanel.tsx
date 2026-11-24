import { useState } from "react";
import { Button, Checkbox, Input, Select } from "../design-system/primitives/Controls";
import { ConfirmationDialog } from "../design-system/primitives/Overlays";
import { EmptyState, LoadingIndicator } from "../design-system/components/Application";
import type { DockSession } from "../workspace/production/controller";
import { MemoryEditor } from "./MemoryEditor";
import { SourceEditor } from "./SourceEditor";
import { mutation, saveMemory, useMemoryData } from "./client";
import {
  categories,
  memoryInput,
  sourceLabel,
  statuses,
  type Candidate,
  type MemoryPage,
  type MemoryRecord,
  type Settings,
  type Source,
} from "./types";
import "./memory.css";

export function MemoryPanel({
  projectId,
  dock,
  initialId,
}: {
  projectId: string;
  dock: DockSession;
  initialId?: string;
}) {
  const [tab, setTab] = useState("saved");
  const [query, setQuery] = useState("");
  const [search, setSearch] = useState("");
  const [category, setCategory] = useState("");
  const [status, setStatus] = useState("");
  const [before, setBefore] = useState<number>();
  const [selected, setSelected] = useState(initialId ?? "");
  const [editing, setEditing] = useState<MemoryRecord | "new">();
  const [version, setVersion] = useState(0);
  const [notice, setNotice] = useState("");
  const [deleted, setDeleted] = useState<Set<string>>(() => new Set());
  const params = new URLSearchParams({ query: search, limit: "50" });
  if (category) params.set("category", category);
  if (status) params.set("status", status);
  if (before) params.set("before", String(before));
  const listing = useMemoryData<MemoryPage>(projectId, `?${params}`, version);
  const reload = () => setVersion((n) => n + 1);
  const settings = useMemoryData<Settings>(projectId, "/settings", version);
  return (
    <div className="asf-memory" aria-label="Project memory">
      <header className="asf-memory-toolbar">
        <h2>Project memory</h2>
        <Button
          icon="plus"
          onClick={() => {
            setEditing("new");
            setTab("saved");
          }}
        >
          New memory
        </Button>
        <Button onClick={reload}>Refresh</Button>
      </header>
      <nav className="asf-memory-tabs" aria-label="Memory sections">
        {["saved", "proposals", "retrieval", "settings"].map((name) => (
          <Button
            key={name}
            aria-pressed={tab === name}
            onClick={() => {
              setTab(name);
              setEditing(undefined);
            }}
          >
            {name === "saved" ? "Saved knowledge" : name[0].toUpperCase() + name.slice(1)}
          </Button>
        ))}
      </nav>
      <div role="status" aria-live="polite">
        {notice}
      </div>
      {settings.value && !settings.value.enabled ? (
        <p>
          Project memory is disabled. Saved records remain editable; agents receive no saved memory.
        </p>
      ) : null}
      {tab === "saved" ? (
        editing ? (
          <MemoryEditor
            key={editing === "new" ? "new" : `${editing.id}:${editing.revision}`}
            projectId={projectId}
            previous={editing === "new" ? undefined : editing}
            cancel={() => setEditing(undefined)}
            done={(record) => {
              setEditing(undefined);
              if (record) setSelected(record.id);
              setNotice("Memory saved in project storage.");
              reload();
            }}
          />
        ) : (
          <>
            <form
              className="asf-memory-search"
              onSubmit={(e) => {
                e.preventDefault();
                setSearch(query);
                setBefore(undefined);
              }}
            >
              <Input
                label="Search project memory"
                value={query}
                maxLength={1000}
                onChange={(e) => setQuery(e.target.value)}
              />
              <Button type="submit">Search memories</Button>
              <Select
                label="Filter category"
                value={category}
                onChange={(e) => {
                  setCategory(e.target.value);
                  setBefore(undefined);
                }}
              >
                <option value="">All categories</option>
                {Object.entries(categories).map(([key, name]) => (
                  <option key={key} value={key}>
                    {name}
                  </option>
                ))}
              </Select>
              <Select
                label="Filter status"
                value={status}
                onChange={(e) => {
                  setStatus(e.target.value);
                  setBefore(undefined);
                }}
              >
                <option value="">All statuses</option>
                {statuses.map((name) => (
                  <option key={name}>{name}</option>
                ))}
              </Select>
            </form>
            {listing.error ? <p role="alert">{listing.error}</p> : null}
            <div className="asf-memory-columns">
              <div className="asf-memory-list" aria-label="Saved memories">
                {!listing.value ? (
                  <LoadingIndicator label="Loading memories" />
                ) : listing.value.items.filter((m) => !deleted.has(m.id)).length === 0 ? (
                  <EmptyState
                    title="No matching project memories"
                    description="Save a scoped fact with source evidence, or review proposed knowledge."
                  />
                ) : (
                  <ul>
                    {listing.value.items
                      .filter((m) => !deleted.has(m.id))
                      .map((m) => (
                        <li key={m.id}>
                          <button
                            type="button"
                            aria-pressed={selected === m.id}
                            onClick={() => setSelected(m.id)}
                          >
                            <strong>{m.title}</strong>
                            <span>
                              {categories[m.category]} · {m.status} · revision {m.revision}
                            </span>
                            <p>
                              {m.content.slice(0, 180)}
                              {m.content.length > 180 ? "…" : ""}
                            </p>
                            <small>
                              {m.origin.replaceAll("_", " ")} · {m.updatedAt}
                            </small>
                          </button>
                        </li>
                      ))}
                  </ul>
                )}
                <div className="asf-memory-actions">
                  <Button disabled={!before} onClick={() => setBefore(undefined)}>
                    Newest memories
                  </Button>
                  <Button
                    disabled={!listing.value?.nextCursor}
                    onClick={() => setBefore(listing.value?.nextCursor ?? undefined)}
                  >
                    Older memories
                  </Button>
                </div>
              </div>
              {selected && !deleted.has(selected) ? (
                <MemoryDetail
                  key={selected}
                  projectId={projectId}
                  memoryId={selected}
                  dock={dock}
                  version={version}
                  edit={setEditing}
                  select={setSelected}
                  changed={(message, removed) => {
                    setNotice(message);
                    if (removed) {
                      setDeleted((s) => new Set([...s, removed]));
                      setSelected("");
                    }
                    reload();
                  }}
                />
              ) : (
                <div className="asf-memory-detail">
                  <p>Select a memory to inspect its sources and revisions.</p>
                </div>
              )}
            </div>
          </>
        )
      ) : tab === "proposals" ? (
        <Proposals
          projectId={projectId}
          settings={settings.value}
          version={version}
          changed={(message) => {
            setNotice(message);
            reload();
          }}
        />
      ) : tab === "retrieval" ? (
        <RetrievalPreview projectId={projectId} />
      ) : settings.value ? (
        <MemorySettings
          key={settings.value.revision}
          projectId={projectId}
          settings={settings.value}
          saved={() => {
            setNotice("Memory policy saved.");
            reload();
          }}
        />
      ) : (
        <p role="alert">{settings.error ?? "Loading memory settings"}</p>
      )}
    </div>
  );
}

function MemoryDetail({
  projectId,
  memoryId,
  version,
  dock,
  edit,
  select,
  changed,
}: {
  projectId: string;
  memoryId: string;
  version: number;
  dock: DockSession;
  edit(m: MemoryRecord): void;
  select(id: string): void;
  changed(message: string, deleted?: string): void;
}) {
  const data = useMemoryData<MemoryRecord>(projectId, `/${memoryId}`, version);
  const [confirm, setConfirm] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const m = data.value;
  async function act(work: () => Promise<unknown>, message: string, deleted?: string) {
    setBusy(true);
    setError("");
    try {
      await work();
      setConfirm(false);
      changed(message, deleted);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  if (!m)
    return (
      <div className="asf-memory-detail">
        {data.error ? (
          <p role="alert">{data.error}</p>
        ) : (
          <LoadingIndicator label="Loading memory detail" />
        )}
      </div>
    );
  return (
    <article className="asf-memory-detail" aria-label={`Memory: ${m.title}`}>
      <h3>{m.title}</h3>
      <p>
        {categories[m.category]} · <strong>{m.status}</strong> · revision {m.revision}
      </p>
      <p className="asf-memory-content">{m.content}</p>
      <dl>
        <dt>Origin</dt>
        <dd>{m.origin.replaceAll("_", " ")}</dd>
        <dt>Subject / scope</dt>
        <dd>
          {m.subject || "Unspecified"} / {m.scope || "Project"}
        </dd>
        <dt>Identity</dt>
        <dd>{m.id}</dd>
        <dt>Updated</dt>
        <dd>{m.updatedAt}</dd>
      </dl>
      <div className="asf-memory-actions">
        <Button disabled={busy} onClick={() => edit(m)}>
          Edit memory
        </Button>
        <Button
          disabled={busy || m.status === "superseded"}
          onClick={() =>
            void act(
              () =>
                saveMemory(
                  projectId,
                  { ...memoryInput(m), status: m.status === "disabled" ? "active" : "disabled" },
                  "Explicit user eligibility change",
                  m
                ),
              "Memory eligibility updated."
            )
          }
        >
          {m.status === "disabled" ? "Enable memory" : "Disable memory"}
        </Button>
        <Button tone="danger" disabled={busy} onClick={() => setConfirm(true)}>
          Delete memory
        </Button>
      </div>
      {error ? <p role="alert">{error}</p> : null}
      <h4>Provenance</h4>
      <p>
        Source checks establish identity and content. They do not prove every claim is semantically
        correct.
      </p>
      {m.provenance.map((p, index) => (
        <section className="asf-memory-provenance" key={index} aria-label={`Source ${index + 1}`}>
          <strong>{sourceLabel(p.source)}</strong>
          <p>{p.validationStatus.replaceAll("_", " ")}</p>
          <dl>
            <dt>Source revision</dt>
            <dd>{p.sourceRevision}</dd>
            <dt>Checked</dt>
            <dd>{p.checkedAt}</dd>
            <dt>Fingerprint</dt>
            <dd>{p.fingerprint}</dd>
          </dl>
          {p.evidenceExcerpt ? <blockquote>{p.evidenceExcerpt}</blockquote> : null}
          {p.source.reference.kind === "repository_file" &&
          !p.validationStatus.startsWith("source_unavailable") ? (
            <Button
              onClick={() => {
                const ref = p.source.reference;
                if (ref.kind === "repository_file")
                  void dock
                    .openFile({ project_id: projectId, root_id: ref.rootId, path: ref.path })
                    .catch((e) => setError(String(e)));
              }}
            >
              Open source file
            </Button>
          ) : null}
        </section>
      ))}
      {m.relatedIds.length ? (
        <section aria-label="Related memory claims">
          <h4>Related claims</h4>
          {m.relatedIds.map((related) => (
            <div className="asf-memory-actions" key={related}>
              <Button onClick={() => select(related)}>{related}</Button>
              <Button
                disabled={busy || m.status === "superseded" || m.status === "stale"}
                onClick={() =>
                  void act(async () => {
                    const other = await import("../../api").then(({ request }) =>
                      request<MemoryRecord>(`/projects/${projectId}/memory/${related}`)
                    );
                    return mutation(projectId, `/${related}/supersede`, "POST", {
                      expectedRevision: other.revision,
                      targetId: m.id,
                      targetRevision: m.revision,
                    });
                  }, "Current claim confirmed; related memory superseded.")
                }
              >
                Use this claim instead
              </Button>
            </div>
          ))}
        </section>
      ) : null}
      <RevisionHistory projectId={projectId} memoryId={m.id} revision={m.revision} />
      {confirm ? (
        <ConfirmationDialog
          title="Delete project memory"
          description="Remove this memory, all its revisions, search entries and related queued proposals. Historical invocation reference hashes, independent mission records and existing backups remain. Future invocations will exclude this memory."
          confirmLabel="Delete permanently"
          danger
          open={confirm}
          onClose={() => setConfirm(false)}
          onConfirm={() =>
            void act(
              () => mutation(projectId, `/${m.id}`, "DELETE", { expectedRevision: m.revision }),
              "Memory deleted from active storage and retrieval.",
              m.id
            )
          }
        />
      ) : null}
    </article>
  );
}

function RevisionHistory({
  projectId,
  memoryId,
  revision,
}: {
  projectId: string;
  memoryId: string;
  revision: number;
}) {
  const [open, setOpen] = useState(false);
  return (
    <details onToggle={(e) => setOpen(e.currentTarget.open)}>
      <summary>Revision history</summary>
      {open ? <RevisionPage key={revision} projectId={projectId} memoryId={memoryId} /> : null}
    </details>
  );
}
function RevisionPage({ projectId, memoryId }: { projectId: string; memoryId: string }) {
  const [before, setBefore] = useState<number>();
  const history = useMemoryData<MemoryRecord[]>(
    projectId,
    `/${memoryId}/revisions${before ? `?before=${before}` : ""}`
  );
  return (
    <div>
      {history.error ? <p role="alert">{history.error}</p> : null}
      {history.value?.map((m) => (
        <details key={m.revision}>
          <summary>
            Revision {m.revision} · {m.status} · {m.updatedAt}
          </summary>
          <p>{m.changeReason}</p>
          <p className="asf-memory-content">{m.content}</p>
          <p>{m.origin.replaceAll("_", " ")}</p>
          <pre tabIndex={0}>{JSON.stringify(m.provenance, null, 2)}</pre>
        </details>
      ))}
      <Button
        disabled={history.value?.length !== 50}
        onClick={() => setBefore(history.value?.at(-1)?.revision)}
      >
        Earlier revisions
      </Button>
    </div>
  );
}

function Proposals({
  projectId,
  settings,
  version,
  changed,
}: {
  projectId: string;
  settings?: Settings;
  version: number;
  changed(message: string): void;
}) {
  const [before, setBefore] = useState<number>();
  const data = useMemoryData<{ items: Candidate[]; nextCursor: number | null }>(
    projectId,
    `/candidates${before ? `?before=${before}` : ""}`,
    version
  );
  const [editing, setEditing] = useState<Candidate>();
  const [sources, setSources] = useState<Source[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const history = useMemoryData<
    Array<{ id: string; state: string; agentSessionId: number | null; error: string | null }>
  >(projectId, "/extractions", version);
  async function act(work: () => Promise<unknown>, message: string) {
    setBusy(true);
    setError("");
    try {
      await work();
      changed(message);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  if (editing)
    return (
      <MemoryEditor
        projectId={projectId}
        candidate={editing}
        done={() => {
          setEditing(undefined);
          changed("Reviewed proposal accepted.");
        }}
        cancel={() => setEditing(undefined)}
      />
    );
  return (
    <section className="asf-memory-section" aria-label="Memory proposals">
      <h3>Proposed knowledge</h3>
      <p>
        Claims are extracted from validated evidence. Review conflicts and interpretation before
        accepting them.
      </p>
      {error || data.error ? <p role="alert">{error || data.error}</p> : null}
      {data.value?.items.length === 0 ? <p>No pending memory proposals.</p> : null}
      {data.value?.items.map((c) => (
        <article className="asf-memory-proposal" key={c.id}>
          <h4>{c.memory.title}</h4>
          <p>
            {categories[c.memory.category]} ·{" "}
            {c.issue.replaceAll("_", " ") || "Source-backed proposal"}
          </p>
          <p className="asf-memory-content">{c.memory.content}</p>
          <details>
            <summary>Inspect proposal sources</summary>
            {c.memory.provenance.map((p, i) => (
              <div key={i}>
                <p>
                  {sourceLabel(p.source)} · {p.validationStatus}
                </p>
                <p>{p.sourceRevision}</p>
                {p.evidenceExcerpt ? <blockquote>{p.evidenceExcerpt}</blockquote> : null}
              </div>
            ))}
          </details>
          <div className="asf-memory-actions">
            <Button
              disabled={busy}
              onClick={() =>
                void act(
                  () => mutation(projectId, `/candidates/${c.id}/accept`, "POST"),
                  "Proposal accepted."
                )
              }
            >
              Accept proposal
            </Button>
            <Button disabled={busy} onClick={() => setEditing(c)}>
              Edit and accept
            </Button>
            <Button
              disabled={busy}
              onClick={() =>
                void act(
                  () => mutation(projectId, `/candidates/${c.id}/reject`, "POST"),
                  "Proposal rejected."
                )
              }
            >
              Reject proposal
            </Button>
          </div>
        </article>
      ))}
      <div className="asf-memory-actions">
        <Button disabled={!before} onClick={() => setBefore(undefined)}>
          Newest proposals
        </Button>
        <Button
          disabled={!data.value?.nextCursor}
          onClick={() => setBefore(data.value?.nextCursor ?? undefined)}
        >
          Older proposals
        </Button>
      </div>
      <details>
        <summary>Extract from selected evidence</summary>
        {!settings?.enabled || settings.extractionMode === "disabled" ? (
          <p>
            Extraction is disabled. Select review or automatic mode in Memory settings before
            invoking an extractor.
          </p>
        ) : (
          <>
            <SourceEditor projectId={projectId} sources={sources} onChange={setSources} extract />
            <Button
              disabled={busy || !sources.length}
              loading={busy}
              onClick={() =>
                void act(
                  () => mutation(projectId, "/extract", "POST", { sources }),
                  "Extraction request finished. Inspect proposals and execution history."
                )
              }
            >
              Extract project facts
            </Button>
          </>
        )}
        {busy ? <p role="status">Processing the authorized memory operation…</p> : null}
      </details>
      <details>
        <summary>Extraction history</summary>
        {history.error ? <p role="alert">{history.error}</p> : null}
        {history.value?.map((h) => (
          <p key={h.id}>
            {h.id} · {h.state} ·{" "}
            {h.agentSessionId ? `AgentSession #${h.agentSessionId}` : "No session recorded"}
            {h.error ? ` · ${h.error}` : ""}
          </p>
        ))}
      </details>
    </section>
  );
}

function RetrievalPreview({ projectId }: { projectId: string }) {
  const [query, setQuery] = useState("");
  const [result, setResult] = useState<{
    records: MemoryRecord[];
    truncated: boolean;
    notice: string;
  }>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  return (
    <section className="asf-memory-section" aria-label="Memory retrieval preview">
      <h3>Context retrieval</h3>
      <p>
        Preview current eligible memories for an objective. Agent invocation audits separately
        record what was actually supplied.
      </p>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          setBusy(true);
          setError("");
          void mutation<typeof result>(projectId, "/retrieve", "POST", { query })
            .then(setResult)
            .catch((e) => setError(String(e)))
            .finally(() => setBusy(false));
        }}
      >
        <Input
          label="Retrieval objective"
          value={query}
          maxLength={16000}
          onChange={(e) => setQuery(e.target.value)}
        />
        <Button type="submit" disabled={busy || !query.trim()}>
          Retrieve context
        </Button>
      </form>
      {error ? <p role="alert">{error}</p> : null}
      {result ? (
        <div role="status">
          <p>{result.notice}</p>
          <p>
            {result.records.length} eligible records
            {result.truncated ? " · context budget reached" : ""}
          </p>
          {result.records.map((m) => (
            <p key={m.id}>
              {m.title} · revision {m.revision} · {m.id}
            </p>
          ))}
        </div>
      ) : null}
    </section>
  );
}

export function MemorySettings({
  projectId,
  settings,
  saved,
}: {
  projectId: string;
  settings: Settings;
  saved(): void;
}) {
  const [draft, setDraft] = useState(settings);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  return (
    <form
      className="asf-memory-section"
      aria-label="Memory settings"
      onSubmit={(e) => {
        e.preventDefault();
        setBusy(true);
        setError("");
        void mutation(projectId, "/settings", "PUT", draft)
          .then(saved)
          .catch((e) => setError(String(e)))
          .finally(() => setBusy(false));
      }}
    >
      <h3>Project memory policy</h3>
      <Checkbox
        label="Enable project memory"
        checked={draft.enabled}
        onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })}
      />
      <Checkbox
        label="Retrieve eligible memory for agents"
        checked={draft.retrievalEnabled}
        onChange={(e) => setDraft({ ...draft, retrievalEnabled: e.target.checked })}
      />
      <Select
        label="Automatic extraction"
        value={draft.extractionMode}
        onChange={(e) =>
          setDraft({ ...draft, extractionMode: e.target.value as Settings["extractionMode"] })
        }
      >
        <option value="disabled">Disabled</option>
        <option value="review">Propose for review</option>
        <option value="automatic">Store permitted source-backed facts</option>
      </Select>
      <p>
        Automatic mode retains review for conflicts, policy-like claims and unsupported evidence
        quality. Memory never changes tool permissions. Mission resume remains manual.
      </p>
      <fieldset>
        <legend>Eligible categories</legend>
        {Object.entries(categories).map(([key, name]) => (
          <Checkbox
            key={key}
            label={name}
            checked={draft.categories.includes(key as keyof typeof categories)}
            onChange={(e) =>
              setDraft({
                ...draft,
                categories: e.target.checked
                  ? [...draft.categories, key as keyof typeof categories]
                  : draft.categories.filter((c) => c !== key),
              })
            }
          />
        ))}
      </fieldset>
      <div className="asf-memory-fields">
        <Input
          label="Maximum context records"
          type="number"
          min={1}
          max={12}
          value={draft.maxContextRecords}
          onChange={(e) => setDraft({ ...draft, maxContextRecords: Number(e.target.value) })}
        />
        <Input
          label="Maximum context bytes"
          type="number"
          min={512}
          max={32000}
          value={draft.maxContextBytes}
          onChange={(e) => setDraft({ ...draft, maxContextBytes: Number(e.target.value) })}
        />
      </div>
      <p>
        Deletion removes the record, revisions, index entries and related proposals. Existing backup
        files and independent mission output remain separately retained. Historical invocation
        audits keep only IDs, revisions and hashes.
      </p>
      {error ? <p role="alert">{error} Settings draft retained.</p> : null}
      <Button type="submit" disabled={busy}>
        Save memory policy
      </Button>
      <Button
        disabled={busy}
        onClick={() => {
          setBusy(true);
          void mutation(projectId, "/index/rebuild", "POST")
            .then(() => setError(""))
            .catch((e) => setError(String(e)))
            .finally(() => setBusy(false));
        }}
      >
        Rebuild search index
      </Button>
    </form>
  );
}
