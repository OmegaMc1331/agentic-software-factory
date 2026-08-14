use std::ffi::{OsStr, OsString};
#[cfg(any(windows, test))]
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(windows)]
const WINDOWS_DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

/// Guards shim -> node -> shim resolution chains against infinite recursion.
#[cfg(windows)]
const MAX_SHIM_RESOLUTION_DEPTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedExecutableKind {
    Native,
    WindowsBatch,
    NpmShim,
}

impl ResolvedExecutableKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ResolvedExecutableKind::Native => "native",
            ResolvedExecutableKind::WindowsBatch => "windows_batch",
            ResolvedExecutableKind::NpmShim => "npm_shim",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedExecutable {
    path: PathBuf,
    program: PathBuf,
    prefix_args: Vec<OsString>,
    kind: ResolvedExecutableKind,
    path_entries_checked: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchCommand {
    pub program: PathBuf,
    pub args: Vec<OsString>,
}

/// A file that was found for a configured command but cannot be launched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokenExecutable {
    /// The `.cmd`/`.bat` shim that referenced the broken file, when one was involved.
    pub shim: Option<PathBuf>,
    /// The file that exists but is not launchable.
    pub path: PathBuf,
    /// Why the file cannot be launched.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutableResolution {
    Resolved(ResolvedExecutable),
    NotFound,
    Broken(BrokenExecutable),
}

impl ExecutableResolution {
    pub fn resolved(self) -> Option<ResolvedExecutable> {
        match self {
            ExecutableResolution::Resolved(resolved) => Some(resolved),
            _ => None,
        }
    }

    pub fn broken(&self) -> Option<&BrokenExecutable> {
        match self {
            ExecutableResolution::Broken(broken) => Some(broken),
            _ => None,
        }
    }
}

impl ResolvedExecutable {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn launch_program(&self) -> &Path {
        &self.program
    }

    pub fn kind(&self) -> ResolvedExecutableKind {
        self.kind
    }

    pub fn path_entries_checked(&self) -> usize {
        self.path_entries_checked
    }

    pub fn process_launch(&self, args: &[String]) -> Result<LaunchCommand, String> {
        #[cfg(windows)]
        if self.kind == ResolvedExecutableKind::WindowsBatch {
            for argument in args {
                validate_batch_argument(argument)?;
            }
        }
        Ok(self.direct_launch(args))
    }

    pub fn pty_launch(&self, args: &[String]) -> Result<LaunchCommand, String> {
        self.process_launch(args)
    }

    fn direct_launch(&self, args: &[String]) -> LaunchCommand {
        let mut launch_args = self.prefix_args.clone();
        launch_args.extend(args.iter().map(OsString::from));
        LaunchCommand {
            program: self.program.clone(),
            args: launch_args,
        }
    }
}

pub fn resolve_executable(command: &str) -> ExecutableResolution {
    let path = std::env::var_os("PATH").unwrap_or_default();
    #[cfg(windows)]
    let pathext =
        std::env::var_os("PATHEXT").unwrap_or_else(|| OsString::from(WINDOWS_DEFAULT_PATHEXT));
    #[cfg(not(windows))]
    let pathext = OsString::new();
    resolve_with_environment(command, &path, &pathext)
}

pub fn runtime_path_entries() -> usize {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).count())
        .unwrap_or(0)
}

fn resolve_with_environment(
    command: &str,
    path_value: &OsStr,
    pathext_value: &OsStr,
) -> ExecutableResolution {
    resolve_at_depth(command, path_value, pathext_value, 0)
}

fn resolve_at_depth(
    command: &str,
    path_value: &OsStr,
    pathext_value: &OsStr,
    depth: usize,
) -> ExecutableResolution {
    let configured = Path::new(command);
    let path_entries: Vec<PathBuf> = std::env::split_paths(path_value).collect();
    if has_path_components(configured) {
        let Some(base) = configured
            .is_absolute()
            .then(|| configured.to_path_buf())
            .or_else(|| std::env::current_dir().ok().map(|cwd| cwd.join(configured)))
        else {
            return ExecutableResolution::NotFound;
        };
        return resolve_candidate_set(&base, path_value, pathext_value, 0, depth);
    }

    for (index, directory) in path_entries.into_iter().enumerate() {
        let directory = if directory.is_absolute() {
            directory
        } else {
            match std::env::current_dir() {
                Ok(cwd) => cwd.join(directory),
                Err(_) => continue,
            }
        };
        match resolve_candidate_set(
            &directory.join(configured),
            path_value,
            pathext_value,
            index + 1,
            depth,
        ) {
            ExecutableResolution::NotFound => {}
            resolution => return resolution,
        }
    }
    ExecutableResolution::NotFound
}

fn has_path_components(path: &Path) -> bool {
    path.is_absolute() || path.components().count() > 1
}

fn resolve_candidate_set(
    candidate: &Path,
    path_value: &OsStr,
    pathext_value: &OsStr,
    path_entries_checked: usize,
    depth: usize,
) -> ExecutableResolution {
    #[cfg(not(windows))]
    {
        resolve_candidate(
            candidate,
            path_value,
            pathext_value,
            path_entries_checked,
            depth,
        )
    }

    #[cfg(windows)]
    {
        let mut resolution = resolve_candidate(
            candidate,
            path_value,
            pathext_value,
            path_entries_checked,
            depth,
        );

        if resolution == ExecutableResolution::NotFound && candidate.extension().is_none() {
            for extension in windows_extensions(pathext_value) {
                let candidate = candidate.with_extension(extension.trim_start_matches('.'));
                resolution = resolve_candidate(
                    &candidate,
                    path_value,
                    pathext_value,
                    path_entries_checked,
                    depth,
                );
                if resolution != ExecutableResolution::NotFound {
                    break;
                }
            }
        }

        resolution
    }
}

fn resolve_candidate(
    candidate: &Path,
    path_value: &OsStr,
    pathext_value: &OsStr,
    path_entries_checked: usize,
    depth: usize,
) -> ExecutableResolution {
    if !candidate.is_file() {
        return ExecutableResolution::NotFound;
    }

    // The environment and shim depth are only consulted by the Windows
    // resolution branches below.
    #[cfg(not(windows))]
    let _ = (path_value, pathext_value, depth);

    #[cfg(windows)]
    {
        let extension = candidate
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "cmd" | "bat" => {
                match npm_shim_launcher(
                    candidate,
                    path_entries_checked,
                    depth,
                    path_value,
                    pathext_value,
                ) {
                    // An unwrappable shim is still launchable as a generic batch file.
                    ExecutableResolution::NotFound => {
                        ExecutableResolution::Resolved(ResolvedExecutable {
                            path: candidate.to_path_buf(),
                            program: candidate.to_path_buf(),
                            prefix_args: Vec::new(),
                            kind: ResolvedExecutableKind::WindowsBatch,
                            path_entries_checked,
                        })
                    }
                    resolution => resolution,
                }
            }
            "exe" | "com" => match windows_executable_problem(candidate) {
                None => ExecutableResolution::Resolved(native(candidate, path_entries_checked)),
                Some(reason) => ExecutableResolution::Broken(BrokenExecutable {
                    shim: None,
                    path: candidate.to_path_buf(),
                    reason: reason.to_string(),
                }),
            },
            // Extensionless files are only launchable when they carry a valid
            // executable header; anything else is simply not a candidate.
            "" => match windows_executable_problem(candidate) {
                None => ExecutableResolution::Resolved(native(candidate, path_entries_checked)),
                Some(_) => ExecutableResolution::NotFound,
            },
            _ => ExecutableResolution::NotFound,
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let Ok(metadata) = candidate.metadata() else {
            return ExecutableResolution::NotFound;
        };
        if metadata.permissions().mode() & 0o111 != 0 {
            ExecutableResolution::Resolved(native(candidate, path_entries_checked))
        } else {
            ExecutableResolution::NotFound
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        ExecutableResolution::Resolved(native(candidate, path_entries_checked))
    }
}

fn native(path: &Path, path_entries_checked: usize) -> ResolvedExecutable {
    ResolvedExecutable {
        path: path.to_path_buf(),
        program: path.to_path_buf(),
        prefix_args: Vec::new(),
        kind: ResolvedExecutableKind::Native,
        path_entries_checked,
    }
}

#[cfg(windows)]
fn windows_extensions(value: &OsStr) -> Vec<String> {
    let value = value.to_string_lossy();
    let value = if value.trim().is_empty() {
        WINDOWS_DEFAULT_PATHEXT
    } else {
        value.as_ref()
    };
    value
        .split(';')
        .map(str::trim)
        .filter(|extension| !extension.is_empty())
        .map(str::to_ascii_lowercase)
        .filter(|extension| matches!(extension.as_str(), ".exe" | ".com" | ".cmd" | ".bat"))
        .collect()
}

/// The single canonical validation for native Windows executables: a candidate
/// must carry a DOS `MZ` header and a `PE\0\0` signature, regardless of its
/// file extension. Returns `None` when the file is launchable.
#[cfg(windows)]
fn windows_executable_problem(path: &Path) -> Option<&'static str> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return Some("file cannot be opened"),
    };
    let mut dos_header = [0_u8; 2];
    if file.read_exact(&mut dos_header).is_err() || dos_header != *b"MZ" {
        return Some("missing the DOS MZ header of a Windows executable");
    }
    let mut pe_offset_bytes = [0_u8; 4];
    if file.seek(SeekFrom::Start(0x3C)).is_err() || file.read_exact(&mut pe_offset_bytes).is_err() {
        return Some("missing the PE header offset of a Windows executable");
    }
    let pe_offset = u32::from_le_bytes(pe_offset_bytes) as u64;
    let Ok(metadata) = file.metadata() else {
        return Some("file cannot be inspected");
    };
    if (0x40..metadata.len()).contains(&pe_offset) {
        let mut signature = [0_u8; 4];
        if file.seek(SeekFrom::Start(pe_offset)).is_err()
            || file.read_exact(&mut signature).is_err()
            || signature != *b"PE\0\0"
        {
            return Some("missing the PE signature of a Windows executable");
        }
        None
    } else {
        Some("invalid PE header offset in the Windows executable")
    }
}

#[cfg(windows)]
fn npm_shim_launcher(
    path: &Path,
    path_entries_checked: usize,
    depth: usize,
    path_value: &OsStr,
    pathext_value: &OsStr,
) -> ExecutableResolution {
    if depth >= MAX_SHIM_RESOLUTION_DEPTH {
        return ExecutableResolution::NotFound;
    }
    let Ok(contents) = fs::read_to_string(path) else {
        return ExecutableResolution::NotFound;
    };
    let Some(parent) = path.parent() else {
        return ExecutableResolution::NotFound;
    };
    let targets: Vec<PathBuf> = contents
        .lines()
        .flat_map(quoted_tokens)
        .filter_map(|token| expand_npm_directory(parent, token))
        .collect();

    let script = targets.iter().find(|target| {
        target.is_file()
            && target
                .extension()
                .and_then(OsStr::to_str)
                .is_some_and(|extension| extension.eq_ignore_ascii_case("js"))
    });
    if let Some(script) = script {
        let sibling_node = parent.join("node.exe");
        let node = if sibling_node.is_file() {
            match windows_executable_problem(&sibling_node) {
                None => native(&sibling_node, path_entries_checked),
                Some(reason) => {
                    return ExecutableResolution::Broken(BrokenExecutable {
                        shim: Some(path.to_path_buf()),
                        path: sibling_node,
                        reason: reason.to_string(),
                    })
                }
            }
        } else {
            match resolve_at_depth("node", path_value, pathext_value, depth + 1) {
                ExecutableResolution::Resolved(node)
                    if node.kind != ResolvedExecutableKind::WindowsBatch =>
                {
                    node
                }
                ExecutableResolution::Broken(broken) => {
                    return ExecutableResolution::Broken(broken)
                }
                _ => return ExecutableResolution::NotFound,
            }
        };
        let mut prefix_args = node.prefix_args;
        prefix_args.push(script.as_os_str().to_owned());
        return ExecutableResolution::Resolved(ResolvedExecutable {
            path: path.to_path_buf(),
            program: node.program,
            prefix_args,
            kind: ResolvedExecutableKind::NpmShim,
            path_entries_checked,
        });
    }

    if let Some(target) = targets.iter().find(|target| {
        target.is_file()
            && matches!(
                target
                    .extension()
                    .and_then(OsStr::to_str)
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .as_str(),
                "exe" | "com"
            )
    }) {
        return match windows_executable_problem(target) {
            None => ExecutableResolution::Resolved(ResolvedExecutable {
                path: path.to_path_buf(),
                program: target.clone(),
                prefix_args: Vec::new(),
                kind: ResolvedExecutableKind::NpmShim,
                path_entries_checked,
            }),
            Some(reason) => ExecutableResolution::Broken(BrokenExecutable {
                shim: Some(path.to_path_buf()),
                path: target.clone(),
                reason: reason.to_string(),
            }),
        };
    }
    ExecutableResolution::NotFound
}

#[cfg(windows)]
fn quoted_tokens(line: &str) -> Vec<&str> {
    let mut tokens = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find('"') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('"') else { break };
        tokens.push(&rest[..end]);
        rest = &rest[end + 1..];
    }
    tokens
}

#[cfg(windows)]
fn expand_npm_directory(parent: &Path, token: &str) -> Option<PathBuf> {
    let lower = token.to_ascii_lowercase();
    let rest = lower
        .starts_with("%dp0%")
        .then(|| &token[5..])?
        .trim_start_matches(['\\', '/']);
    Some(parent.join(rest))
}

/// A generic `.cmd`/`.bat` is launched as itself: both `std::process::Command`
/// (since the BatBadBut fix) and ConPTY's `CreateProcess` route batch files
/// through `cmd.exe` with correct per-argument quoting, so Factory never
/// concatenates arguments into a command string. Because `cmd.exe` re-parses
/// that implicit invocation, any argument containing characters cmd would
/// reinterpret (percent expansion, caret escapes, delayed expansion,
/// redirection) is rejected instead of spawned.
#[cfg(windows)]
fn validate_batch_argument(value: &str) -> Result<(), String> {
    if value.chars().any(|character| {
        matches!(
            character,
            '\0' | '\r' | '\n' | '"' | '%' | '!' | '^' | '&' | '|' | '<' | '>'
        )
    }) {
        return Err(format!(
            "the batch-backed invocation argument `{value}` contains unsafe cmd.exe characters"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn resolve(command: &str, path: &Path, pathext: &str) -> ExecutableResolution {
        resolve_with_environment(command, path.as_os_str(), OsStr::new(pathext))
    }

    #[cfg(unix)]
    #[test]
    fn resolves_an_executable_from_unix_path() {
        use std::os::unix::fs::PermissionsExt;
        let directory = TempDir::new().unwrap();
        let executable = directory.path().join("fake-agent");
        fs::write(&executable, "#!/bin/sh\necho ok\n").unwrap();
        let mut permissions = executable.metadata().unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&executable, permissions).unwrap();

        let resolved = resolve("fake-agent", directory.path(), "")
            .resolved()
            .unwrap();
        assert_eq!(resolved.path(), executable);
        assert_eq!(resolved.kind(), ResolvedExecutableKind::Native);
    }

    #[cfg(windows)]
    #[test]
    fn resolves_windows_pathext_candidates_and_explicit_paths() {
        let directory = TempDir::new().unwrap();
        let shim = directory.path().join("fake-agent.cmd");
        fs::write(&shim, "@echo off\r\necho ok\r\n").unwrap();

        let resolved = resolve("fake-agent", directory.path(), ".CMD;.EXE")
            .resolved()
            .unwrap();
        assert_eq!(resolved.path(), shim);
        assert_eq!(resolved.kind(), ResolvedExecutableKind::WindowsBatch);

        let batch = directory.path().join("batch-agent.bat");
        fs::write(&batch, "@echo off\r\necho batch\r\n").unwrap();
        let resolved = resolve("batch-agent", directory.path(), ".BAT;.EXE")
            .resolved()
            .unwrap();
        assert_eq!(resolved.path(), batch);
        assert_eq!(resolved.kind(), ResolvedExecutableKind::WindowsBatch);

        let without_extension = directory.path().join("fake-agent");
        let resolved = resolve(
            without_extension.to_str().unwrap(),
            Path::new(""),
            ".CMD;.EXE",
        )
        .resolved()
        .unwrap();
        assert_eq!(resolved.path(), shim);
    }

    #[cfg(windows)]
    #[test]
    fn a_text_file_named_exe_is_broken_not_native() {
        let directory = TempDir::new().unwrap();
        let fake = directory.path().join("fake-agent.exe");
        fs::write(
            &fake,
            "#!/bin/sh\nthis is a placeholder script, not an executable\n",
        )
        .unwrap();

        let resolution = resolve("fake-agent", directory.path(), ".EXE;.CMD");
        let broken = resolution.broken().expect("text .exe must not resolve");
        assert_eq!(broken.path, fake);
        assert_eq!(broken.shim, None);
        assert!(broken.reason.contains("DOS MZ header"));
        assert!(resolution.resolved().is_none());
    }

    #[cfg(windows)]
    #[test]
    fn an_exe_with_mz_bytes_but_no_pe_header_is_broken() {
        let directory = TempDir::new().unwrap();
        let fake = directory.path().join("fake-agent.exe");
        // MZ magic, but the e_lfanew slot points outside the file, so there is
        // no PE signature anywhere: this must not be treated as launchable.
        let mut bytes = vec![b'.'; 0x80];
        bytes[0] = b'M';
        bytes[1] = b'Z';
        bytes[0x3C..0x40].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        fs::write(&fake, bytes).unwrap();

        let resolution = resolve("fake-agent", directory.path(), ".EXE");
        let broken = resolution.broken().expect("MZ-only file must not resolve");
        assert_eq!(broken.path, fake);
        assert!(broken.reason.contains("PE header"));
    }

    #[cfg(windows)]
    #[test]
    fn a_broken_npm_shim_target_is_reported_with_diagnostics() {
        let directory = TempDir::new().unwrap();
        let target_dir = directory.path().join("fake-package/bin");
        fs::create_dir_all(&target_dir).unwrap();
        let target = target_dir.join("fake-agent.exe");
        fs::write(&target, "text placeholder shipped by a broken package\n").unwrap();
        let shim = directory.path().join("fake-agent.cmd");
        fs::write(
            &shim,
            "@ECHO off\r\n\"%dp0%\\fake-package\\bin\\fake-agent.exe\" %*\r\n",
        )
        .unwrap();

        let resolution = resolve("fake-agent", directory.path(), ".CMD;.EXE");
        let broken = resolution
            .broken()
            .expect("broken shim target must be reported");
        assert_eq!(broken.shim.as_deref(), Some(shim.as_path()));
        assert_eq!(broken.path, target);
        assert!(broken.reason.contains("Windows executable"));
        assert!(resolution.resolved().is_none());
    }

    #[cfg(windows)]
    #[test]
    fn a_broken_sibling_node_exe_is_reported() {
        let directory = TempDir::new().unwrap();
        fs::write(directory.path().join("node.exe"), "definitely not node\n").unwrap();
        let script = directory.path().join("node_modules/pkg/agent.js");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(&script, "console.log('ok')").unwrap();
        let shim = directory.path().join("fake-agent.cmd");
        fs::write(
            &shim,
            "@ECHO off\r\n\"%dp0%\\node.exe\" \"%dp0%\\node_modules\\pkg\\agent.js\" %*\r\n",
        )
        .unwrap();

        let resolution = resolve("fake-agent", directory.path(), ".CMD;.EXE");
        let broken = resolution
            .broken()
            .expect("broken sibling node.exe must be reported");
        assert_eq!(broken.path, directory.path().join("node.exe"));
        assert_eq!(broken.shim.as_deref(), Some(shim.as_path()));
    }

    #[cfg(windows)]
    #[test]
    fn unwraps_an_npm_native_shim() {
        let directory = TempDir::new().unwrap();
        let native = directory.path().join("fake-agent.exe");
        fs::copy(std::env::var_os("ComSpec").unwrap(), &native).unwrap();
        let shim = directory.path().join("fake-agent.cmd");
        fs::write(&shim, "@ECHO off\r\n\"%dp0%\\fake-agent.exe\" %*\r\n").unwrap();

        let resolved = resolve("fake-agent", directory.path(), ".CMD;.EXE")
            .resolved()
            .unwrap();
        assert_eq!(resolved.path(), shim);
        assert_eq!(resolved.launch_program(), native);
        assert_eq!(resolved.kind(), ResolvedExecutableKind::NpmShim);
        let pty = resolved
            .pty_launch(&["/d".into(), "/c".into(), "echo".into(), "PTY_OK".into()])
            .unwrap();
        assert_eq!(pty.program, native);
        assert_eq!(pty.args.last(), Some(&OsString::from("PTY_OK")));
    }

    #[cfg(windows)]
    #[test]
    fn unwraps_an_npm_node_shim() {
        let directory = TempDir::new().unwrap();
        let node = directory.path().join("node.exe");
        fs::copy(std::env::var_os("ComSpec").unwrap(), &node).unwrap();
        let script = directory.path().join("node_modules/pkg/agent.js");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(&script, "console.log('ok')").unwrap();
        let shim = directory.path().join("fake-agent.cmd");
        fs::write(
            &shim,
            "@ECHO off\r\n\"%dp0%\\node.exe\" \"%dp0%\\node_modules\\pkg\\agent.js\" %*\r\n",
        )
        .unwrap();

        let resolved = resolve("fake-agent", directory.path(), ".CMD;.EXE")
            .resolved()
            .unwrap();
        assert_eq!(resolved.path(), shim);
        assert_eq!(resolved.launch_program(), node);
        assert_eq!(resolved.kind(), ResolvedExecutableKind::NpmShim);
        let launch = resolved.process_launch(&[]).unwrap();
        assert_eq!(launch.args.len(), 1);
        assert_eq!(
            fs::canonicalize(PathBuf::from(&launch.args[0])).unwrap(),
            fs::canonicalize(script).unwrap()
        );
    }

    #[cfg(windows)]
    #[test]
    fn a_node_shim_resolves_node_from_the_path_when_no_sibling_exists() {
        let directory = TempDir::new().unwrap();
        let node_dir = TempDir::new().unwrap();
        let node = node_dir.path().join("node.exe");
        fs::copy(std::env::var_os("ComSpec").unwrap(), &node).unwrap();
        let script = directory.path().join("node_modules/pkg/agent.js");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(&script, "console.log('ok')").unwrap();
        let shim = directory.path().join("fake-agent.cmd");
        fs::write(
            &shim,
            "@ECHO off\r\n\"node\" \"%dp0%\\node_modules\\pkg\\agent.js\" %*\r\n",
        )
        .unwrap();

        let combined = OsString::from(
            format!(
                "{};{}",
                directory.path().display(),
                node_dir.path().display()
            )
            .replace('/', "\\"),
        );
        let resolved =
            resolve_with_environment("fake-agent", combined.as_os_str(), OsStr::new(".CMD;.EXE"))
                .resolved()
                .unwrap();
        assert_eq!(resolved.kind(), ResolvedExecutableKind::NpmShim);
        assert_eq!(resolved.launch_program(), node);
    }

    #[cfg(windows)]
    #[test]
    fn self_referential_node_shims_do_not_recurse_forever() {
        let directory = TempDir::new().unwrap();
        let script = directory.path().join("agent.js");
        fs::write(&script, "console.log('ok')").unwrap();
        // node.cmd looks like a JS shim itself and would resolve `node` again.
        fs::write(
            directory.path().join("node.cmd"),
            "@ECHO off\r\n\"%dp0%\\node.exe\" \"%dp0%\\agent.js\" %*\r\n",
        )
        .unwrap();
        fs::write(
            directory.path().join("fake-agent.cmd"),
            "@ECHO off\r\n\"%dp0%\\node.exe\" \"%dp0%\\agent.js\" %*\r\n",
        )
        .unwrap();

        let resolved = resolve("fake-agent", directory.path(), ".CMD;.EXE")
            .resolved()
            .expect("depth guard must terminate and fall back to a batch launcher");
        assert_eq!(resolved.kind(), ResolvedExecutableKind::WindowsBatch);
    }

    #[cfg(windows)]
    #[test]
    fn resolves_and_executes_a_native_exe_from_path() {
        let directory = TempDir::new().unwrap();
        let executable = directory.path().join("fake-native.exe");
        fs::copy(std::env::var_os("ComSpec").unwrap(), &executable).unwrap();
        let resolved = resolve("fake-native", directory.path(), ".EXE;.CMD")
            .resolved()
            .unwrap();
        let launch = resolved
            .process_launch(&["/d".into(), "/c".into(), "echo".into(), "NATIVE_OK".into()])
            .unwrap();
        let output = std::process::Command::new(launch.program)
            .args(launch.args)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("NATIVE_OK"));
    }

    #[cfg(windows)]
    #[test]
    fn raw_batch_launches_consistently_for_process_and_pty() {
        let directory = TempDir::new().unwrap();
        let batch = directory.path().join("fake-agent.cmd");
        fs::write(&batch, "@echo off\r\necho ARG=%~1\r\n").unwrap();

        let resolved = resolve("fake-agent", directory.path(), ".CMD;.EXE")
            .resolved()
            .unwrap();
        assert_eq!(resolved.kind(), ResolvedExecutableKind::WindowsBatch);
        let args = vec!["BATCH_OK".to_string()];
        let process = resolved.process_launch(&args).unwrap();
        let pty = resolved.pty_launch(&args).unwrap();
        assert_eq!(process.program, batch);
        assert_eq!(pty.program, batch);
        assert_eq!(process.args, pty.args);

        let output = std::process::Command::new(process.program)
            .args(process.args)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("BATCH_OK"));

        let pty_output = std::process::Command::new(pty.program)
            .args(pty.args)
            .output()
            .unwrap();
        assert!(pty_output.status.success());
        assert!(String::from_utf8_lossy(&pty_output.stdout).contains("BATCH_OK"));
    }

    #[cfg(windows)]
    #[test]
    fn batch_launch_survives_spaces_in_paths_and_arguments() {
        let outer = TempDir::new().unwrap();
        let spaced = outer.path().join("factory spaced dir");
        fs::create_dir_all(&spaced).unwrap();
        let batch = spaced.join("fake agent.cmd");
        fs::write(&batch, "@echo off\r\necho ARG=%~1\r\n").unwrap();

        let resolved = resolve_with_environment(
            batch.to_str().unwrap(),
            OsStr::new(""),
            OsStr::new(".CMD;.EXE"),
        )
        .resolved()
        .unwrap();
        assert_eq!(resolved.kind(), ResolvedExecutableKind::WindowsBatch);
        let launch = resolved.process_launch(&["SPACE ARG".into()]).unwrap();
        let output = std::process::Command::new(launch.program)
            .args(launch.args)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "ARG=SPACE ARG"
        );
    }

    #[cfg(windows)]
    #[test]
    fn batch_launch_rejects_unsafe_cmd_characters() {
        let directory = TempDir::new().unwrap();
        let batch = directory.path().join("fake-agent.cmd");
        fs::write(&batch, "@echo off\r\necho ARG=%~1\r\n").unwrap();

        let resolved = resolve("fake-agent", directory.path(), ".CMD")
            .resolved()
            .unwrap();
        let error = resolved
            .process_launch(&["safe".into(), "bad&command".into()])
            .unwrap_err();
        assert!(error.contains("unsafe cmd.exe characters"));
        assert!(resolved.pty_launch(&["bad&command".into()]).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn missing_windows_command_is_not_resolved() {
        let directory = TempDir::new().unwrap();
        let resolution = resolve("missing-agent", directory.path(), ".COM;.EXE;.BAT;.CMD");
        assert_eq!(resolution, ExecutableResolution::NotFound);
    }
}
