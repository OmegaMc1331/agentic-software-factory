export interface PolicyScope {
  preset?: "custom" | "implementation" | "read_only" | "review" | "documentation";
  filesystem?: { read: string[]; write: string[]; deny_write: string[] };
  commands?: { mode: "restricted" | "denied" | "unrestricted"; allow: string[]; deny: string[] };
  network?: { mode: "allow" | "deny" };
  tools?: { allow: string[]; deny: string[] };
}
export interface ProfileInput {
  name: string;
  description: string;
  providerAdapterId: string;
  modelId: string | null;
  reasoningEffort: string | null;
  roles: string[];
  instructions: string;
  permissions: PolicyScope;
  maxConcurrency: number;
  timeoutSecs: number;
  enabled: boolean;
  automaticDelegation: boolean;
  preferred: boolean;
}
export interface Profile extends ProfileInput {
  activeExecutions?: number;
  id: string;
  projectId: string;
  revision: number;
  createdAt: string;
  updatedAt: string;
  archived: boolean;
}
export interface Provider {
  id: string;
  kind: string;
  available: boolean;
  modelSelection: boolean;
  modelDiscovery: boolean;
  reasoningSelection: boolean;
  maxConcurrency: number;
  roles: string[];
}
export interface Providers {
  providers: Provider[];
  roles: { id: string; name: string }[];
}
export interface ProfileDetails {
  profile: Profile;
  effectivePermissions: { role: string; policy: Record<string, unknown> }[];
  eligible: boolean;
  hostEnforcement: string;
  modelValidation: string;
}
export interface ExecutionSnapshot {
  schemaVersion: number;
  profile: Profile;
  role: string;
  providerKind: string;
  requestedModel: string | null;
  effectivePolicy: Record<string, unknown>;
  selectedAt: string;
}
export interface Handoff {
  id: string;
  taskId: number;
  runId: number;
  state: string;
  boundary: string;
  fromAgent: string | null;
  fromSessionId: number | null;
  target: Profile;
  reason: string;
  requestedAt: string;
  replacementSessionId: number | null;
  evidence: unknown;
}
export function profileConfiguration(profile: Profile): ProfileInput {
  const {
    name,
    description,
    providerAdapterId,
    modelId,
    reasoningEffort,
    roles,
    instructions,
    permissions,
    maxConcurrency,
    timeoutSecs,
    enabled,
    automaticDelegation,
    preferred,
  } = profile;
  return {
    name,
    description,
    providerAdapterId,
    modelId,
    reasoningEffort,
    roles,
    instructions,
    permissions,
    maxConcurrency,
    timeoutSecs,
    enabled,
    automaticDelegation,
    preferred,
  };
}
export function newProfile(provider: string): ProfileInput {
  return {
    name: "",
    description: "",
    providerAdapterId: provider,
    modelId: null,
    reasoningEffort: null,
    roles: ["worker"],
    instructions: "",
    permissions: {
      filesystem: { read: ["**"], write: [], deny_write: [] },
      commands: { mode: "restricted", allow: ["git"], deny: [] },
      tools: { allow: [], deny: ["*"] },
    },
    maxConcurrency: 1,
    timeoutSecs: 300,
    enabled: true,
    automaticDelegation: false,
    preferred: false,
  };
}
