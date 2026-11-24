# Settings architecture

Settings consolidates presentation and navigation while existing services retain configuration ownership.

| Scope | Authority | Behavior |
| --- | --- | --- |
| Application | Existing application WorkspaceStore SQLite file | Typed theme, density, editor font/tab/wrap, new-session default layout, toast and optional OS-notification preferences |
| Workspace | P04 version-3 document | Open views, sessions, docking, selected resources and presentation |
| Project | Existing validated TOML/configuration, P06 services and P11 memory settings | Read-only summary plus direct access to the actual authorized editor/service |
| Agent profile | P09 project profile/revision store | Provider/model, instructions, roles, capacity and effective permission inspection |

The WorkspaceStore schema upgrades additively from 1 to 2, retaining all existing workspace rows. It adds a singleton preferences envelope with its own revision. No project policy or credentials are copied there. Unknown versions/corrupt payloads fail explicitly. Existing acknowledged workspace theme migrates once if application preferences are absent; the browser theme cache is a boot-time presentation hint. Subsequently the acknowledged application preference is authoritative. Theme controls in Projects and Workspace use that same endpoint.

`GET/PUT /api/application/preferences` uses existing authentication, Host/Origin checks and mutation CSRF. Payloads are limited to 4 KiB. The Rust schema rejects unknown fields, invalid enum values, out-of-range font/indentation and stale revisions. Save uses an immediate transaction/conditional revision and returns only after commit. React retains a rejected draft and offers reload; it never reports a failed save as success. Editor preferences reconfigure an existing CodeMirror compartment, retaining content, undo history and write permissions. Default layout applies to newly created sessions, not an unsolicited rewrite of restored layouts.

`GET /api/projects/:id/settings` is a read-only facade over the current authoritative project configuration. It returns revision, scheduler/routing limits, role/ceiling summaries and counts. It deliberately excludes provider commands/arguments, environment values, tool headers and instructions. Configured summaries are not a new effective-policy resolver: P09's effective permission display remains authoritative, including inherited denials. The legacy advanced editor retains its existing authenticated configuration contract; graphical Settings does not duplicate it.

Fifteen searchable categories identify Application, Project, Agent profile or Workspace scope. Search matches actual category names/descriptions and focuses a real control. Tool profiles, extensions and memory settings reuse their existing Rust services and concurrency checks. Unsupported controls are not invented. Advanced TOML configuration is labelled truthfully. Changing memory policy never starts a suspended Worker or changes P10's manual-resume policy.

Browser notifications are disabled by default. Permission is requested only by the explicit settings action and only where supported. A denied browser permission leaves in-app notifications functional. Preference writes cannot grant execution, MCP, filesystem, provider or publication capabilities.
