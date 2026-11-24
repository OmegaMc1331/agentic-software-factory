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
