import { useState } from "react";
import { Button, Input, Select } from "../design-system/primitives/Controls";
import { useData } from "../workspace/production/data";
import { mutation } from "./client";
import { sourceLabel, type Source, type SourceRef, type Provenance } from "./types";

const kinds = [
  "manual",
  "repository_file",
  "mission",
  "conversation_message",
  "task_attempt",
  "review",
  "test_evidence",
  "agent_session",
  "integration_commit",
] as const;
export function SourceEditor({
  projectId,
  sources,
  onChange,
  extract = false,
}: {
  projectId: string;
  sources: Source[];
  onChange(value: Source[]): void;
  extract?: boolean;
}) {
  const roots = useData<{ roots: Array<{ id: string; kind: string; label: string }> }>(
    projectId,
    "files/roots"
  );
  const [kind, setKind] = useState<SourceRef["kind"]>(extract ? "repository_file" : "manual");
  const [text, setText] = useState("");
  const [other, setOther] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const root = roots.value?.roots.find((r) => r.kind === "repository");
  const numeric = Number(text);
  async function add() {
    setBusy(true);
    setError("");
    try {
      let reference: SourceRef;
      switch (kind) {
        case "manual":
          reference = { kind, note: text };
          break;
        case "repository_file":
          reference = { kind, rootId: root?.id ?? "", path: text, revision: null };
          break;
        case "mission":
          reference = { kind, runId: numeric };
          break;
        case "conversation_message":
          reference = { kind, conversationId: text, messageId: Number(other) };
          break;
        case "agent_session":
          reference = { kind, sessionId: numeric };
          break;
        case "integration_commit":
          reference = { kind, runId: numeric, commitSha: other };
          break;
        default:
          reference = { kind, attemptId: numeric };
      }
      const value = await mutation<Provenance>(projectId, "/sources/resolve", "POST", {
        projectId,
        reference,
      });
      onChange([...sources, value.source]);
      setText("");
      setOther("");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <fieldset className="asf-memory-sources">
      <legend>Source evidence</legend>
      <ul>
        {sources.map((source, index) => (
          <li key={`${source.reference.kind}:${index}`}>
            <span>{sourceLabel(source)}</span>
            <Button
              aria-label={`Remove source ${index + 1}`}
              onClick={() => onChange(sources.filter((_, i) => i !== index))}
            >
              Remove
            </Button>
          </li>
        ))}
      </ul>
      <Select
        label="Source type"
        value={kind}
        onChange={(e) => {
          setKind(e.target.value as SourceRef["kind"]);
          setText("");
          setOther("");
        }}
      >
        {kinds
          .filter((k) => !extract || k !== "manual")
          .map((k) => (
            <option key={k} value={k}>
              {k.replaceAll("_", " ")}
            </option>
          ))}
      </Select>
      <Input
        label={
          kind === "repository_file"
            ? "Project-relative source path"
            : kind === "manual"
              ? "Source note"
              : kind === "conversation_message"
                ? "Conversation ID"
                : kind === "agent_session"
                  ? "AgentSession ID"
                  : kind === "mission" || kind === "integration_commit"
                    ? "Mission ID"
                    : "Task attempt ID"
        }
        maxLength={kind === "repository_file" ? 2048 : 1000}
        value={text}
        onChange={(e) => setText(e.target.value)}
      />
      {kind === "conversation_message" || kind === "integration_commit" ? (
        <Input
          label={kind === "integration_commit" ? "Integration commit SHA" : "Message ID"}
          value={other}
          maxLength={64}
          onChange={(e) => setOther(e.target.value)}
        />
      ) : null}
      {error ? <p role="alert">{error}</p> : null}
      <Button
        onClick={() => void add()}
        disabled={
          busy ||
          !text.trim() ||
          sources.length >= (extract ? 4 : 8) ||
          (kind === "repository_file" && !root)
        }
        loading={busy}
      >
        Validate and add source
      </Button>
      {roots.error ? <p role="alert">{roots.error}</p> : null}
    </fieldset>
  );
}
