use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::{
    pump_terminal_output, subscribe_terminal_output, terminal_process, terminal_size,
    InteractiveTerminal, RuntimeBudget, RuntimeError, TerminalOutput, TerminalSubscription,
};
use portable_pty::{native_pty_system, CommandBuilder};

pub struct DevelopmentTerminal {
    inner: Arc<InteractiveTerminal>,
    pub pid: u32,
    exit: Arc<Mutex<Option<u32>>>,
    #[cfg(windows)]
    job: Arc<factory_proc::windows_job::JobGuard>,
}

impl DevelopmentTerminal {
    pub fn start(
        budget: &RuntimeBudget,
        executable: &Path,
        arguments: &[String],
        root: &Path,
        environment: &BTreeMap<String, String>,
        cols: u16,
        rows: u16,
    ) -> Result<Self, RuntimeError> {
        let permit = budget.terminal()?;
        let pair = native_pty_system()
            .openpty(terminal_size(cols, rows))
            .map_err(error)?;
        let mut command = CommandBuilder::new(factory_git::git_path(executable));
        command.args(arguments);
        command.cwd(factory_git::git_path(root));
        command.env_clear();
        for (key, value) in environment {
            command.env(key, value);
        }
        let mut child = pair.slave.spawn_command(command).map_err(error)?;
        let setup = terminal_process::TerminalSetup::new(child.as_mut()).map_err(error)?;
        let pid = child
            .process_id()
            .ok_or_else(|| error("terminal process has no PID"))?;
        #[cfg(windows)]
        let job = {
            use std::os::windows::io::BorrowedHandle;
            let handle = child
                .as_raw_handle()
                .ok_or_else(|| error("terminal has no process handle"))?;
            let borrowed = unsafe { BorrowedHandle::borrow_raw(handle) };
            Arc::new(
                factory_proc::windows_job::JobGuard::assign_borrowed(borrowed)
                    .ok_or_else(|| error("terminal process tree supervision unavailable"))?,
            )
        };
        let mut reader = pair.master.try_clone_reader().map_err(error)?;
        let writer = pair.master.take_writer().map_err(error)?;
        let output = Arc::new(Mutex::new(TerminalOutput::default()));
        let inner = Arc::new(InteractiveTerminal {
            _permit: Mutex::new(Some(permit)),
            master: Mutex::new(Some(pair.master)),
            writer: Mutex::new(Some(writer)),
            killer: Mutex::new(setup.finish()),
            stopping: AtomicBool::new(false),
            output: output.clone(),
        });
        let exited = Arc::new(Mutex::new(None));
        let exit = exited.clone();
        let weak = Arc::downgrade(&inner);
        #[cfg(windows)]
        let cleanup_job = job.clone();
        std::thread::spawn(move || {
            let code = child.wait().map(|status| status.exit_code()).unwrap_or(1);
            *exit.lock().unwrap_or_else(|p| p.into_inner()) = Some(code);
            if let Some(inner) = weak.upgrade() {
                inner.stopping.store(true, Ordering::Release);
                if let Ok(mut permit) = inner._permit.lock() {
                    permit.take();
                }
            }
            #[cfg(windows)]
            cleanup_job.terminate();
            #[cfg(unix)]
            unsafe {
                libc::kill(-(pid as i32), libc::SIGKILL);
            }
        });
        std::thread::spawn(move || {
            pump_terminal_output(0, &output, None, reader.as_mut());
            output
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .close_subscribers();
        });
        Ok(Self {
            inner,
            pid,
            exit: exited,
            #[cfg(windows)]
            job,
        })
    }

    pub fn exit_code(&self) -> Option<u32> {
        *self.exit.lock().unwrap_or_else(|p| p.into_inner())
    }
    pub fn subscribe(&self) -> Result<TerminalSubscription, RuntimeError> {
        subscribe_terminal_output(&self.inner.output)
            .map_err(|_| error("terminal subscriber limit reached"))
    }
    pub fn write(&self, bytes: &[u8]) -> Result<(), RuntimeError> {
        if bytes.len() > 65536 || self.exit_code().is_some() {
            return Err(error("terminal input unavailable or too large"));
        }
        let mut writer = self.inner.writer.try_lock().map_err(error)?;
        let writer = writer.as_mut().ok_or_else(|| error("terminal is closed"))?;
        writer
            .write_all(bytes)
            .and_then(|_| writer.flush())
            .map_err(error)
    }
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), RuntimeError> {
        self.inner
            .master
            .lock()
            .map_err(error)?
            .as_ref()
            .ok_or_else(|| error("terminal is closed"))?
            .resize(terminal_size(cols, rows))
            .map_err(error)
    }
    pub fn stop(&self) -> Result<(), RuntimeError> {
        let stopping = self.inner.stopping.swap(true, Ordering::AcqRel);
        let result = if stopping || self.exit_code().is_some() {
            Ok(())
        } else {
            self.inner
                .killer
                .lock()
                .map_err(error)?
                .kill()
                .map_err(error)
        };
        #[cfg(windows)]
        self.job.terminate();
        #[cfg(unix)]
        if !stopping {
            unsafe {
                libc::kill(-(self.pid as i32), libc::SIGKILL);
            }
        }
        result
    }
}

fn error(value: impl std::fmt::Display) -> RuntimeError {
    RuntimeError::Terminal(value.to_string())
}

impl Drop for DevelopmentTerminal {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
