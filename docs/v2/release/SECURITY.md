# Security and privacy of the release candidate

ASF runs a local Rust HTTP server. Loopback binding is combined with authenticated
sessions, Host and Origin validation, CSRF for mutations, and authenticated scoped
SSE/WebSocket access. Localhost alone is not the security boundary. Do not expose
the listener through a reverse proxy or change browser security to bypass refusals.

Session bootstrap protects the browser boundary; it does not authenticate an OS
user. Native clients can supply their own headers. As documented in the
authoritative local security model, anyone able to use this machine's loopback
interface is treated as a trusted local operator. This service is not a security
boundary against untrusted local users or programs.

Rust owns project identity, files, process supervision, Git/index coordination,
review/integration, memory eligibility and publication authorization. An ID, deep
link, command-palette action, model instruction or project memory grants no extra
authority. File revisions and stale index/hunk checks must remain enabled.

ASF may invoke separately authenticated coding-agent CLIs and send selected project
context to their external model providers. Local execution does not mean offline
processing. Provider terms, native CLI sandbox and host permissions still apply.
ASF application policy is not a universal OS sandbox. It does not serialize model
inference state or store hidden chain-of-thought as project memory.

Approved local tools, terminals, Git hooks/signing/filter helpers and language/debug
servers may execute code as your user. Trust only intended repositories/configuration.
Declarative ASF extensions do not implement arbitrary VS Code extension code.
No security or provider refusal is authorization to broaden permissions.

Windows Job Objects and Unix owned process groups support cleanup, with documented
spawn/assignment and hostile-local-process limitations. Forced server death differs
from normal controlled shutdown. Browser presence is a lease: a frozen background
tab can be indistinguishable from a disconnected client after lease and grace expiry.
Manual resume and checkpoint validation remain required.

Project `.factory` data can contain conversations, code evidence, task output,
profiles, checkpoints, memory revisions and remote delivery identities. Application
sidecars contain registry, workspace, IDE, settings, notifications and presence data.
Backups retain older content. Known-secret redaction is useful but not perfect;
do not enter credentials into conversations, memory or logs. Memory deletion removes
its authoritative revisions/index representation and future eligibility, while
independent historical mission evidence and backups have separate retention.

GitHub delivery requires effective project policy, verified target repository,
current integration evidence and explicit authorization. A completed local mission
does not authorize push, PR creation or merge. This qualification does not publish
the ASF repository or create a remote test PR.

The audit and artifact manifest identify the exact source, dependency lockfiles,
known findings and unsigned status. No code signing, notarization, remote CI or
artifact attestation is claimed until actual matching evidence exists. Report
suspected vulnerabilities privately using the repository SECURITY.md process.
