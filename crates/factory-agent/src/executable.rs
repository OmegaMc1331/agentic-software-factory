use std::ffi::{OsStr, OsString};
#[cfg(any(windows, test))]
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(windows)]
const WINDOWS_DEFAULT_PATHEXT: &str = ".COM;.EXE;.BAT;.CMD";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedExecutableKind {
    Native,
    WindowsBatch,
    NpmShim,
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

    pub fn process_launch(&self, args: &[String]) -> LaunchCommand {
        let mut launch_args = self.prefix_args.clone();
        launch_args.extend(args.iter().map(OsString::from));
        LaunchCommand {
            program: self.program.clone(),
            args: launch_args,
        }
    }

    pub fn pty_launch(&self, args: &[String]) -> Result<LaunchCommand, String> {
        #[cfg(windows)]
        if self.kind == ResolvedExecutableKind::WindowsBatch {
            return windows_batch_pty_launch(&self.path, args);
        }
        Ok(self.process_launch(args))
    }
}

pub fn resolve_executable(command: &str) -> Option<ResolvedExecutable> {
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
) -> Option<ResolvedExecutable> {
    let configured = Path::new(command);
    let path_entries: Vec<PathBuf> = std::env::split_paths(path_value).collect();
    if has_path_components(configured) {
        let base = if configured.is_absolute() {
            configured.to_path_buf()
        } else {
            std::env::current_dir().ok()?.join(configured)
        };
        return resolve_candidate_set(&base, pathext_value, 0);
    }

    for (index, directory) in path_entries.into_iter().enumerate() {
        let directory = if directory.is_absolute() {
            directory
        } else {
            std::env::current_dir().ok()?.join(directory)
        };
        if let Some(resolved) =
            resolve_candidate_set(&directory.join(configured), pathext_value, index + 1)
        {
            return Some(resolved);
        }
    }
    None
}

fn has_path_components(path: &Path) -> bool {
    path.is_absolute() || path.components().count() > 1
}

fn resolve_candidate_set(
    candidate: &Path,
    pathext_value: &OsStr,
    path_entries_checked: usize,
) -> Option<ResolvedExecutable> {
    #[cfg(not(windows))]
    let _ = pathext_value;

    if let Some(resolved) = resolve_candidate(candidate, path_entries_checked) {
        return Some(resolved);
    }

    #[cfg(windows)]
    if candidate.extension().is_none() {
        for extension in windows_extensions(pathext_value) {
            let candidate = candidate.with_extension(extension.trim_start_matches('.'));
            if let Some(resolved) = resolve_candidate(&candidate, path_entries_checked) {
                return Some(resolved);
            }
        }
    }

    None
}

fn resolve_candidate(candidate: &Path, path_entries_checked: usize) -> Option<ResolvedExecutable> {
    if !candidate.is_file() {
        return None;
    }

    #[cfg(windows)]
    {
        let extension = candidate
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "cmd" | "bat" => Some(
                npm_shim_launcher(candidate, path_entries_checked).unwrap_or_else(|| {
                    ResolvedExecutable {
                        path: candidate.to_path_buf(),
                        program: candidate.to_path_buf(),
                        prefix_args: Vec::new(),
                        kind: ResolvedExecutableKind::WindowsBatch,
                        path_entries_checked,
                    }
                }),
            ),
            "exe" | "com" => Some(native(candidate, path_entries_checked)),
            "" if has_windows_executable_header(candidate) => {
                Some(native(candidate, path_entries_checked))
            }
            _ => None,
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let metadata = candidate.metadata().ok()?;
        (metadata.permissions().mode() & 0o111 != 0)
            .then(|| native(candidate, path_entries_checked))
    }

    #[cfg(not(any(unix, windows)))]
    {
        Some(native(candidate, path_entries_checked))
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

#[cfg(windows)]
fn has_windows_executable_header(path: &Path) -> bool {
    use std::io::Read;
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return false,
    };
    let mut header = [0_u8; 2];
    file.read_exact(&mut header).is_ok() && header == *b"MZ"
}

#[cfg(windows)]
fn npm_shim_launcher(path: &Path, path_entries_checked: usize) -> Option<ResolvedExecutable> {
    let contents = fs::read_to_string(path).ok()?;
    let parent = path.parent()?;
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
        let node = if parent.join("node.exe").is_file() {
            native(&parent.join("node.exe"), path_entries_checked)
        } else {
            resolve_executable("node")?
        };
        if node.kind == ResolvedExecutableKind::WindowsBatch {
            return None;
        }
        let mut prefix_args = node.prefix_args;
        prefix_args.push(script.as_os_str().to_owned());
        return Some(ResolvedExecutable {
            path: path.to_path_buf(),
            program: node.program,
            prefix_args,
            kind: ResolvedExecutableKind::NpmShim,
            path_entries_checked,
        });
    }

    if let Some(program) = targets.iter().find(|target| {
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
        return Some(ResolvedExecutable {
            path: path.to_path_buf(),
            program: program.clone(),
            prefix_args: Vec::new(),
            kind: ResolvedExecutableKind::NpmShim,
            path_entries_checked,
        });
    }
    None
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

#[cfg(windows)]
fn windows_batch_pty_launch(path: &Path, args: &[String]) -> Result<LaunchCommand, String> {
    let path = path.to_string_lossy();
    let mut tokens = Vec::with_capacity(args.len() + 1);
    tokens.push(quote_batch_token(&path)?);
    for argument in args {
        tokens.push(quote_batch_token(argument)?);
    }
    let command_line = format!("\"{}\"", tokens.join(" "));
    let comspec = std::env::var_os("ComSpec")
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .or_else(|| resolve_executable("cmd.exe").map(|resolved| resolved.program))
        .ok_or_else(|| "cmd.exe was not found for the configured batch agent".to_string())?;
    Ok(LaunchCommand {
        program: comspec,
        args: vec!["/d".into(), "/s".into(), "/c".into(), command_line.into()],
    })
}

#[cfg(windows)]
fn quote_batch_token(value: &str) -> Result<String, String> {
    if value.chars().any(|character| {
        matches!(
            character,
            '\0' | '\r' | '\n' | '"' | '%' | '!' | '^' | '&' | '|' | '<' | '>'
        )
    }) {
        return Err(
            "the batch-backed interactive invocation contains unsafe cmd.exe characters"
                .to_string(),
        );
    }
    Ok(format!("\"{value}\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

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

        let resolved =
            resolve_with_environment("fake-agent", directory.path().as_os_str(), OsStr::new(""))
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

        let resolved = resolve_with_environment(
            "fake-agent",
            directory.path().as_os_str(),
            OsStr::new(".CMD;.EXE"),
        )
        .unwrap();
        assert_eq!(resolved.path(), shim);
        assert_eq!(resolved.kind(), ResolvedExecutableKind::WindowsBatch);

        let batch = directory.path().join("batch-agent.bat");
        fs::write(&batch, "@echo off\r\necho batch\r\n").unwrap();
        let resolved = resolve_with_environment(
            "batch-agent",
            directory.path().as_os_str(),
            OsStr::new(".BAT;.EXE"),
        )
        .unwrap();
        assert_eq!(resolved.path(), batch);
        assert_eq!(resolved.kind(), ResolvedExecutableKind::WindowsBatch);

        let without_extension = directory.path().join("fake-agent");
        let resolved = resolve_with_environment(
            without_extension.to_str().unwrap(),
            OsStr::new(""),
            OsStr::new(".CMD;.EXE"),
        )
        .unwrap();
        assert_eq!(resolved.path(), shim);
    }

    #[cfg(windows)]
    #[test]
    fn unwraps_an_npm_native_shim() {
        let directory = TempDir::new().unwrap();
        let native = directory.path().join("fake-agent.exe");
        fs::copy(std::env::var_os("ComSpec").unwrap(), &native).unwrap();
        let shim = directory.path().join("fake-agent.cmd");
        fs::write(&shim, "@ECHO off\r\n\"%dp0%\\fake-agent.exe\" %*\r\n").unwrap();

        let resolved = resolve_with_environment(
            "fake-agent",
            directory.path().as_os_str(),
            OsStr::new(".CMD;.EXE"),
        )
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

        let resolved = resolve_with_environment(
            "fake-agent",
            directory.path().as_os_str(),
            OsStr::new(".CMD;.EXE"),
        )
        .unwrap();
        assert_eq!(resolved.path(), shim);
        assert_eq!(resolved.launch_program(), node);
        assert_eq!(resolved.kind(), ResolvedExecutableKind::NpmShim);
        let launch = resolved.process_launch(&[]);
        assert_eq!(launch.args.len(), 1);
        assert_eq!(
            fs::canonicalize(PathBuf::from(&launch.args[0])).unwrap(),
            fs::canonicalize(script).unwrap()
        );
    }

    #[cfg(windows)]
    #[test]
    fn resolves_and_executes_a_native_exe_from_path() {
        let directory = TempDir::new().unwrap();
        let executable = directory.path().join("fake-native.exe");
        fs::copy(std::env::var_os("ComSpec").unwrap(), &executable).unwrap();
        let resolved = resolve_with_environment(
            "fake-native",
            directory.path().as_os_str(),
            OsStr::new(".EXE;.CMD"),
        )
        .unwrap();
        let launch =
            resolved.process_launch(&["/d".into(), "/c".into(), "echo".into(), "NATIVE_OK".into()]);
        let output = std::process::Command::new(launch.program)
            .args(launch.args)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("NATIVE_OK"));
    }

    #[cfg(windows)]
    #[test]
    fn missing_windows_command_is_not_resolved() {
        let directory = TempDir::new().unwrap();
        assert!(resolve_with_environment(
            "missing-agent",
            directory.path().as_os_str(),
            OsStr::new(".COM;.EXE;.BAT;.CMD"),
        )
        .is_none());
    }
}
