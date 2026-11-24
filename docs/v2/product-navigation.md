# Product navigation

V1 remains `/`; the production IDE is `/workspace.html`, and `/projects.html` owns registration, creation, cloning and runtime management. Design previews and deterministic fixtures remain separate entries. Existing users restore their acknowledged workspace; a new workspace offers the real project manager and application settings without inventing sample activity.

Dockview remains the layout owner. Open projects, workspace sessions and views are separate identities. Switching or hiding presentation tabs never closes an owning lifecycle session. The explicit session-close control uses P10, including its consequence confirmation, durable suspension and manual reopening/resume. Restoring presentation state does not submit objectives, run terminal commands, resume missions, change Git or publish.

The Activity Rail exposes existing Explorer, Search, Source Control, Agents, Memory, Missions, Debug, Terminal, Extensions and Settings services. Focus, Code, Agents and Review presets retain open resources and drafts. The top bar provides project selection, Commands, Notifications, Settings and the existing project-manager/V1 links. Settings can open without a project; project controls retain their explicit owning identity.

## Commands and keyboard

The shell constructs one typed registry over existing actions. The palette and searchable shortcut reference read that same registry. Commands explain missing project/editor context, capture the selected view's project (including pinned views), and preserve backend authorization. They cannot provide arbitrary shell commands or executable paths.

| Binding | Actual action |
| --- | --- |
| Ctrl/Cmd+Shift+P | Command palette |
| Ctrl/Cmd+P | Authorized file quick-open |
| Ctrl/Cmd+Shift+F | Project content search |
| Ctrl/Cmd+, | Settings |
| Ctrl/Cmd+S / Shift+S | Save current / all open documents |
| Alt+Shift+O | Orchestrator |
| Alt+Shift+N | Notifications |
| F6 / Shift+F6 | Dock group navigation |
| Alt+PageUp / PageDown | Previous / next view |
| Alt+Shift+W | Close presentation view with draft safeguards |
| Alt+Shift+1–4 | Focus / Code / Agents / Review |
| Shift+F10 | Active view actions, including keyboard docking alternatives |

The registry matches exact modifiers, ignores composing/default-prevented events and detects duplicate bindings. A focused terminal retains its input; only explicitly terminal-eligible global commands are intercepted. CodeMirror handles its own local find/edit commands first. Browser Ctrl+W, Ctrl+Tab and Ctrl+L remain available. User-editable shortcut bindings are not advertised: the shipped map is fixed and searchable.

Inside CodeMirror, Tab retains indentation behavior. Its installed default keymap toggles Tab focus navigation with Ctrl+M (Shift+Alt+M on macOS); use that before tabbing out of the editor. The keyboard-only acceptance uses this actual editor binding rather than changing the indentation keymap or forcing focus from automation.

## Resource links

Notifications carry backend event provenance and scoped run IDs. Navigation verifies the registered open project and resolves the actual run before changing presentation. A changed navigation request/session invalidates an outstanding resolution; it does not redirect a newer selection or mark an unopened target read. Missing targets produce an error. Mission details link to the existing scoped Review and Source Control views. File/review/memory links retain P05 root/path/revision and existing provenance authorization.

Opening a view is not permission to execute its actions. Backend ownership, Origin/CSRF, effective policy, Git coordination and lifecycle fencing remain mandatory. See the P13 test matrix and delivery for executed journeys and remaining qualification limits.
