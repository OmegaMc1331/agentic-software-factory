import { useState, type ReactNode } from "react";
import { Button } from "../primitives/Controls";
import { CompactStatus, InlineAlert } from "../components/Application";
import { Icon } from "../icons/Icon";
import type { AgentData, ChatMessage, FileReference, MissionData, TestData } from "./domain";

export function ReferencedFile({
  file,
  onOpen,
}: {
  file: FileReference;
  onOpen(path: string): void;
}) {
  return (
    <button
      type="button"
      className="ds-file-reference"
      title={file.path}
      onClick={() => onOpen(file.path)}
    >
      <Icon name="file" size={13} />
      <span>{file.path}</span>
      <small>
        +{file.added} −{file.removed}
      </small>
    </button>
  );
}

function MessageBody({ message, children }: { message: ChatMessage; children?: ReactNode }) {
  const [expanded, setExpanded] = useState(false);
  const long = message.body.length > 1400;
  return (
    <article
      className={`ds-message ds-message--${message.role}`}
      data-message-id={message.id}
      aria-label={`${message.author} message`}
    >
      <header>
        <strong>{message.author}</strong>
        <time>{message.time}</time>
      </header>
      <div className="ds-message-body">
        {long && !expanded ? `${message.body.slice(0, 1400)}…` : message.body}
      </div>
      {long && (
        <Button tone="quiet" onClick={() => setExpanded(!expanded)}>
          {expanded ? "Show less" : "Read full message"}
        </Button>
      )}
      {children}
    </article>
  );
}

export function UserMessage({ message }: { message: ChatMessage }) {
  return <MessageBody message={message} />;
}

export function OrchestratorMessage({
  message,
  onOpenFile,
  onReview,
}: {
  message: ChatMessage;
  onOpenFile(path: string): void;
  onReview(): void;
}) {
  return (
    <MessageBody message={message}>
      {message.files && (
        <div className="ds-file-references">
          {message.files.map((file) => (
            <ReferencedFile key={file.path} file={file} onOpen={onOpenFile} />
          ))}
        </div>
      )}
      {message.result && (
        <ResultSummary
          title={message.result.title}
          files={message.result.fileCount}
          tests={message.result.testsPassed}
          limitation={message.result.limitation}
          provenance={message.provenance}
          onReview={onReview}
        />
      )}
    </MessageBody>
  );
}

export function ResultSummary({
  title,
  files,
  tests,
  limitation,
  onReview,
  provenance,
}: {
  title: string;
  files: number;
  tests: number;
  limitation?: string;
  onReview(): void;
  provenance: "fixture" | "runtime";
}) {
  return (
    <div className="ds-result">
      <div>
        <span className="ds-status ds-status--completed">
          <Icon name="check" />
        </span>
        <strong>{title}</strong>
        <Button tone="quiet" onClick={onReview}>
          Open review
          <Icon name="chevron" size={12} />
        </Button>
      </div>
      <p>
        {files} files changed<span>·</span>
        {tests} tests passed<span>·</span>
        {provenance === "fixture" ? "Fixture record" : "Recorded evidence"}
      </p>
      {limitation && <p>{limitation}</p>}
    </div>
  );
}

export function MissionStatus({ mission }: { mission: MissionData }) {
  const complete = mission.steps.filter((step) => step.status === "completed").length;
  return (
    <div className="ds-mission-status">
      <CompactStatus status={mission.status} />
      <h3 title={mission.title}>{mission.title}</h3>
      <p>{mission.expectedResult}</p>
      <progress
        aria-label="Completed mission tasks"
        value={complete}
        max={mission.steps.length || 1}
      />
      <span>
        {complete} of {mission.steps.length} tasks completed
        {mission.provenance === "fixture" ? " · fixture" : ""}
      </span>
    </div>
  );
}

export function CurrentMissionStep({ mission }: { mission: MissionData }) {
  const step =
    mission.steps.find((item) => item.status === "running" || item.status === "attention") ??
    mission.steps.find((item) => item.status !== "completed");
  return (
    <section className="ds-current-step">
      <span className="ds-meta-label">Current step</span>
      <h4>{step?.title ?? "All tasks completed"}</h4>
      <p>{step?.detail ?? mission.expectedResult}</p>
    </section>
  );
}

export function AgentIdentity({ agent }: { agent: AgentData }) {
  return (
    <span className="ds-agent-identity">
      <span className="ds-agent-mark" aria-hidden="true">
        {agent.name.slice(0, 2).toUpperCase()}
      </span>
      <span>
        <strong>{agent.name}</strong>
        <small>
          {agent.role} · {agent.model}
        </small>
      </span>
    </span>
  );
}

export function AgentStatus({ agent }: { agent: AgentData }) {
  const status =
    agent.status === "working"
      ? "running"
      : agent.status === "error"
        ? "failed"
        : agent.status === "unavailable"
          ? "suspended"
          : "idle";
  return (
    <CompactStatus
      status={status}
      label={
        agent.status === "working"
          ? "Working"
          : agent.status === "available"
            ? "Available"
            : agent.status === "error"
              ? "Error"
              : "Unavailable"
      }
    />
  );
}

export function AttentionNotification({
  title,
  detail,
  children,
}: {
  title: string;
  detail: string;
  children?: ReactNode;
}) {
  return (
    <InlineAlert tone="warning" title={title}>
      <span>{detail}</span>
      {children && <span className="ds-attention-actions">{children}</span>}
    </InlineAlert>
  );
}

export function TestResult({ test }: { test: TestData }) {
  return (
    <div className="ds-test-result">
      <CompactStatus
        status={
          test.status === "passed" ? "completed" : test.status === "failed" ? "failed" : "idle"
        }
        label={
          test.status === "passed" ? "Passed" : test.status === "failed" ? "Failed" : "Not run"
        }
      />
      <span title={test.command}>{test.name}</span>
      <code>{test.duration}</code>
    </div>
  );
}

export function CodeChangeSummary({ files }: { files: FileReference[] }) {
  return (
    <div className="ds-change-summary">
      <span>{files.length} files changed</span>
      <span className="ds-positive">+{files.reduce((total, file) => total + file.added, 0)}</span>
      <span className="ds-negative">−{files.reduce((total, file) => total + file.removed, 0)}</span>
    </div>
  );
}

export function ReviewSummary({
  status,
  summary,
  children,
}: {
  status: "pending" | "accepted";
  summary: string;
  children?: ReactNode;
}) {
  return (
    <section className="ds-review-summary">
      <CompactStatus
        status={status === "accepted" ? "completed" : "idle"}
        label={status === "accepted" ? "Accepted in fixture" : "Ready for review"}
      />
      <h3>Review summary</h3>
      <p>{summary}</p>
      {children}
    </section>
  );
}

export function CheckpointState({ checkpoint }: { checkpoint: MissionData["checkpoint"] }) {
  return (
    <div className="ds-checkpoint">
      <Icon name="checkpoint" />
      <div>
        <strong>
          {checkpoint.verified ? "Checkpoint verified" : "Checkpoint awaiting verification"}
        </strong>
        <code>
          {checkpoint.id} · {checkpoint.commit}
        </code>
      </div>
    </div>
  );
}
