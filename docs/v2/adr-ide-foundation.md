# ADR — ASF-owned workbench with CodeMirror 6

Date: 2026-10-27. Phase: V2-P01. Status: **selected for implementation planning; production integration and release qualification pending their gates**. Decision evidence: [IDE evaluation](ide-evaluation.md), raw experiments (historical record held privately), [architecture contracts](architecture-target.md).

## Decision

Use CodeMirror 6, its official `@codemirror/lsp-client`, `@codemirror/merge` and Dockview core inside the existing React/TypeScript/Vite frontend. Keep Rust authoritative for files, process trees, terminal sessions, permissions, Git, workflows and persistence. Implement a native ASF extension contribution contract; do not advertise VS Code VSIX compatibility. Approved external LSP servers and DAP adapters run as Rust-supervised helpers, with bundled runtimes where necessary. There is no secondary Node IDE server.

Evaluated pins are view 6.43.15, state 6.7.6, lsp-client 6.3.0, merge 6.12.3, Dockview 8.7.0. These pins establish the P01 result, not an indefinite production update policy. An adapter boundary between workspace/document ownership and editor views preserves a feasible future editor replacement without duplicating files, Git or execution state.

## Context

ASF V2 requires a distinct orchestrator-centered workspace and a complete IDE before release. The existing application ships Rust-hosted embedded React assets and does not require manually installed Rust/Node. A foundation that silently moves file/process/extension authority into another IDE backend violates that architecture even if its user interface is polished. A widget that merely resembles VS Code also fails the complete-product requirement.

The existing security, Runtime PTY, project data and V1 compatibility guarantees must survive. P01 therefore compares complete proposed stacks with common real-file/protocol/process experiments, not screenshots or package popularity. P02–P14 are outside this implementation authorization.

## Alternatives

| Alternative | Evidence | Decision |
| --- | --- | --- |
| CodeMirror + official LSP + merge + Dockview + Rust | All ten common prototype scenarios pass; native YAML actually contributes syntax; project-attached LSP; measured static asset/resource profile | Accepted |
| Monaco 0.57 + native LSP + Dockview + Rust | Nine common scenarios pass, including LSP/DAP/PTY; native global synchronizer attempts a foreign model, blocked by Rust; YAML worker integration fails; larger measured full-entry bundle | Credible fallback, not selected |
| Monaco + TypeFox/Codingame VS Code services | Current metadata and documentation reviewed; richer compatibility possible, coordinated API/service versions and host trust implications | Not directly prototyped; benefits cannot override observed finalist evidence |
| Theia full workbench/Node | Genuine filesystem provider reads owned outside-project file directly; full editor/protocol/plugin capabilities documented; Rust service authority replacement not demonstrated | Reject default architecture; reassess only with a proven mediated service/plugin design |
| Theia browser-only | Current 1.76 documentation supports bundled web extensions, OPFS; no terminal or runtime extension installation without backend | Does not satisfy the full target as-is; adding ASF services is a substantial separate integration |
| OpenVSCode upstream distribution | Actual official Linux ELF archive inspected and native Windows launch fails; only Linux upstream archives; custom workbench/authority integration unverified | Reject current distribution for required native four-target product |

Theia is customizable through public contributions; no claim that it requires a permanent fork for every layout change is made. The rejection concerns the authority/process/distribution costs under ASF's constraints. OpenVSCode's missing native archives do not imply a source port is impossible. Neither unexecuted full IDE receives an invented performance measurement.

## Rationale

CodeMirror's explicit state/view and per-workspace LSP APIs fit an ASF-owned document and session model. The same Rust bridge demonstrated external TypeScript diagnostics/hover, debugpy breakpoints/variables, real existing ConPTY lifecycle and real disk conflict handling in both editors. Thus DAP/terminal feasibility comes from the complete stack and reusable backend, not an editor feature claim.

CodeMirror's native YAML module activated and deactivated under persistent install state, establishing a legitimate extension path without inheriting another product's marketplace contract. Its measured build/resource profile was smaller for the tested configuration, although Monaco included more built-in languages/features and a selectively imported Monaco build was not benchmarked. Performance is supporting evidence, not the sole reason.

The scorecard yields **3.825/5 CodeMirror**, 3.400 Monaco, 3.250 Theia and 3.125 OpenVSCode. Scores are engineering judgments with source/test support and explicit confidence; screened-out runtime/authority blockers take precedence over totals. Confidence is high for local feasibility and medium for completing all production IDE infrastructure.

## Consequences and owned work

ASF must build the application workbench and complete debugger UI, language-server lifecycle, extension manager, file conflict/recovery and document ownership. CodeMirror does not provide a VS Code workbench, terminal backend, DAP UI or arbitrary marketplace compatibility. This is substantial work in P03–P06, already required by the approved scope. No feature is removed to make the selection pass.

Native editor modules can contribute syntax, commands, hooks and views through a defined API. Initially only trusted bundled/imported code may execute; untrusted code requires isolated-origin capability mediation or separately qualified OS isolation. Same-origin workers and Git worktrees are not OS security sandboxes. Node/Python helpers retain OS user privileges and require explicit trust plus Rust ownership of every ASF-mediated action.

Default CodeMirror LSP document tracking supports one view per URI. Same-document split views require a custom documented Workspace implementation and one authoritative document buffer. P01 tests distinct documents in splits; it does not qualify this remaining production case. Large-tree rendering, full keyboard/screen-reader/IME behavior, both themes, all editor commands and complete Rust/TypeScript/Python journeys remain future gates.

The Windows PTY probe observes a portable-pty 0.9.0 termination diagnostic despite actual exit. The experiment reconciles durable state without suppressing the diagnostic. A production fix or dependency update and platform tests are required. Rust LSP was unavailable locally; the absent rust-analyzer component is recorded, not simulated.

## Dependency, packaging and security implications

React/Vite remain; P01 introduces no production dependencies. Experimental dependencies live in their own lockfile. The experiment pins Vite 6.4.5 and overrides Monaco's DOMPurify to 3.4.17 after current advisories; production dependency maintenance is a separate task. Dockview enterprise is excluded. License notices for editor modules, Node/Python, TypeScript tooling and debugpy's vendored components must accompany any actual distribution.

The representative Windows package embeds assets in Rust and bundles Node/Python tool runtimes. Packaging execution (historical record held privately) defines what was demonstrated; native Linux/macOS execution is pending. Exact target recipes are documented without changing release workflows. Offline launch, data outside installation and process cleanup must be verified on every release target before V2.0.

The [integration contracts](architecture-target.md) define one loopback authority, authenticated project-bound WebSockets, controlled files, protocol limits, process generations, environment/secret policy, extension trust and persistence. Probe endpoints, test globals, fixture allowlists and fixed debug programs are experimental and must not be copied into production as a finished service.

## Prerequisites for production implementation

1. Preserve the P00 contracts and obtain the required exact-commit CI. Neither unpushed phase has a remote run.
2. P02 must establish accessible tokens/components, both themes and real design constraints; P01 does not authorize starting it.
3. P03/P04 define project/session/document identities, schema migrations, recovery and versioned layout ownership.
4. P05/P06 qualify duplicate views, race-safe files, complete LSP/DAP commands, trusted extension packaging/import/update and approved development shells; resolve the PTY termination path.
5. P06/P14 run actual Rust/TypeScript/Python IDE journeys and native Windows/Linux/macOS x64/ARM64 packaging/cleanup tests as applicable. P10/P14 qualify crash/reconnect behavior and every mandatory release scenario.

## Revisit conditions

Reopen this ADR if real P05/P06 prototypes cannot deliver required editor accessibility, duplicate-view semantics, debugging or native extension capabilities; if the expected infrastructure burden exceeds a demonstrably Rust-mediated Theia alternative; if a maintained Monaco integration demonstrates scoped disposal, compatible plugins and materially better developer UX; or if an upstream license/security/maintenance change undermines the selected stack. A future requirement for broad VS Code extension compatibility would materially change this decision and requires explicit product approval, not an implicit promise.

P01 evidence supports the choice, not a release. **ASF V2.0: NO-GO.**
