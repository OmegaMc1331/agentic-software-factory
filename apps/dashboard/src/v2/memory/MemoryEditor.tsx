import { useState, type FormEvent } from "react";
import { Button, Input, Select, Textarea } from "../design-system/primitives/Controls";
import { SourceEditor } from "./SourceEditor";
import { mutation, saveMemory } from "./client";
import {
  categories,
  memoryInput,
  statuses,
  type Candidate,
  type MemoryInput,
  type MemoryRecord,
  type Provenance,
} from "./types";

export function MemoryEditor({
  projectId,
  previous,
  candidate,
  done,
  cancel,
}: {
  projectId: string;
  previous?: MemoryRecord;
  candidate?: Candidate;
  done(record?: MemoryRecord): void;
  cancel(): void;
}) {
  const [draft, setDraft] = useState<MemoryInput>(() =>
    previous || candidate
      ? memoryInput((previous ?? candidate?.memory)!)
      : {
          title: "",
          content: "",
          category: "general_facts",
          status: "active",
          subject: "",
          scope: "",
          sources: [
            {
              projectId,
              reference: { kind: "manual", note: "Explicit user-authored project fact" },
            },
          ],
        }
  );
  const [reason, setReason] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      if (candidate) {
        await mutation(projectId, `/candidates/${candidate.id}/accept`, "POST", { memory: draft });
        done();
      } else done(await saveMemory(projectId, draft, reason, previous));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function refresh() {
    setBusy(true);
    setError("");
    try {
      const provenance = await Promise.all(
        draft.sources.map((source) =>
          mutation<Provenance>(projectId, "/sources/resolve", "POST", {
            ...source,
            reference:
              source.reference.kind === "repository_file"
                ? { ...source.reference, revision: null }
                : source.reference,
          })
        )
      );
      setDraft((d) => ({ ...d, sources: provenance.map((p) => p.source) }));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <form
      className="asf-memory-editor"
      aria-label="Memory editor"
      onSubmit={(e) => void submit(e)}
      onKeyDown={(e) => {
        if ((e.ctrlKey || e.metaKey) && e.key === "Enter") {
          e.preventDefault();
          e.currentTarget.requestSubmit();
        }
        if (e.key === "Escape" && !busy) {
          e.stopPropagation();
          cancel();
        }
      }}
    >
      <h2>
        {candidate
          ? "Edit and accept proposal"
          : previous
            ? `Edit memory · revision ${previous.revision}`
            : "New project memory"}
      </h2>
      <Input
        label="Title"
        required
        maxLength={180}
        value={draft.title}
        onChange={(e) => setDraft({ ...draft, title: e.target.value })}
      />
      <div className="asf-memory-fields">
        <Select
          label="Category"
          value={draft.category}
          onChange={(e) =>
            setDraft({ ...draft, category: e.target.value as MemoryInput["category"] })
          }
        >
          {Object.entries(categories).map(([id, label]) => (
            <option key={id} value={id}>
              {label}
            </option>
          ))}
        </Select>
        <Select
          label="Memory status"
          value={draft.status}
          onChange={(e) => setDraft({ ...draft, status: e.target.value as MemoryInput["status"] })}
        >
          {statuses.map((status) => (
            <option key={status}>{status}</option>
          ))}
        </Select>
      </div>
      <Textarea
        label="Project fact"
        required
        maxLength={8192}
        rows={9}
        value={draft.content}
        onChange={(e) => setDraft({ ...draft, content: e.target.value })}
        hint="Context for agents. This text cannot grant permissions or change project policy."
      />
      <div className="asf-memory-fields">
        <Input
          label="Subject key"
          maxLength={120}
          value={draft.subject}
          onChange={(e) => setDraft({ ...draft, subject: e.target.value })}
          hint="Matching subject and scope expose conflicting claims."
        />
        <Input
          label="Component scope"
          maxLength={200}
          value={draft.scope}
          onChange={(e) => setDraft({ ...draft, scope: e.target.value })}
        />
      </div>
      <SourceEditor
        projectId={projectId}
        sources={draft.sources}
        onChange={(sources) => setDraft({ ...draft, sources })}
      />
      <Button onClick={() => void refresh()} disabled={busy}>
        Refresh source references
      </Button>
      <Input
        label="Change reason"
        maxLength={1000}
        value={reason}
        onChange={(e) => setReason(e.target.value)}
      />
      {error ? (
        <p role="alert">
          {error} Your draft is retained. For a revision conflict, cancel and reopen the current
          record before applying your correction.
        </p>
      ) : null}
      <div className="asf-memory-actions">
        <Button
          type="submit"
          tone="primary"
          loading={busy}
          disabled={busy || !draft.sources.length}
        >
          Save memory
        </Button>
        <Button disabled={busy} onClick={cancel}>
          Cancel editing
        </Button>
      </div>
    </form>
  );
}
