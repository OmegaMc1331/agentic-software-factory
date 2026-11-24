# Production debugger service

`factory-ide::dap::DebugSession` owns a supervised adapter and its project/root/configuration identity. API calls cannot attach to a session in another project. A debugger is distinct from a workflow, an agent session and a Dockview tab.

## Profiles and launch authorization

Approve a `kind: "dap"` tool profile, then a stored launch configuration containing `id`, `name`, `profile_id`, `root_id`, `program: FileRef` and an argument object. Start requires a separate explicit user action. Rust resolves the root/program through P05, supplies the working directory and internal console, filters the environment and rejects browser-supplied PID/attach addresses, executable overrides, env files, external terminals and launch tasks.

Launch configurations may supply debugger-specific data such as program arguments or `stopOnEntry`; sensitive adapter defaults belong to the explicitly trusted profile's `settings.launch`. A debug target can execute code with OS permissions. Manual starts are not automatic trust in repository-provided configuration.

Native Rust build executables can be hardlinked into Cargo's `target` directory. The process-only P05 launch-path verifier permits an ordinary hardlinked launch target after component/leaf checks; it does not expose content through the editor or permit hardlinked editor writes. Standard P05 reads/writes retain their stricter hardlink policy.

## Protocol sequence

Rust starts the adapter, sends `initialize`, records capabilities, begins `launch`, waits for `initialized`, configures breakpoints, sends `configurationDone` where supported and awaits launch completion. Adapter startup alone is not reported as successful debugging. Correlated requests support threads, stack, scopes, variables, continue, pause and stepping. Evaluation requires explicit confirmation and an advertised capability. Unsupported conditional breakpoints fail rather than being shown as verified.

Stopped/continued/output/exit events feed the shared debugger model. CodeMirror contributes a breakpoint gutter and execution-line decoration; keyboard-accessible toggle and panel controls offer alternatives to gutter clicking. The workspace hosts stack, variables, breakpoint controls and a connected debug console. Source paths are resolved through P05; outside-workspace sources are marked unavailable.

## TCP adapters and js-debug targets

CodeLLDB/js-debug can use a profile with `transport: "tcp"` and `${port}` in fixed arguments. Rust chooses a loopback port, launches the approved adapter, connects under its timeout and owns termination. The ephemeral port reservation/launch interval is not a defense against a hostile same-user OS process; no additional browser-accessible service is advertised.

The official js-debug standalone server requests a second DAP connection for its already launched target. An explicitly approved profile setting `target_sessions: true` enables one restricted target connection to the same owned adapter endpoint. Only the adapter's opaque pending target identifier, type and display name are accepted; no new executable, program path, command or arbitrary PID is accepted. Initialization and breakpoints are replayed to that channel. Additional child targets and arbitrary reverse `runInTerminal`/process requests are refused. The qualification profile disables automatic child-process attachment.

Generic attach to browser-supplied PIDs is disabled. The restricted js-debug target channel is not general process attachment.

## Cleanup

Explicit Stop sends `disconnect` with debuggee termination and then terminates the supervised tree. Crashes and protocol deadlines are visible failures. Windows Job Objects kill owned descendants; process groups are used on Unix. Hiding/moving the view keeps a controlled session alive; it does not stop unrelated workflows. Browser reload may reconnect to a still-owned session. Backend restart does not relaunch a debugger or debuggee from layout metadata. This is independent of P10 mission suspension.

Real representative qualification uses debugpy, js-debug and CodeLLDB. Language adapters/toolchains remain optional installations. Windows results do not prove Linux/macOS behavior.

Primary references retrieved 2026-10-29: [DAP specification](https://microsoft.github.io/debug-adapter-protocol/specification), [DAP overview](https://microsoft.github.io/debug-adapter-protocol/overview), [debugpy](https://github.com/microsoft/debugpy), [js-debug standalone releases](https://github.com/microsoft/vscode-js-debug/releases), [js-debug target protocol source](https://github.com/microsoft/vscode-js-debug/blob/v1.140.0/src/dapDebugServer.ts), [CodeLLDB](https://github.com/vadimcn/codelldb).
