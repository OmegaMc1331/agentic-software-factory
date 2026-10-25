# Security Policy

How to report security vulnerabilities in Agentic Software Factory ("Factory")
and how reports are handled. This page covers disclosure. For the trust model of
the running application — what Factory isolates and what it deliberately does
not — see [docs/security.md](docs/security.md).

## Supported Versions

Factory is developed as a single line on the `main` branch. Releases are cut
from `main` by version tag; there are no backport or long-term-support
branches.

| Version | Supported |
| --- | --- |
| `main` (default branch) | Yes — fixes land here first |
| Latest release (currently [v1.0.5](https://github.com/OmegaMc1331/agentic-software-factory/releases)) | Yes |
| Earlier releases | Not patched — upgrade to the latest release |

Security fixes are committed to `main` and reach users with the next tagged
release; older releases do not receive separate patches. Both installers fetch
the latest release by default and replace an older binary, so upgrading means
running the installer again.

## Reporting a Vulnerability

**Do not post vulnerability details, exploit steps, or sensitive logs in public
issues, pull requests, or discussions.**

Preferred channel: GitHub's private vulnerability reporting — the **Report a
vulnerability** form at
<https://github.com/OmegaMc1331/agentic-software-factory/security/advisories>.
Reports submitted there are visible only to the maintainers.

> **Maintainer action required:** private vulnerability reporting is not yet
> enabled on this repository (verified 2026-10-25). The repository owner must
> enable it under *Settings → Code security and analysis → Private
> vulnerability reporting*; the form above does not work until then. Remove
> this note once it is enabled.

If the form is unavailable, open a public issue that contains **no
vulnerability details** — only a request for a private security contact — and
the maintainers will respond with a working private channel. Do not include
summaries, stack traces, or reproduction hints in that issue.

## What to Include

A useful report contains:

- a concise summary of the vulnerability;
- the Factory version (`factory --version`) or commit, and your OS;
- the affected component (API, Agent Console, agent execution, policies, MCP,
  persistence, Git integration, installers);
- prerequisites and step-by-step reproduction;
- expected and observed behavior;
- the security impact you believe it has;
- relevant evidence, redacted as described below;
- a suggested mitigation, if you have one.

Please redact from any evidence: API keys, session cookies, CSRF tokens, MCP
capability grants, private repository content, personal information, and
environment secrets. Never upload real credentials. Reproduce only on systems
you control; do not test against third-party services.

## Responsible Disclosure

- Report privately before anything is public, and allow the maintainers
  reasonable time to investigate and fix.
- Coordinate the timing of any public disclosure with the maintainers.
- Keep reproduction minimal and controlled.
- Do not access, modify, or exfiltrate data from systems you do not own or
  operate, and do not run denial-of-service tests against third parties.

No fixed disclosure deadline is imposed — none has been adopted by the
maintainers — and there is no bug bounty program. This policy is not
authorization to test third-party systems.

## What Happens After a Report

1. A maintainer acknowledges and triages the report.
2. The report is reproduced and its impact assessed.
3. Affected versions are identified.
4. A fix is developed on `main` with regression tests.
5. A release is tagged when appropriate, and publication is coordinated with
   you.

Handling is best effort: no response-time targets are promised, severity and
remediation depend on the verified impact, and not every report will lead to a
fix. A published advisory or CVE assignment may follow when warranted, but is
not guaranteed.

## Security Scope

Reports are welcome on anything inside Factory's trust boundaries:

- **Local HTTP API** — loopback binding, Host/Origin checks, session cookies,
  CSRF protection, route authorization.
- **Agent Console** — interactive terminal WebSocket, PTY launch limited to
  configured agents, session isolation, terminal output persistence.
- **Agent execution** — executable resolution from configuration, argument
  construction, environment filtering and redaction, worktree working
  directories.
- **Policy Engine** — role and agent policy evaluation, filesystem scopes,
  command allow/deny, Git limits, always-on safety invariants, policy
  bypasses.
- **MCP integration** — server configuration, scoped capability grants, tool
  schema validation, untrusted-output marking, credential redaction.
- **Workflow persistence** — SQLite state integrity, atomic configuration
  writes, captured session output, concurrent access.
- **Git integration** — worktree isolation, integration-branch control, the
  delivery-only push path, `git`/`gh` command construction.
- **Distribution** — release archives, SHA-256 checksums and manifest,
  `install.sh` / `install.ps1`.

Out of scope: vulnerabilities in third-party coding agents (Codex, Claude
Code, OpenCode, Gemini CLI, Qwen Code), in MCP servers, or in model providers —
report those to the upstream project. Factory orchestrates these tools; it
cannot patch them. Findings that require code already running as your user on
your machine fall inside the documented local-operator trust boundary
([docs/security.md](docs/security.md)) rather than outside it, though reports
are still welcome.

## Security Limitations

These are documented boundaries, not vulnerabilities:

- Factory does not sandbox agent processes. External coding agents run with
  the OS permissions of the user who started Factory; task worktrees provide
  Git isolation, not OS isolation.
- Network `deny` policy is advisory on every current platform; Factory cannot
  restrict a launched process's network access.
- The API's session, CSRF, and Host/Origin checks are a browser boundary, not
  user authentication; a local process running as the same user is treated as
  a trusted operator.
- The Policy Engine controls Factory's boundaries and recorded evidence; it
  cannot intercept everything an external CLI and its descendant processes do.
- MCP servers are separate external processes or services; Factory's grants
  and limits are orchestration controls around them, not a sandbox for them.
- Factory never manages model-provider credentials, and GitHub authentication
  belongs to the locally installed `gh` CLI; Factory reads, stores, and logs
  no tokens.

## Security Issue or Ordinary Bug?

Likely security issues: authentication bypass, unauthorized API access, path
traversal, privilege escalation, secret exposure, capability-grant bypass,
unsafe command execution, unauthorized modification of project files,
security-relevant state corruption, release integrity failures.

Ordinary bugs — layout and visual problems, non-security UI defects,
formatting, feature requests, performance issues without a security impact —
belong in
[GitHub Issues](https://github.com/OmegaMc1331/agentic-software-factory/issues).
A performance issue that enables denial of service may still be security
relevant. If you are unsure, report privately with a minimal summary.
