# V2-P01 — IDE foundation evaluation

Evaluation date: **2026-10-27**. Base: `the historical private baseline`, branch `codex/v2-p01-ide-evaluation`. This report separates documented capabilities, observed experiments and future implementation. [Reproduction instructions](../../experiments/v2-ide-evaluation/README.md), [integration contracts](architecture-target.md), [ADR](adr-ide-foundation.md) and delivery report (historical record held privately) accompany it.

## Executive decision

Select **CodeMirror 6 + its official LSP client + merge views + Dockview, inside an ASF-owned React workbench, with Rust-owned files/processes/PTY/LSP/DAP/persistence**. Native ASF extensions are the compatibility contract. VS Code extension compatibility is not promised or required by the approved product specification.

The decisive evidence is successful real-file editing/conflict recovery, both protocols, the existing ConPTY path, native YAML activation and persisted docking, with project-specific LSP attachment and a small static deployment surface. Monaco also passed nine common functional scenarios, including real LSP and DAP, but the tested YAML integration failed with its new worker contract and its native LSP synchronizer attempted to send a different project's model. Rust refused that request. Those costs matter more than visual resemblance to VS Code.

This selects an architecture for subsequent implementation. It does not deliver a production IDE. Complete debugging UI, arbitrary extension import, multi-project product runtime, Rust-language qualification and all V2 acceptance gates remain mandatory. The packaging outcome and local gate status are recorded below and in the delivery report; no release is authorized.

## Baseline and preserved contracts

The initial working tree was clean on `codex/v2-p00-baseline`, HEAD `aec5c073…`, containing P00 commits `a8267af4…` and `aec5c073…`. A new branch was created at that HEAD; neither baseline branch nor historical data was reset. P00's complete delivery, product/implementation/acceptance documents, architecture, development/security/API docs, Rust packages, dashboard, installers and CI/release workflows were inspected.

`architecture-target.md` did not exist in P00; its actual architecture document was `current-architecture.md`. The new target document contains P01 contracts. Experiments are a separate Cargo workspace and npm package. They do not add production routes, migrations or dashboard dependencies. The unchanged production API security module is source-included to exercise the same browser authentication/Host/Origin/CSRF rules. Factory Runtime and factory-proc Job Objects are reused. Owned fixture roots are checked before initialization; real project data is never used as a write fixture.

Read-only GitHub Actions lookup for the exact P00 HEAD returned zero runs. **P00 CI: PENDING — NO REMOTE RUN. P01 CI: PENDING — NO REMOTE RUN.** The green older reference run `40383707233` does not qualify either new commit. P01's local decision is not release validation.

## Versions and source provenance

All source URLs in this report were retrieved or checked on **2026-10-27**. npm package versions and licenses are captured in registry metadata (historical record held privately); exact transitive versions/integrities are pinned in the experiment lockfiles. Current documentation may move after this date; local package/source behavior is the experiment's authority.

| Component | Version evaluated | License / evidence boundary |
| --- | --- | --- |
| Monaco Editor | 0.57.0 | MIT; actual editor, native LSP SDK and workers tested |
| monaco-yaml | 5.5.1 | MIT; actual install/activation attempted; incompatible worker behavior observed |
| monaco-languageclient | 11.0.3 | MIT; metadata/docs only; not a working prototype claim |
| Codingame monaco-vscode-api | 38.0.4 | MIT; researched, not installed/prototyped; TypeFox package uses a different coordinated 37.6.x family |
| CodeMirror view / state / commands / language | 6.43.15 / 6.7.6 / 6.11.1 / 6.13.1 | MIT; tested modules |
| CodeMirror LSP / merge / lint | 6.3.0 / 6.12.3 / 6.9.7 | MIT; actual protocol, diff and diagnostics |
| CodeMirror JavaScript / Python / YAML | 6.2.5 / 6.2.1 / 6.1.3 | MIT; TS/Python syntax included; YAML contribution activated |
| Eclipse Theia | 1.76.0 | EPL-2.0 OR GPL-2.0-only WITH Classpath-exception-2.0; real filesystem service spike, no full workbench |
| OpenVSCode Server | openvscode-server-v1.109.5 | MIT Code OSS lineage; actual official archive inspected; no running workbench |
| Dockview / dockview-core | 8.7.0 | MIT; enterprise package not used |
| React / xterm | 18.3.1 / 6.0.0 | MIT; same families as ASF baseline |
| TypeScript language server / TypeScript | 6.0.2 / 5.7.3 | Apache-2.0; real external server in both finalists |
| Pyright / debugpy | 1.1.414 / 1.8.22 | MIT top-level; debugpy includes separately licensed vendored code |
| Vite / TypeScript compiler / Playwright | 6.4.5 / 5.7.3 / 1.64.1 | Experimental build/test tools; installed Edge used |
| Runtime Node / local Python | 24.22.1 / 3.14.5 | Measured development environment; not assumed to be installed for packaged launch |
| Packaged Python | 3.14.10 embeddable x64 | Official security release; checksum verified; packaging result below |

### Primary references

| ID | Authoritative reference | Use |
| --- | --- | --- |
| S01 | [Monaco README](https://github.com/microsoft/monaco-editor), [changelog](https://raw.githubusercontent.com/microsoft/monaco-editor/main/CHANGELOG.md) | Editor scope, browser workers, 0.57 changes |
| S02 | [TypeFox monaco-languageclient](https://github.com/TypeFox/monaco-languageclient), [Codingame API](https://github.com/CodinGame/monaco-vscode-api) | Additional LSP/workbench/extension integration stacks |
| S03 | [CodeMirror reference](https://codemirror.net/docs/ref/), [LSP package README](https://raw.githubusercontent.com/codemirror/lsp-client/main/README.md) | State/view/modules, LSP transport/workspace contract |
| S04 | [Current CodeMirror source host](https://code.haverbeke.berlin/codemirror/lsp-client), [changelog](https://code.haverbeke.berlin/codemirror/lsp-client/src/branch/main/CHANGELOG.md) | GitHub archive is a repository move, not evidence of abandonment |
| S05 | [Theia architecture](https://theia-ide.org/docs/architecture/), [widgets](https://theia-ide.org/docs/widgets/) | Node backend, DI/services and custom React widgets |
| S06 | [Theia application composition](https://theia-ide.org/docs/composing_applications/) | Browser-only 1.76 capabilities and exclusions |
| S07 | [Theia VS Code extensions](https://theia-ide.org/docs/authoring_vscode_extensions/), [extensions user guide](https://theia-ide.org/docs/user_install_vscode_extensions/) | VSIX/Open VSX, host and API compatibility qualifications |
| S08 | [Theia source](https://github.com/eclipse-theia/theia/tree/v1.76.0) | Package/license and backend service source |
| S09 | [OpenVSCode source/releases](https://github.com/gitpod-io/openvscode-server), [license](https://github.com/gitpod-io/openvscode-server/blob/main/LICENSE.txt) | Official archives, Node distribution and Code OSS basis |
| S10 | [Code OSS versus Visual Studio Code](https://github.com/microsoft/vscode/wiki/Differences-between-the-repository-and-Visual-Studio-Code), [Microsoft FAQ](https://code.visualstudio.com/docs/supporting/faq) | Branding, redistribution and Marketplace restrictions |
| S11 | [VS Code web extensions](https://code.visualstudio.com/api/extension-guides/web-extensions), [extension host](https://code.visualstudio.com/api/advanced-topics/extension-host) | Browser versus Node/native execution |
| S12 | [Dockview source/license](https://github.com/dockview/dockview), [license breakdown](https://raw.githubusercontent.com/dockview/dockview/master/LICENCE.md) | Core docking, serialization, theming and separate enterprise license |
| S13 | [LSP specification](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/), [DAP overview](https://microsoft.github.io/debug-adapter-protocol/overview) | Protocols are independent of editor widgets |
| S14 | [TypeScript server releases](https://github.com/typescript-language-server/typescript-language-server/releases), [Pyright](https://github.com/microsoft/pyright), [debugpy](https://github.com/microsoft/debugpy) | Real helper versions and process protocols |
| S15 | [Monaco YAML](https://github.com/remcohaszing/monaco-yaml), [CodeMirror YAML](https://github.com/codemirror/lang-yaml) | Native modules, not VSIX compatibility |
| S16 | [DOMPurify advisory](https://github.com/cure53/DOMPurify/security/advisories/GHSA-mrv2-3pqr-j5jr), [Vite advisory](https://github.com/vitejs/vite/security/advisories/GHSA-8g5g-mxqh-grgj) | Current supply-chain findings; exact audit retained |
| S17 | [Python 3.14.10](https://www.python.org/downloads/release/python-31410/), [Node distribution](https://nodejs.org/dist/v24.22.1/) | Packaged runtime provenance and published checksums |

The web fetcher returned 403/unavailable for some CodeMirror reference/current-host pages. The npm-shipped README, license, implementation and public declarations were read directly instead; no inaccessible document is represented as an executed test. The old GitHub README explicitly identifies the new source host.

## Stage 1 — screening

| Candidate architecture | Small integration spike | Finding | Disposition |
| --- | --- | --- | --- |
| Monaco + native LSP + Dockview + Rust services | Actual Vite production build and Rust-hosted editor | Static assets; no mandatory workbench server. DAP and extension lifecycle remain ASF responsibilities | Finalist A |
| CodeMirror + official LSP + merge + Dockview + Rust services | Actual Vite production build and Rust-hosted editor | Explicit document/client attachment and native module system; custom debugger/workbench required | Finalist B |
| Theia full browser IDE + Node + Rust | Imported genuine 1.76 DiskFileSystemProvider; read an owned file outside the configured ASF fixture roots | Default Node provider reads disk directly; Rust authorization was not invoked | Reject default architecture for this target; no complete mediated-service prototype |
| Theia browser-only + Rust adapters | Documentation and same service/package inspection | 1.76 browser-only uses OPFS; bundled web extensions work, runtime Open VSX install and terminals do not. Restoring mandatory capabilities requires custom service integration | Not a shortcut to complete ASF; not shortlisted |
| OpenVSCode upstream server + Rust proxy | Downloaded official Linux x64 archive, inspected product/package/Node, attempted native Windows launch | Only Linux x64/arm64/armhf release archives; ELF runtime fails with WinError 193 on Windows | Reject current upstream distribution; source port/custom workbench/policy bridge unverified |

Theia spike (historical record held privately) is deliberately a direct service invocation, not a web security exploit or a claim that Theia cannot be customized. Theia's DI/ReactWidget contributions can reshape a workbench without a fork [S05]. Its full LSP/DAP/terminal/plugin stack is a real advantage, documented rather than benchmarked here. ASF would need to replace or mediate filesystem, terminal, process, plugin-host and persistence paths, including direct Node APIs inside extensions. That is a broad authority migration with no demonstrated safe implementation in P01.

OpenVSCode spike (historical record held privately) identifies archive hash `b433bf4f0227321a7014d8460d10a8f958adc0f45aa79bd889e84e65e8f88363`, ELF magic and actual launch failure. This does not prove a source port impossible. It establishes that the inspected ready-to-run upstream release does not meet ASF's native four-target distribution requirement. Its established workbench/extension host would still expose Node authority, and a distinctly orchestrator-centered shell would require unsupported internal workbench changes or a maintained product fork [S09–S11]. A reverse proxy alone does not enforce Rust file/process policy inside that host.

No additional finalist was added to inflate coverage. Codingame/TypeFox is a credible Monaco integration variant, but brings coordinated VS Code service packages and a different API/version matrix [S02]. Its advertised compatibility is not counted as prototyped, nor as permission to install Microsoft Marketplace packages.

## Stage 2 — common executable matrix

`tests/evaluate.mjs` uses Playwright with installed headless Edge, real Git fixture files and the same Rust backend for both editors. It drives keyboard edits and checks disk contents, protocol messages and process exits. Fixtures are synthetic source workloads in real repositories, not fake backend responses. Raw results (historical record held privately) retain failures as well as passes.

| Test | Monaco | CodeMirror | Evidence and qualification |
| --- | --- | --- | --- |
| IDE-01 project/tree/safe paths | PASS | PASS | Real nested files; unauthenticated 401; traversal, `.env` and Windows junction escape 403 |
| IDE-02 edit/save/conflict | PASS | PASS | Keyboard UTF-8 edit, authenticated disk save, external-write conflict, disk and unsaved buffer preserved through reload |
| IDE-03 multiple/split editors | PASS | PASS | Independent documents, switching/reopening retains edits; real Dockview groups. Same URI duplicated in two groups remains untested |
| IDE-04 diff | PASS | PASS | Actual disk/buffer versions, Unicode, changes detected; attempted keyboard edit leaves read-only diff unchanged |
| IDE-05 LSP | PASS | PASS | TypeScript server PID, initialize, diagnostics rendered, hover `greet`, visible termination error, process stopped |
| IDE-06 DAP | PASS | PASS | Real debugpy, verified line-2 breakpoint, stopped event, stack/scopes/value=40, continue and terminated event |
| IDE-07 terminal | PASS with retained upstream error | PASS with retained upstream error | Existing Runtime ConPTY: isatty, output/input, OS-confirmed resize, same-session replay/reconnect, durable finish and PID exit |
| IDE-08 extension/plugin | **FAIL for tested YAML stack** | PASS | Monaco YAML worker reports missing handlers; CM YAML syntax tree changes on activation, uninstall deactivates, unknown package refused |
| IDE-09 custom workspace | PASS | PASS | Central ASF composition surface, explorer, editors/diff, optional side view and bottom xterm in dockable groups |
| IDE-10 restart/restore | PASS | PASS | Persisted panel IDs, active tab, buffer and view state survive browser reload; backend restart qualification is separate |

The additional browser-error check fails for Monaco's YAML worker and passes for CodeMirror. Thus the all-candidate test command intentionally returns nonzero in the retained evaluation; it must not be presented as an all-green product suite. CodeMirror passes all ten common tests and its browser-error check, including a final selected-stack rerun (historical record held privately) after path hardening. Screenshots (historical record held privately) and Monaco comparison (historical record held privately) show technical composition only. The conversation region explicitly has no AI backend; no mission or orchestrator progress is simulated.

### Integration findings and failures

* Monaco 0.57 includes a native LSP SDK; it is incorrect to assume every current Monaco evaluation needs monaco-languageclient. Its global synchronizer sent a foreign project's model during the scope probe. Rust rejected it before child stdin. The CodeMirror view without that project's LSP plugin sent no such request. Neither observation is a complete multi-project product qualification.
* Both stacks required URI identity normalization on Windows: TypeScript returned `file:///c%3A/...`, while opened documents used `file:///C:/...`. Monaco also lowercases synchronized URIs. The scoped bridge now maps only validated project paths and refuses sibling-prefix roots. Initial diagnostic-render failures are retained in the first results.
* Monaco's changed exports required a legacy-path Vite alias for its third-party YAML dependencies. The public native SDK exposes no public disposal/selector contract in the inspected declarations; the synchronizer and feature lifecycle need further work. YAML 5.5.1 still failed actual worker requests despite its broad peer range. No private Monaco internals were patched to label it successful.
* CodeMirror dependencies initially used inconsistent state versions; aligning to one state instance fixed the type mismatch. Its merge UI and language support are separate packages. Full IDE commands, debugger UI and extension registry do not come from the editor alone.
* Windows portable-pty 0.9.0's inspected `WinChildKiller::kill` branch returns an error on a successful `TerminateProcess`. The experiment preserves that diagnostic, observes the finished durable session and independently verifies process exit. The production dependency was not patched. A production fix/upgrade and regression test are prerequisites for P06.

A separate security/protocol run (historical record held privately) passed twelve checks, including real Pyright initialize/diagnostics/hover and rejection of a foreign URI and `workspace/executeCommand` before forwarding. rust-analyzer's wrapper reports the component absent in this toolchain; no Rust LSP success is claimed. Complete Rust/TS/Python journeys remain P06/P14 requirements.

## Complete-stack capability comparison

Legend: **T** directly tested; **D** documented in primary sources/package API; **C** significant custom implementation; **X** unavailable in tested configuration; **NT** not tested. D is not a passed test.

| Capability | Monaco stack | CodeMirror stack | Theia full stack | OpenVSCode upstream |
| --- | --- | --- | --- | --- |
| Editing, multiple models/documents | T; rich standalone editor | T; state/view/modules | D, NT workbench | D, NT workbench |
| Find/replace, undo, multi-selection | D; not a full P01 UX qualification | D; history/keymaps included, not full UX qualification | D, NT | D, NT |
| Side-by-side diff | T standalone diff | T merge package | D, NT | D, NT |
| Real project disk authorization | T Rust | T Rust | T direct Node service; Rust mediation C | Node service D; Rust mediation C |
| LSP diagnostics and hover | T native SDK; scope/lifecycle C | T official client; per-workspace transport | D; adapter/extension dependent, NT | D; adapter/extension dependent, NT |
| Completion/definition/rename/imports | D in clients/server; NT complete flow | D in clients/server; NT complete flow | D, NT | D, NT |
| DAP protocol | T shared ASF adapter; full UI C | T shared ASF adapter; full UI C | D integrated; NT | D integrated; NT |
| PTY | T existing ASF; development-shell policy C | T existing ASF; development-shell policy C | D Node service, NT | D Node service, NT |
| Native editor plugin | Tested YAML X | T YAML module | D Theia extensions, NT | D workbench contributions, NT |
| VS Code extension host | X in standalone stack; third-party S02, NT | X; native ASF API C | D partial API compatibility, NT | D Node/web hosts; individual compatibility NT |
| Orchestrator-first composition | T React/Dockview | T React/Dockview | D custom widgets/DI; full reshape NT | Product fork/internal customization C, NT |
| Persistent layout/buffers | T experimental Rust storage | T experimental Rust storage | D, NT | D, NT |
| Light/dark/a11y | D APIs; dark technical prototype T; AA NT | D APIs; dark technical prototype T; AA NT | D; ASF theme/AA NT | D; ASF theme/AA NT |
| Production-ready Rust authority | C beyond demonstrated bounded bridge | C beyond demonstrated bounded bridge | Broad service/extension authority rework C | Broad service/extension authority rework C |

Monaco's accessibility features and familiar editing surface are advantages [S01]. CodeMirror supplies accessible editor DOM and keyboard APIs [S03], but neither P01 prototype was qualified with a screen reader, IME, physical keyboard or WCAG contrast suite. Theia supports custom contributions rather than mere CSS rebranding [S05]; rejection is based on the demonstrated authority/packaging burden, not presumed lack of theming. OpenVSCode preserves a complete familiar workbench, but offers much less public composition control for ASF's distinct central interaction [S09–S11].

## Extension-runtime assessment

| Concern | Monaco / CodeMirror native composition | Theia | OpenVSCode / Code OSS |
| --- | --- | --- | --- |
| Package/API contract | Native JS modules T; ASF manifest, stable contribution API and import/update manager C | Theia application packages plus VSIX plugin API D | VSIX and VS Code API D |
| Activation and editor hooks | CM YAML T; Monaco YAML failed; commands/views can be ASF contributions C | D plugins and DI contributions, NT | D activation/events/contributions, NT |
| Browser-only code | Native UI module T; no privilege isolation | 1.76 bundled web extensions D; restricted browser-only service set | Web extension host D; no Node APIs in browser host |
| Node/native helpers | Only Rust-approved tool profiles T; general extension host C | Node plugin host and native tools D | Local/remote Node host and native dependencies D |
| LSP/DAP/terminal contributions | Protocols/PTY demonstrated separately; packaged contribution wiring C | D existing APIs; direct process authority mediation needed | D existing APIs; direct process authority mediation needed |
| Filesystem/network/trust | Rust APIs scoped T; trusted in-process plugin can use UI privileges. Worker alone is not a security sandbox | Node/native code has OS user privileges; workspace trust is not OS sandboxing | Same; extension installability does not prove safety |
| Versions/dependencies/conflicts | Locked bundled package T; arbitrary package resolution C | Version-specific API and compatibility reports D | Engine/API/native target compatibility D |
| Update/uninstall/failure isolation | CM install/activate/uninstall T; runtime cache unload, rollback, crash isolation C | D; NT in P01 | D; NT in P01 |
| Marketplace | Authorized native ASF sources/import format C; no Microsoft Marketplace | Open VSX D; package-specific rights | Open VSX/configurable gallery D; Microsoft Marketplace rights cannot be assumed |

Browser, Node-host and native-process extensions are distinct trust classes [S11]. Theia's documented API includes stubs/incomplete areas and per-version compatibility [S07]. Code OSS's MIT license does not grant use of Microsoft's product branding or its Marketplace, and individual extensions may carry narrower product licenses [S10]. No untrusted extension was installed or executed. YAML packages were known, pinned and explicitly licensed. A native-module proof is sufficient for P01 extensibility feasibility; it does not count as the complete P06 extension manager.

## Performance measurements

Host: Windows 11 Home 10.0.26200 x64, i5-12500H, 16,802,754,560 physical RAM bytes; shared machine, finite memory/commit headroom. Installed Edge runs headless via Playwright. Three new browser processes per candidate/project, identical viewport and backend fixtures. No Rust/frontend regression build ran during the final browser/service measurements. Other user/system applications remained present.

Small/medium/large fixtures contain 21/511/10,011 non-Git/non-Factory files (approximately 0.20/0.49/6.55 MB); ten/five hundred/ten thousand generated tree modules plus real source/config files. The tree API hides `.env`; visible file counts are 20/510/10,010. A common 5,000-line TypeScript document exercises larger buffers. These are generated workloads, not a claim about all large repositories.

Raw interface samples (historical record held privately), service samples (historical record held privately), derived medians (historical record held privately) and bundle inventory (historical record held privately) are retained. Times below are medians, milliseconds, n=3. No meaningful population p95 is inferred from three observations.

| Stack / project | Project ready, new browser navigation | Warm reload + restore | Open source | Switch tab | Split | Open 5,000 lines | Keyboard roundtrip + 2 frames |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Monaco small | 744.44 | 348.32 | 152.52 | 22.25 | 51.33 | 76.42 | 44.42 |
| Monaco medium | 885.59 | 427.63 | 148.00 | 26.97 | 57.12 | 85.06 | 54.16 |
| Monaco large | 2,500.09 | 2,049.60 | 198.36 | 34.17 | 60.89 | 100.95 | 85.20 |
| CodeMirror small | 493.82 | 223.98 | 147.92 | 19.57 | 50.66 | 76.64 | 34.86 |
| CodeMirror medium | 499.06 | 306.29 | 142.21 | 17.60 | 50.03 | 90.18 | 35.98 |
| CodeMirror large | 2,014.69 | 1,735.05 | 78.15 | 15.92 | 56.28 | 90.75 | 36.74 |

“Cold” here means a fresh browser process/context, **not flushed OS caches**. Browser launch adds approximately 337–378 ms by group median; the raw data includes launch and combined times. Warm reload retains browser/HTTP caches and a persisted layout. Input timing includes automation and two frames; it is not INP, native keystroke latency or a physical-input test. Open-file timing includes the experiment's polling/paint barrier. The non-virtualized common explorer renders every path, so large-tree rendering is a shared bottleneck and a P05 virtualization prerequisite. One CM small sample was 1,096 ms and one Monaco large sample 4,357 ms; outliers are retained.

| Metric | Monaco | CodeMirror |
| --- | ---: | ---: |
| All built assets, raw bytes | 16,505,354 | 1,601,144 |
| All built assets, derived gzip bytes | 4,076,726 | 442,788 |
| Asset count | 98 | 4 |
| Initial small-workflow resource encoded bytes | 5,833,400 | 1,795,256 |
| Small-workflow JS heap used bytes | 26,954,177 | 11,936,948 |
| Small-workflow owned browser + Rust working-set bytes | 691,294,208 | 623,267,840 |
| Small-workflow owned browser + Rust private bytes | 401,227,776 | 334,405,632 |
| LSP initialize ready | 327.70 ms | 297.40 ms |
| First rendered real diagnostic | 1,923.38 ms | 1,817.36 ms |
| LSP helper process-tree working set | 498,290,688 bytes | 498,442,240 bytes |
| DAP initialize ready | 616.80 ms | 569.60 ms |
| DAP stopped and variables retrieved | 2,329.40 ms | 2,314.60 ms |
| Successful native YAML activation | Unavailable | 23.70 ms |

Bundle profiles are **not feature-equivalent**: Monaco's full entry includes many built-in languages/features; CM loads the explicitly selected languages/modules. Both execute the same tested workflows. A selectively imported Monaco build could shrink; that optimization was not measured. “Encoded resource bytes” includes the actual observed assets/API resource entries, not a network-compressed shipping guarantee. Gzip totals are derived offline; Rust's experimental static serving does not claim those transfer sizes. Working sets include shared pages and should not be treated as unique physical allocation. JS heap excludes separate worker heaps; process measurements are snapshots, not peak/endurance results.

The P00 backend measurements (single-project debug HTTP/history) are not directly comparable with browser paint/editor/service metrics. No V2 production performance thresholds or “faster than V1” claims are inferred. Helper memory is material for both candidates and requires P06 budgets/lazy activation. Theia/OpenVSCode resource figures are **not measured**; dependency breadth alone is not a RAM benchmark.

## Security assessment

The common experimental Rust boundary reuses production authentication, rejects unsafe Host/Origin/CSRF, binds only to loopback, authenticates project-bound WebSockets and denies arbitrary process profiles. Twelve additional checks (historical record held privately) verify mutation CSRF, cross-origin requests, case-variant private/Git paths, raw rebinding Host (400), unknown profiles (400), oversized HTTP bodies (413), cross-project leases (403), duplicate ownership (409), unauthenticated WS (401), WS origin (403), and real Python LSP with pre-forward refusals. Traversal/secrets/junction tests and frame limits supplement them. They are a bounded probe, not a penetration-test certification.

| Threat surface | Monaco finalist | CodeMirror finalist | Required production mitigation |
| --- | --- | --- | --- |
| Extra local servers / WS | One Rust listener; native LSP WS T | Same | Preserve same-origin auth; no unauthenticated helper port |
| Cross-project documents | Native global model attempt observed and blocked T | Explicit view attachment did not send foreign model T | Canonical document identity, generations, scoped clients and adversarial two-project tests |
| Files / symlinks / conflicts | Shared Rust service T | Same | Pin handles across TOCTOU races; reparse/hardlink/worktree tests; no generic frontend FS authority |
| Protocol server/adapters | Trusted user-privileged helpers T | Same | Profiles, env/secret handling, bounded messages, cancellation, leases, process-tree cleanup and resource quotas |
| Terminal | Existing Runtime fixed trusted probe T | Same | Separate development-shell permission model; fix termination diagnostic path |
| Plugin code / HTML | YAML worker incompatibility; patched sanitizer | Native module in main UI; escaped LSP hover markup | Trusted code only until isolated origin/capability broker; no claim that Worker/worktree is OS sandbox |
| Supply chain | More language/worker compatibility edges | Modular version alignment and upstream source-host migration | Locked integrity, provenance/license inventory, update policy, audit at integration |
| Persistence / secrets | Experimental buffers stored locally | Same | Project/session access control, secret redaction, retention, encryption policy where required |
| Cleanup | Protocol Job Objects and observed exits T | Same | Native crash/descendant/lease tests on four platforms; P10 recovery remains unimplemented |

Theia full-stack threats include a second server/auth surface, Node filesystem/process authority and extension-host direct APIs [S05/S07]. The service spike demonstrates the default authority gap. Mitigation would require private IPC, Rust-brokered service replacements, supervised Node lifecycle, filtered project capabilities and a trusted-extension policy. These mitigations were not built, so the full Theia stack is not verified against ASF's boundary. Browser-only Theia removes the Node server but does not supply the mandatory local tools [S06].

OpenVSCode's Node server/extension host similarly cannot be made Rust-authoritative by adding an HTTP proxy. Its terminal/files/processes and extension native modules need brokered authority or an explicitly different, approved trust model. Porting the Linux release runtime also adds supply-chain/native-target maintenance [S09–S11]. No insecure alternative service was launched to evade these constraints.

The initial experimental npm audit reported one critical, one high and one low package entry. Monaco pinned DOMPurify 3.4.15; Vite 6.4.3 also inherited current advisories. The experiment overrides DOMPurify to 3.4.17 and pins Vite 6.4.5; the final audit (historical record held privately) reports zero known vulnerabilities at retrieval time. This is not a guarantee of absence. Applicable advisory conditions vary [S16]. Production dependencies were not silently changed; the existing production Vite version requires separate maintenance review. No secrets or downloaded binaries are checked into Git.

## Licensing and distribution review

Monaco, CodeMirror and Dockview core permit an ASF-owned shell under their MIT licenses, retaining notices. Dockview enterprise is proprietary and excluded [S01/S03/S12]. Native YAML packages are MIT. TypeScript tooling uses Apache-2.0 and includes third-party notices. Pyright is MIT. Node has its own MIT/third-party notice bundle; Python has PSF and bundled-component licenses [S14/S17].

debugpy's top-level MIT label is insufficient for distribution review: its shipped `ThirdPartyNotices.txt` includes PyDev.Debugger under EPL-1.0 and other component terms. The representative package preserves the vendored source/notices and dist-info license; no claim of an entirely MIT runtime bundle is made. Theia's dual-license expression is not inherently a disqualifier, but brings different notice/source obligations [S08]. Code OSS/OpenVSCode MIT source rights must remain distinct from Microsoft's branded binaries, Marketplace terms and individually restricted extensions [S09/S10].

The experimental npm inventory (historical record held privately) records all 173 lockfile packages, their integrity/license metadata and installed notice paths; none has missing license metadata. This is an engineering license/provenance review using the actual artifacts and upstream texts, not a grant of rights for arbitrary extensions. P14 requires a full shipping SBOM, notices/source offers where applicable, native runtime dependency review and license checks for every supported adapter. No proprietary marketplace asset was bundled.

## Packaging feasibility

Both finalists compile to static assets suitable for existing Rust-Embed hosting. Neither requires a Node workbench backend or an external account. JavaScript language servers still need Node; Python debugging needs an interpreter/debugpy. These must be bundled/provisioned automatically as approved tool packs, or explicitly reported unavailable until installed. A development-machine PATH is not a deployment strategy.

The representative Windows recipe builds the isolated Rust executable with `--release --features embedded-assets`, embeds the actual Vite builds, obtains official checksum-verified Node 24.22.1 and Python 3.14.10 runtimes, and copies pinned TSLS/TypeScript/Pyright/debugpy plus license notices. `package_windows.py` emits a local zip, per-file manifest and archive SHA-256 outside Git. `tests/package.mjs` expands it to a different directory, launches with only Windows system directories on PATH, blocks external browser requests, uses project state outside the installation, and checks real editor save/LSP/DAP/plugin/PTY/restore/shutdown. The packaging build (historical record held privately) and execution result (historical record held privately) are authoritative for its result and exact sizes.

The experiment package contains both comparison frontends; this is not the proposed finished ASF installer, update implementation or a published release. Linux x64, macOS x64 and macOS ARM64 native execution remain **PENDING**. The exact proposed pipeline and target runtime strategy are in the experiment README. No existing release workflow or install script was changed. Extensions/configuration/project data belong outside installation; uninstalling the package must not remove them. Offline use is demonstrated for the pinned package, not arbitrary extension downloads.

## Weighted decision

Scale: 0 absent/unusable; 1 severe unverified constraint; 2 substantial integration limitation; 3 feasible with substantial owned implementation; 4 demonstrated good fit; 5 demonstrated excellent fit or unambiguous distribution fit. Scores are **engineering judgments**, not measurements. Half points are allowed. Untested performance is conservatively 1; documented but unexecuted full-IDE features do not receive the same confidence as direct tests. Weighted result is `sum(score × weight)`, out of 5.

| Criterion | Weight | Monaco stack | CodeMirror stack | Theia full stack | OpenVSCode upstream |
| --- | ---: | ---: | ---: | ---: | ---: |
| Editing quality / developer UX | 15% | 4.5 | 4.0 | 4.5 | 4.5 |
| LSP | 15% | 3.0 | 4.0 | 4.0 | 4.0 |
| DAP | 15% | 3.0 | 3.0 | 4.0 | 4.0 |
| Extension architecture / compatibility | 15% | 2.0 | 3.0 | 4.0 | 4.0 |
| Rust integration | 15% | 4.5 | 4.5 | 1.5 | 1.0 |
| Performance / resources | 10% | 3.0 | 4.0 | 1.0 NT | 1.0 NT |
| Security / maintenance | 10% | 3.0 | 4.0 | 2.5 | 2.0 |
| Licensing / distribution | 5% | 5.0 | 5.0 | 4.0 | 4.0 |
| **Weighted /5** | **100%** | **3.400** | **3.825** | **3.250** | **3.125** |

| Candidate | Evidence supporting scores | Effort / limitations / risk | Confidence |
| --- | --- | --- | --- |
| Monaco | Rich editing/diff T; LSP T but global models; DAP/PTY T shared Rust; YAML failed; measured larger shipped profile; MIT | High: scoped LSP/provider lifecycle, worker/plugin compatibility, owned debugger and extension manager; S02 could improve capabilities but unqualified | Medium-high for tested stack; low for S02 alternative |
| CodeMirror | Editing/merge T; project-attached LSP T; DAP/PTY T shared Rust; native YAML T; measured static profile; MIT | High: full debugger UI, stable native extension host/import/update, duplicate-view document ownership, language-specific qualification, accessibility and large-tree virtualization | High for local feasibility; medium for full V2 implementation effort |
| Theia | Editing/LSP/DAP/plugin architecture D, not full workbench T; actual direct Node file provider T; browser-only exclusions D; license reviewed | Very high under Rust-authority requirement: broad service/host mediation, supervised packaged Node and native modules, full customization prototype missing | Medium for documented architecture; low for ASF-compliant integration/performance |
| OpenVSCode | Full editor/protocol/extension design D; upstream archive/platform failure T; source license reviewed; no frontend/runtime benchmark | Very high: native four-target packaging, product/workbench customization and Rust authority mediation; marketplace and extension-target validation | Medium for archive finding; low for a custom port/integration |

Theia/OpenVSCode's licensing score addresses their open-source distribution basis; their severe runtime/authority findings independently disqualify the screened architecture. No numerical score overrides those blockers. CodeMirror does not win because it has a marketplace or attractive appearance. Its winning evidence is the tested composition, explicit service ownership and native extension path. Monaco remains a credible fallback if an upstream-supported scoped lifecycle and compatible module stack are demonstrated; a speculative compatibility library is not a tie-breaker result.

## Remaining implementation obligations

The selected foundation makes the full V2 target plausible without a second IDE control plane; it does not supply all of it. P03–P06 must implement production document ownership, safe file handles, multi-project/session persistence, all supported editor commands, complete LSP/DAP lifecycle and UI, approved development shells, extension package management/trust and real Rust/TS/Python qualification. P02 must create both themes and accessible components; its implementation has not begun. P10/P14 must validate crash recovery, descendants, native platforms, installers and the complete product.

Before production integration: obtain green P00/P01 CI for the exact published commits through a separately authorized workflow, review these contracts, resolve the PTY termination path and plan the native extension API explicitly. Do not copy the probe's fixed profiles, fixture-only policy or test seams into production as a finished IDE. **ASF V2.0 remains NO-GO.**
