# P14 local qualification

Use Python 3.14.5 on Windows, Rust 1.99.1 from `rust-toolchain.toml`, Node 24.22.1
and npm 12.0.2. Other native Python versions must be recorded; scripts require
Python 3.11+ (`tomllib`). Tools and dependencies remain separate from end-user
application requirements. `npm ci` uses the existing dashboard lockfile.

```powershell
python -B scripts/v2_p14/run.py contracts --output .factory/v2/p14/contracts
python -B scripts/v2_p14/run.py rust --output .factory/v2/p14/windows
python -B scripts/v2_p14/run.py normal --output .factory/v2/p14/windows
python -B scripts/v2_p14/run.py frontend --output .factory/v2/p14/frontend
python -B scripts/v2_p14/run.py frontend-tests --output .factory/v2/p14/frontend
python -B scripts/v2_p14/run.py build --output .factory/v2/p14/native
python -B scripts/v2_p14/run.py embedded-tests --output .factory/v2/p14/native
python -B scripts/v2_p14/package.py build --target x86_64-pc-windows-msvc --output .factory/v2/p14/packages
python -B scripts/v2_p14/smoke.py --package-dir .factory/v2/p14/packages --output .factory/v2/p14/native/packaged-smoke.json
python -B scripts/v2_p14/dependencies.py --output .factory/v2/p14/dependencies
```

Freeze/commit source before the release build. Run commands serially under low RAM.
`run.py` records actual source fingerprints before/after and refuses changed inputs.

### Hosted qualification output

The qualification workflow invokes the fixed recipes in `ci.py`. Raw stdout,
stderr and diagnostics stay under the job's temporary directory. The public
log contains a structured step result, exact source SHA, exit, timing and any
validated test counts. Rust commands emit a progress record at most every 30
seconds during capture. A Cargo build completion event distinguishes the
build (including linking) from test discovery and execution. Cargo artifact
counts distinguish newly generated outputs from reused ones. Test progress
counts completed suites, with ignored cases separate. When libtest supplies
durations, the result includes the sum of the completed suites' durations.

The wrapper resolves failing test names against checked-in declarations and
binds them to a Cargo test target. Unexpected names, doctest paths and unknown
binaries get a generic identifier. A timeout or interruption reports the last
observed phase and completed test count when available. Assert messages,
command configuration and paths stay private.

Selected Rust failures also carry a checkpoint from a fixed allowlist bound
to the suite and test. The Python contract steps use buffered verbose unittest output
inside private capture. Public summaries include counts and failing case
identifiers validated against tracked test declarations. Missing completion
evidence blocks a passing result, and an unrecognized case uses a generic
identifier. Contract progress uses the same 30-second heartbeat.

Chromium, Firefox and WebKit use only the screened Linux package from the
current run and run their full assertions with screenshots disabled. The
inventory still depends on all four native targets. The Intel macOS job uses a
240-minute ceiling. Its embedded command has a 150-minute inner limit, with one
minute for outer capture and cleanup. The embedded test recipe and release
profile remain as configured in the repository.

The only artifact publishers are the native package and consolidated inventory
steps. Each uploader enumerates exact files and requires a successful final
privacy approval. Native screening checks package and build identity, actual
archive members, binary and frontend strings, and the native smoke receipt.
Approval checks the staged files against a private hash seal. Browser and
failed-step diagnostics stay runner-local. A missing or changed file blocks
approval.
Build records include binary and embedded-asset hashes; package creation requires
that exact successful build record and native host. Select fresh output directories
to preserve failed attempts. `CARGO_TARGET_DIR` may point at an explicitly owned
external build cache; a separate empty target directory is required for clean-build
qualification. No source dependency is fetched through an unlocked build.

The package builder uses the existing ZIP/tar.gz names, stable commit-based archive
timestamps, native architecture validation and a per-file manifest. It includes
actual license inventory, SBOM, operating instructions and a scoped V2 launcher.
The smoke test extracts into a new bounded Unicode/spaces fixture, authenticates,
verifies every embedded asset, exercises real file CAS/Git/SQLite/preferences,
signals normal shutdown and removes only the extracted application files.

Use the retained production P13 browser/mission/performance drivers against the
P14 binary, with fresh P14 outputs. Their historical environment variable names
are retained deliberately: `ASF_P13_BINARY`, `ASF_P13_OUTPUT`,
`ASF_P13_PERFORMANCE_OUTPUT`, `ASF_P13_BUILD_RECORD`, `ASF_P13_BUILD_PROFILE`.
Set `ASF_QUALIFICATION_PHASE=p14` for P14 performance output. This reuses the same
ordinary API workloads; it does not relabel an old binary. `ASF_BROWSER_ENGINE`
selects chromium/firefox/webkit in the product browser driver. Default remains Edge.
Real-model acceptance uses `scripts/v2_p13/mission.mjs --model` only after deterministic
health, with the intended installed CLI and its unchanged policy boundaries.

Large-memory qualification must use `ASF_P13_MEMORY_COUNTS=100,1000,10000` without
`--stress` to preserve the longer ordinary payload. Also compare the original
P11 driver's 100/1,000/10,000 fixture; different payloads/build profiles are not
an implementation speedup denominator. Preserve independent Git/test verification.

`bootstrap-linux.sh` installs verified pinned tooling into its owned WSL/Linux tool
directory. It does not copy Windows authentication or treat `.exe` interop as native
Linux provider coverage. Native compiler/system-library prerequisites must be checked.

The CI and release-candidate workflows have no publication job. Candidate push or
manual qualification may run CI and retain 14-day Actions artifacts only after
explicit authorization. No remote command is run by these local instructions.
