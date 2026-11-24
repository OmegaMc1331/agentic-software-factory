# Candidate limitations and open qualification

ASF V2.0 remains **NO-GO** until the mandatory ledger gates pass and release is
explicitly authorized. Initial P14 audit findings and their final disposition are
recorded in the qualification report; this list does not turn unexecuted gates into
passes.

- Exact-source remote CI remains blocked pending publication authorization.
  Packages and genuine coding acceptance must be verified for each candidate.
- Native macOS ARM64/x86_64 machines and Apple signing/notarization identities are
  unavailable. Windows execution does not qualify them. WSL2 Linux results describe
  that actual Linux environment, not every distribution or native desktop.
- Live GitHub write acceptance has no authorized disposable target. Controlled
  production-provider tests are distinct from a real push/PR. No remote mutation
  is performed merely to finish qualification.
- Native screen-reader and installed IME journeys require actual assistive/input
  technology. Axe and synthetic composition tests cannot certify them.
- Provider adapters have different fixture/live qualification. Genuine Codex
  execution does not qualify Claude Code, OpenCode, Gemini, Qwen or custom adapters.
- P10 retains manual resume and default 180-second lease plus 60-second grace.
  Browser freezes/disconnection cannot always be distinguished by heartbeats.
- P11's historical 9,000-memory/53.44-minute load and P12's debug/concurrent-host
  timings are not release benchmarks. Final release-build large fixtures remain
  separate mandatory measurements.
- The frontend's large main chunk and high-worker test memory need bounded,
  measured qualification. Two-worker test success is not unrestricted-worker proof.
- Git/local helper trust, host sandbox limits and hostile local filesystem races
  remain documented. No universal OS containment or secret-detection claim is made.
- Portable packaging is the existing supported model. Start menu integration,
  MSI, signing and automatic updates are not invented by the candidate.
- The initial npm audit found seven moderate development-only ESLint/minimatch
  findings; these are distinct from shipped browser runtime dependencies and must
  remain in the supply-chain disposition. Rust dependency findings trigger narrow
  patched-family updates and fresh regression/migration tests.
- The frontend toolchain locks picomatch 4.0.5. The CNA still lists CVE-2026-112366
  as published. GitHub withdrew its advisory. The release security gate remains
  failed pending a verified resolution or an authorized policy disposition.
