# Installing the local release candidate

This is an unsigned qualification candidate, not an approved V2.0 release.
Consult the qualification ledger and artifact manifest before using a build.
Windows x86_64, Linux x86_64 and both macOS architectures have separate gates;
an archive for an unavailable target is not generated.

## Prerequisites

Launching an embedded ASF package requires neither Node.js nor Rust. A modern
browser is required. Git is required for Git projects and missions. Coding
agents require their separately installed, authenticated CLI and its supported
runtime. LSPs, debug adapters and development terminal profiles are optional,
explicitly approved tools; they are not universally bundled.

Windows packages use the Microsoft v14 x64 runtime and the Windows Universal CRT.
They do not bundle those system runtimes. Install the matching current
[Microsoft Visual C++ v14 x64 Redistributable](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist)
when it is absent. Microsoft requires the runtime version to be at least as
recent as the compiler used to build the application. The qualification build
uses MSVC 14.51; consult the actual artifact's dependency inventory. Launching on
an existing development machine does not establish that a minimal Windows image
already contains these prerequisites.

## Portable installation

Verify the archive's SHA-256 against its `.sha256` file and the artifact manifest.
Extract into a directory owned by your ordinary user. Preserve all accompanying
license and security documents. Do not run ASF as administrator/root.

Windows:

```powershell
Get-FileHash .\factory-windows-x86_64.zip -Algorithm SHA256
Expand-Archive .\factory-windows-x86_64.zip -DestinationPath '.\ASF candidate'
cd '.\ASF candidate'
.\start-workspace.ps1 -ProjectDirectory 'C:/work/project'
```

The selected directory explicitly authorizes the project locations ASF may open.
Choose a narrow development parent directory. The launcher opens the V2
workspace and records startup output under
`%LOCALAPPDATA%\AgenticSoftwareFactory\logs`. Press Ctrl+C to shut down normally.
Keep the console open while ASF is running. No background service, installer
elevation or automatic Start menu registration is added.

If the local PowerShell execution policy refuses the unsigned launcher, use the
executable directly from the extracted directory:
` .\factory.exe start --workspace --no-default-project --allow-project-root 'C:/work/project' `.
Do not weaken the system execution policy to launch this candidate.

Linux/macOS, when a native artifact has passed its gate:

```sh
sha256sum factory-linux-x86_64.tar.gz
mkdir 'ASF candidate'
tar -xzf factory-linux-x86_64.tar.gz -C 'ASF candidate'
cd 'ASF candidate'
./start-workspace.sh "$HOME/dev"
```

On macOS use the matching native archive and `shasum -a 256`. Unsigned macOS
installation/Gatekeeper and notarization are not qualified. Do not disable
system-wide security settings to run a candidate.

## Explicit CLI startup

```sh
factory start --workspace --no-default-project --allow-project-root /your/dev
```

The server binds only to `127.0.0.1:4321`. Use `--port` to select another port.
`--no-browser` prints the address without opening a browser. `--registry FILE`
selects an isolated application registry. The original `factory start` behavior
and `/` V1 entry remain available for initialized projects. V2 is explicitly
selected through `--workspace` or `/workspace.html`.

Application state belongs in the platform application-data directory (Windows
LOCALAPPDATA, macOS Application Support, Linux XDG data home). Project execution
state belongs in each project's `.factory` directory. The registry also has
workspace, IDE, presence and preferences sidecars. Never place these files in
a shared/untrusted directory. See UPGRADE.md before replacing binaries.

## Startup recovery

An occupied port fails before opening project state. Stop the known ASF instance
or choose a free port; do not kill unrelated processes. A second process serving
the same project is refused by project ownership. Missing CLI/LSP/DAP tools are
configuration problems, not evidence that the application failed to install.
Use Settings and Diagnostics for actual availability and policy errors.

Normal Ctrl+C (Windows also Ctrl+Break; Unix also SIGTERM) closes admission and
uses existing lifecycle/process services. A forced termination, OS crash or power
loss instead requires restart reconciliation. An interrupted or uncertain
operation is not promised to be safely suspended. Suspended missions require
manual resume after verification.

## Removal

Stop ASF, verify its owned operations have stopped, and remove only the extracted
application directory. Preserve user repositories, `.factory` evidence, Git
worktrees and application data. There is no automatic data erasure on uninstall.
The existing download installers remain checksum-enforcing binary installers;
they must not be pointed at an unpublished candidate on GitHub. Local qualification
can serve verified archives from a loopback-only fixture server.
