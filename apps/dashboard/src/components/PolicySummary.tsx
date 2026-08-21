import type { PolicyView } from "../types";

function scopeList(scopes: string[]): string {
  if (scopes.length === 0) return "none";
  return scopes.join(", ");
}

function filesystemLabel(policy: PolicyView): string {
  switch (policy.filesystemMode) {
    case "read_only":
      return "Read-only";
    case "restricted":
      return "Restricted";
    default:
      return "Open";
  }
}

function networkLabel(policy: PolicyView): string {
  return policy.network === "allow" ? "Allowed" : "Denied";
}

function environmentLabel(policy: PolicyView): string {
  return policy.environmentMode === "filtered" ? "Restricted" : "Full inheritance";
}

function gitLabel(policy: PolicyView): string {
  const allowed = policy.gitAllowed;
  if (allowed.includes("commit_in_task_worktree")) return "Task worktree only";
  if (allowed.length === 0) return "None";
  return allowed.join(", ");
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="inspector-row">
      <span className="inspector-label">{label}</span>
      <span className="inspector-value">{children}</span>
    </div>
  );
}

/**
 * The Permissions section shown in the Role and Agent inspectors. It renders
 * the same effective policy Factory enforces at execution time; network is
 * explicitly marked advisory because Factory cannot sandbox a launched
 * process's network on the current OS.
 */
export function PolicySummary({ permissions }: { permissions: PolicyView | undefined | null }) {
  if (!permissions) {
    return (
      <section className="policy-section">
        <h4>Permissions</h4>
        <p className="inspector-hint">No policy information available.</p>
      </section>
    );
  }
  return (
    <section className="policy-section" aria-label="Permissions">
      <h4>Permissions</h4>
      {permissions.permissive && (
        <p className="policy-permissive-note">
          No policy configured — permissive defaults apply. Configure a role policy to restrict what
          Factory permits.
        </p>
      )}
      <Row label="Filesystem">
        {filesystemLabel(permissions)}
        {permissions.filesystemMode !== "read_only" && (
          <span className="policy-scopes"> — write: {scopeList(permissions.writeScopes)}</span>
        )}
        {permissions.denyWriteScopes.length > 0 && (
          <span className="policy-scopes"> — deny: {scopeList(permissions.denyWriteScopes)}</span>
        )}
      </Row>
      <Row label="Read">
        {scopeList(permissions.readScopes.length > 0 ? permissions.readScopes : ["repository"])}
      </Row>
      <Row label="Commands">{permissions.commandsMode.replaceAll("_", " ")}</Row>
      {permissions.commandsMode !== "unrestricted" && (
        <Row label="Allowed commands">{scopeList(permissions.commandsAllow)}</Row>
      )}
      <Row label="Network">
        {networkLabel(permissions)}{" "}
        <span className="policy-advisory">(advisory — not process-enforced)</span>
      </Row>
      <Row label="Environment">{environmentLabel(permissions)}</Row>
      <Row label="Git">{gitLabel(permissions)}</Row>
      <p className="policy-invariant-note">
        Dangerous Git operations (push, force push, branch deletion, reset, remotes) are always
        denied; task agents cannot touch Factory's integration branches.
      </p>
      <p className="policy-source">Policy source: {permissions.source}</p>
    </section>
  );
}
