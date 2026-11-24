import { useState, type ReactNode } from "react";
import { Button, Input, Select, Textarea } from "../design-system/primitives/Controls";
import { LoadingIndicator } from "../design-system/components/Application";
import { useData, refreshData } from "../workspace/production/data";
import type { DockSession } from "../workspace/production/controller";
import type { AgentSession } from "../../types";
import { archiveProfile, saveProfile } from "./client";
import {
  newProfile,
  profileConfiguration,
  type Profile,
  type ProfileInput,
  type ProfileDetails,
  type Providers,
} from "./types";
import "./agents.css";

export function AgentManagement({
  projectId,
  dock,
  children,
}: {
  projectId: string;
  dock: DockSession;
  children?: ReactNode;
}) {
  const profiles = useData<Profile[]>(projectId, "agent-profiles?archived=true");
  const providers = useData<Providers>(projectId, "agent-profiles/providers");
  const [editing, setEditing] = useState<Profile | "new">();
  const [error, setError] = useState("");
  const [archived, setArchived] = useState(false);
  return (
    <div className="ws-scroll asf-agents" aria-label="Agent management">
      <header className="asf-agent-toolbar">
        <h2>Agents</h2>
        <Button onClick={() => setEditing("new")} disabled={!providers.value?.providers.length}>
          New agent
        </Button>
        <label>
          <input
            type="checkbox"
            checked={archived}
            onChange={(e) => setArchived(e.target.checked)}
          />{" "}
          Include archived
        </label>
      </header>
      {(error || profiles.error || providers.error) && (
        <p role="alert">{error || profiles.error || providers.error}</p>
      )}
      {editing ? (
        <ProfileEditor
          key={editing === "new" ? "new" : `${editing.id}:${editing.revision}`}
          projectId={projectId}
          profile={editing === "new" ? undefined : editing}
          providers={providers.value ?? { providers: [], roles: [] }}
          done={() => {
            setEditing(undefined);
            void refreshData(projectId, "agent-profiles?archived=true");
          }}
        />
      ) : (
        <>
          {!profiles.value ? (
            <LoadingIndicator label="Loading agent profiles" />
          ) : (
            profiles.value
              .filter((p) => archived || !p.archived)
              .map((profile) => (
                <section
                  key={profile.id}
                  className="asf-agent-row"
                  aria-label={`Agent ${profile.name}`}
                >
                  <div>
                    <strong>{profile.name}</strong>
                    <small>
                      {profile.id} · revision {profile.revision}
                    </small>
                    <p>
                      {profile.roles.join(", ")} · {profile.providerAdapterId} ·{" "}
                      {profile.modelId ?? "Provider default"}
                      {profile.reasoningEffort ? ` · ${profile.reasoningEffort}` : ""}
                    </p>
                    <p>
                      {profile.archived ? "Archived" : profile.enabled ? "Enabled" : "Disabled"} ·{" "}
                      {profile.automaticDelegation
                        ? "Automatic delegation eligible"
                        : "Explicit selection required"}
                    </p>
                    <p>
                      {profile.activeExecutions ?? 0} active executions ·{" "}
                      {providers.value?.providers.find((p) => p.id === profile.providerAdapterId)
                        ?.available
                        ? "CLI executable available"
                        : "CLI executable unavailable"}{" "}
                      · Model availability requires execution
                    </p>
                  </div>
                  <div className="asf-agent-actions">
                    <Button disabled={profile.archived} onClick={() => setEditing(profile)}>
                      Edit {profile.name}
                    </Button>
                    <Button
                      disabled={profile.archived}
                      onClick={() => {
                        setError("");
                        void saveProfile(
                          projectId,
                          { ...profileConfiguration(profile), enabled: !profile.enabled },
                          profile
                        )
                          .then(() => refreshData(projectId, "agent-profiles?archived=true"))
                          .catch((e) => setError(String(e)));
                      }}
                    >
                      {profile.enabled ? "Disable" : "Enable"}
                    </Button>
                    <Button
                      disabled={profile.archived}
                      onClick={() => {
                        setError("");
                        void archiveProfile(projectId, profile)
                          .then(() => refreshData(projectId, "agent-profiles?archived=true"))
                          .catch((e) => setError(String(e)));
                      }}
                    >
                      Archive
                    </Button>
                  </div>
                  <ProfileInspection projectId={projectId} profile={profile} dock={dock} />
                </section>
              ))
          )}
          {profiles.value?.length === 0 && (
            <p>
              No custom agents. Trusted CLI installations and legacy agents remain available in
              existing settings.
            </p>
          )}
          <p>
            Profiles reuse trusted server-configured CLI installations and their existing
            authentication. Model availability is verified only by execution. No credentials are
            stored here.
          </p>
        </>
      )}
      {children}
    </div>
  );
}

export function ProfileInspection({
  projectId,
  profile,
  dock,
}: {
  projectId: string;
  profile: Profile;
  dock?: DockSession;
}) {
  const [open, setOpen] = useState(false);
  return (
    <details onToggle={(e) => setOpen(e.currentTarget.open)}>
      <summary>Permissions, revisions and executions</summary>
      {open && <ProfileFacts projectId={projectId} profile={profile} dock={dock} />}
    </details>
  );
}
function ProfileFacts({
  projectId,
  profile,
  dock,
}: {
  projectId: string;
  profile: Profile;
  dock?: DockSession;
}) {
  const details = useData<ProfileDetails>(projectId, `agent-profiles/${profile.id}`);
  const revisions = useData<Profile[]>(projectId, `agent-profiles/${profile.id}/revisions`);
  const sessions = useData<AgentSession[]>(projectId, `agents/${profile.id}/sessions`);
  return (
    <div>
      {details.error && <p role="alert">{details.error}</p>}
      <p>{details.value?.modelValidation}</p>
      <p>{details.value?.hostEnforcement}</p>
      <h4>Effective permissions</h4>
      <pre tabIndex={0}>{JSON.stringify(details.value?.effectivePermissions ?? [], null, 2)}</pre>
      <h4>Revision history</h4>
      <ul>
        {revisions.value?.map((r) => (
          <li key={r.revision}>
            Revision {r.revision} · {r.updatedAt} · {r.providerAdapterId} ·{" "}
            {r.modelId ?? "Provider default"} · {r.enabled ? "enabled" : "disabled"}
          </li>
        ))}
      </ul>
      <p>Newest 100 revisions. Earlier revisions remain available through the API cursor.</p>
      <h4>Recent executions</h4>
      {sessions.value?.map((s) => (
        <p key={s.id}>
          {s.role} · {s.status}{" "}
          <Button onClick={() => dock?.open("console", { projectId, resourceId: s.id })}>
            Session #{s.id}
          </Button>
        </p>
      ))}
    </div>
  );
}

export function ProfileEditor({
  projectId,
  profile,
  providers,
  done,
}: {
  projectId: string;
  profile?: Profile;
  providers: Providers;
  done: () => void;
}) {
  const [input, setInput] = useState<ProfileInput>(() =>
    profile
      ? structuredClone(profileConfiguration(profile))
      : newProfile(providers.providers[0]?.id ?? "")
  );
  const [error, setError] = useState("");
  const [saving, setSaving] = useState(false);
  const provider = providers.providers.find((p) => p.id === input.providerAdapterId);
  const set = <K extends keyof ProfileInput>(key: K, value: ProfileInput[K]) =>
    setInput((i) => ({ ...i, [key]: value }));
  const fs = input.permissions.filesystem ?? { read: ["**"], write: [], deny_write: [] };
  const commands = input.permissions.commands ?? {
    mode: "restricted" as const,
    allow: [],
    deny: [],
  };
  const tools = input.permissions.tools ?? { allow: [], deny: [] };
  return (
    <form
      className="asf-profile-editor"
      aria-label="Agent profile editor"
      onSubmit={(e) => {
        e.preventDefault();
        setError("");
        setSaving(true);
        void saveProfile(projectId, input, profile)
          .then(done)
          .catch((e) => setError(String(e)))
          .finally(() => setSaving(false));
      }}
    >
      <h3>
        {profile ? `Edit ${profile.name} · revision ${profile.revision}` : "New agent profile"}
      </h3>
      {error && (
        <p role="alert">
          {error} Your draft is retained. Reload the profile before resolving a stale revision.
        </p>
      )}
      <Input
        label="Agent name"
        value={input.name}
        maxLength={100}
        required
        onChange={(e) => set("name", e.target.value)}
      />
      <Input
        label="Description"
        value={input.description}
        maxLength={2000}
        onChange={(e) => set("description", e.target.value)}
      />
      <Select
        label="Provider installation"
        value={input.providerAdapterId}
        onChange={(e) => {
          set("providerAdapterId", e.target.value);
          set("reasoningEffort", null);
          set("modelId", null);
        }}
      >
        {providers.providers.map((p) => (
          <option key={p.id} value={p.id}>
            {p.id} · {p.kind} · {p.available ? "Executable available" : "Executable unavailable"}
          </option>
        ))}
      </Select>
      <Input
        label="Model identifier"
        disabled={!provider?.modelSelection}
        value={input.modelId ?? ""}
        maxLength={160}
        placeholder="Empty uses provider default"
        onChange={(e) => set("modelId", e.target.value || null)}
        hint="Manual identifier. Syntax validation does not prove account access or quota. No automatic model fallback."
      />
      {provider?.reasoningSelection && (
        <Select
          label="Reasoning effort"
          value={input.reasoningEffort ?? ""}
          onChange={(e) => set("reasoningEffort", e.target.value || null)}
        >
          <option value="">Provider default</option>
          {["low", "medium", "high", "xhigh", "max", "ultra"].map((s) => (
            <option key={s}>{s}</option>
          ))}
        </Select>
      )}
      <fieldset>
        <legend>Supported roles</legend>
        {providers.roles.map((role) => (
          <label key={role.id}>
            <input
              type="checkbox"
              checked={input.roles.includes(role.id)}
              onChange={(e) =>
                set(
                  "roles",
                  e.target.checked
                    ? [...input.roles, role.id]
                    : input.roles.filter((r) => r !== role.id)
                )
              }
            />
            {role.name}
          </label>
        ))}
      </fieldset>
      <Textarea
        label="Agent instructions"
        value={input.instructions}
        maxLength={16384}
        rows={7}
        onChange={(e) => set("instructions", e.target.value)}
        hint="Instructions cannot override application, project, role or host restrictions."
      />
      <fieldset>
        <legend>Requested restrictions</legend>
        <p>
          These controls only narrow inherited permissions. Reviewer roles remain read-only. Network
          policy is advisory, not an OS firewall.
        </p>
        <CommaList
          label="ASF file-context read scopes"
          values={fs.read}
          hint="Controls ASF-provided file context and transferred evidence. Native CLI reads remain subject to its own sandbox and the host."
          onChange={(values) =>
            set("permissions", {
              ...input.permissions,
              filesystem: { ...fs, read: values },
            })
          }
        />
        <CommaList
          label="Write scopes (comma separated)"
          values={fs.write}
          onChange={(values) =>
            set("permissions", {
              ...input.permissions,
              filesystem: { ...fs, write: values },
            })
          }
          hint="Empty denies writes. Managed worktree and protected-path restrictions still apply."
        />
        <CommaList
          label="Denied write scopes"
          values={fs.deny_write}
          onChange={(values) =>
            set("permissions", {
              ...input.permissions,
              filesystem: { ...fs, deny_write: values },
            })
          }
        />
        <Select
          label="Command policy"
          value={commands.mode}
          onChange={(e) =>
            set("permissions", {
              ...input.permissions,
              commands: { ...commands, mode: e.target.value as typeof commands.mode },
            })
          }
        >
          <option value="restricted">Approved programs</option>
          <option value="denied">No commands</option>
        </Select>
        <CommaList
          label="Approved programs"
          values={commands.allow}
          onChange={(values) =>
            set("permissions", {
              ...input.permissions,
              commands: { ...commands, allow: values },
            })
          }
        />
        <CommaList
          label="Denied programs"
          values={commands.deny}
          onChange={(values) =>
            set("permissions", {
              ...input.permissions,
              commands: { ...commands, deny: values },
            })
          }
        />
        <CommaList
          label="Allowed MCP tools"
          values={tools.allow}
          onChange={(values) =>
            set("permissions", {
              ...input.permissions,
              tools: { ...tools, allow: values },
            })
          }
        />
        <CommaList
          label="Denied MCP tools"
          values={tools.deny}
          onChange={(values) =>
            set("permissions", {
              ...input.permissions,
              tools: { ...tools, deny: values },
            })
          }
        />
        <label>
          <input
            type="checkbox"
            checked={input.permissions.network?.mode === "deny"}
            onChange={(e) =>
              set("permissions", {
                ...input.permissions,
                network: { mode: e.target.checked ? "deny" : "allow" },
              })
            }
          />
          Deny network operations in application policy
        </label>
        <p>
          Publication, integration, terminals and protected configuration are not granted by a
          profile. Existing Rust authorization remains required.
        </p>
      </fieldset>
      <Input
        label="Concurrent executions"
        type="number"
        min={1}
        max={32}
        value={input.maxConcurrency}
        onChange={(e) => set("maxConcurrency", Number(e.target.value))}
        hint={`Installation ceiling: ${provider?.maxConcurrency ?? "unavailable"}. Workflow, project and application limits also apply.`}
      />
      <Input
        label="Execution deadline (seconds)"
        type="number"
        min={10}
        max={3600}
        value={input.timeoutSecs}
        onChange={(e) => set("timeoutSecs", Number(e.target.value))}
      />
      <label>
        <input
          type="checkbox"
          checked={input.enabled}
          onChange={(e) => set("enabled", e.target.checked)}
        />
        Enabled for new executions
      </label>
      <label>
        <input
          type="checkbox"
          checked={input.automaticDelegation}
          onChange={(e) => set("automaticDelegation", e.target.checked)}
        />
        Eligible for automatic delegation
      </label>
      <label>
        <input
          type="checkbox"
          checked={input.preferred}
          onChange={(e) => set("preferred", e.target.checked)}
        />
        Preferred for assigned roles
      </label>
      <div className="asf-agent-actions">
        <Button type="submit" tone="primary" loading={saving}>
          Save agent profile
        </Button>
        <Button onClick={done} disabled={saving}>
          Close editor
        </Button>
      </div>
      {profile && <ProfileInspection projectId={projectId} profile={profile} />}
    </form>
  );
}

function CommaList({
  label,
  values,
  onChange,
  hint,
}: {
  label: string;
  values: string[];
  onChange: (values: string[]) => void;
  hint?: string;
}) {
  const [draft, setDraft] = useState(() => values.join(", "));
  return (
    <Input
      label={label}
      value={draft}
      hint={hint}
      onChange={(e) => {
        setDraft(e.target.value);
        onChange(
          e.target.value
            .split(",")
            .map((s) => s.trim())
            .filter(Boolean)
        );
      }}
    />
  );
}
