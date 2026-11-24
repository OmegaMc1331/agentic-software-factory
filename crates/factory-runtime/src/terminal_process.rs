use std::io;

pub(crate) struct TerminalKiller {
    #[cfg(windows)]
    handle: std::os::windows::io::OwnedHandle,
    #[cfg(not(windows))]
    inner: Box<dyn portable_pty::ChildKiller + Send + Sync>,
}

impl TerminalKiller {
    pub(crate) fn new(child: &dyn portable_pty::Child) -> io::Result<Self> {
        #[cfg(windows)]
        {
            use std::os::windows::io::BorrowedHandle;
            let raw = child
                .as_raw_handle()
                .ok_or_else(|| io::Error::other("terminal child has no process handle"))?;
            let handle = unsafe { BorrowedHandle::borrow_raw(raw) }.try_clone_to_owned()?;
            Ok(Self { handle })
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                inner: child.clone_killer(),
            })
        }
    }

    pub(crate) fn kill(&mut self) -> io::Result<()> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
            use windows_sys::Win32::System::Threading::{TerminateProcess, WaitForSingleObject};
            let handle = self.handle.as_raw_handle();
            if unsafe { TerminateProcess(handle, 1) } != 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if unsafe { WaitForSingleObject(handle, 0) } == WAIT_OBJECT_0 {
                return Ok(());
            }
            Err(error)
        }
        #[cfg(not(windows))]
        {
            self.inner.kill()
        }
    }
}

pub(crate) struct TerminalSetup {
    killer: Option<TerminalKiller>,
}

impl TerminalSetup {
    pub(crate) fn new(child: &mut dyn portable_pty::Child) -> io::Result<Self> {
        match TerminalKiller::new(child) {
            Ok(killer) => Ok(Self {
                killer: Some(killer),
            }),
            Err(error) => {
                let _ = child.kill();
                Err(error)
            }
        }
    }

    pub(crate) fn finish(mut self) -> TerminalKiller {
        self.killer.take().expect("terminal setup already consumed")
    }
}

impl Drop for TerminalSetup {
    fn drop(&mut self) {
        if let Some(killer) = self.killer.as_mut() {
            let _ = killer.kill();
        }
    }
}
