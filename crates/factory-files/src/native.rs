use std::fs::{self, File, Metadata, OpenOptions};
use std::path::Path;

use cap_std::fs::Dir;

use crate::{FileError, Result};

pub(crate) fn is_link(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    false
}

pub(crate) fn open_directory(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(3).custom_flags(0x02200000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY);
    }
    let file = options.open(path)?;
    if is_link(&file.metadata()?) || !file.metadata()?.is_dir() {
        return Err(FileError::Forbidden);
    }
    Ok(file)
}

pub(crate) fn open_file(parent: &Dir, name: &str, absolute: &Path) -> Result<File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let _ = (parent, name);
        Ok(OpenOptions::new()
            .read(true)
            .share_mode(5)
            .custom_flags(0x00200000)
            .open(absolute)?)
    }
    #[cfg(not(windows))]
    {
        use cap_std::fs::OpenOptionsExt;
        let _ = absolute;
        let mut options = cap_std::fs::OpenOptions::new();
        options
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
        Ok(parent.open_with(name, &options)?.into_std())
    }
}

#[cfg(windows)]
fn information(
    file: &File,
) -> Result<windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION> {
    use std::os::windows::io::AsRawHandle;
    let mut info = std::mem::MaybeUninit::uninit();
    if unsafe {
        windows_sys::Win32::Storage::FileSystem::GetFileInformationByHandle(
            file.as_raw_handle().cast(),
            info.as_mut_ptr(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(unsafe { info.assume_init() })
}

pub(crate) fn key(file: &File) -> Result<String> {
    #[cfg(windows)]
    {
        let info = information(file)?;
        Ok(format!(
            "{}:{}:{}",
            info.dwVolumeSerialNumber, info.nFileIndexHigh, info.nFileIndexLow
        ))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = file.metadata()?;
        Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
}

pub(crate) fn metadata_key(path: &Path, metadata: &Metadata) -> Result<String> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        let _ = metadata;
        key(&OpenOptions::new()
            .read(true)
            .share_mode(7)
            .custom_flags(0x02200000)
            .open(path)?)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = path;
        Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
}

pub(crate) fn links(file: &File) -> Result<u64> {
    #[cfg(windows)]
    {
        Ok(information(file)?.nNumberOfLinks as u64)
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(file.metadata()?.nlink())
    }
}

pub(crate) fn open_metadata(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(7).custom_flags(0x02200000);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    Ok(options.open(path)?)
}

pub(crate) fn change_stamp(metadata: &Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", metadata.ctime(), metadata.ctime_nsec())
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        format!(
            "{:?}:{:?}:{}",
            metadata.created().ok(),
            metadata.modified().ok(),
            metadata.file_attributes()
        )
    }
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}

pub(crate) fn exchange(
    parent: &Dir,
    absolute: &Path,
    temporary: &str,
    target: &str,
    backup: &Path,
) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = parent;
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::ReplaceFileW(
                wide(&absolute.join(target)).as_ptr(),
                wide(&absolute.join(temporary)).as_ptr(),
                wide(backup).as_ptr(),
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        } == 0
        {
            return Err(FileError::Uncertain);
        }
        Ok(())
    }
    #[cfg(unix)]
    {
        let _ = (absolute, backup);
        rename_flags(parent, temporary, parent, target, true)
    }
}

pub(crate) fn no_replace(
    from: &Dir,
    from_absolute: &Path,
    source: &str,
    to: &Dir,
    to_absolute: &Path,
    target: &str,
) -> Result<()> {
    #[cfg(windows)]
    {
        let _ = (from, to);
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                wide(&from_absolute.join(source)).as_ptr(),
                wide(&to_absolute.join(target)).as_ptr(),
                windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(unix)]
    {
        let _ = (from_absolute, to_absolute);
        rename_flags(from, source, to, target, false)
    }
}

pub(crate) fn restore(
    target: &crate::path::Guard,
    name: &str,
    backup: &crate::path::Guard,
    backup_name: &str,
    retained: &str,
) -> Result<()> {
    #[cfg(windows)]
    {
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::ReplaceFileW(
                wide(&target.absolute.join(name)).as_ptr(),
                wide(&backup.absolute.join(backup_name)).as_ptr(),
                wide(&backup.absolute.join(retained)).as_ptr(),
                0,
                std::ptr::null(),
                std::ptr::null(),
            )
        } == 0
        {
            return Err(FileError::Uncertain);
        }
        Ok(())
    }
    #[cfg(unix)]
    {
        let _ = retained;
        rename_flags(&backup.dir, backup_name, &target.dir, name, true)
    }
}

#[cfg(unix)]
fn rename_flags(from: &Dir, source: &str, to: &Dir, target: &str, exchange: bool) -> Result<()> {
    use std::os::fd::AsRawFd;
    let source = std::ffi::CString::new(source).map_err(|_| FileError::Invalid)?;
    let target = std::ffi::CString::new(target).map_err(|_| FileError::Invalid)?;
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            from.as_raw_fd(),
            source.as_ptr(),
            to.as_raw_fd(),
            target.as_ptr(),
            if exchange {
                libc::RENAME_EXCHANGE
            } else {
                libc::RENAME_NOREPLACE
            },
        )
    };
    #[cfg(target_os = "macos")]
    let result = unsafe {
        libc::renameatx_np(
            from.as_raw_fd(),
            source.as_ptr(),
            to.as_raw_fd(),
            target.as_ptr(),
            if exchange {
                libc::RENAME_SWAP
            } else {
                libc::RENAME_EXCL
            },
        )
    };
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let result = -1;
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

pub(crate) fn sync_directory(dir: &Dir) -> Result<&'static str> {
    #[cfg(unix)]
    {
        dir.open(".")
            .and_then(|directory| directory.sync_all())
            .map_err(|_| FileError::Uncertain)?;
        Ok("file_and_directory_synced")
    }
    #[cfg(windows)]
    {
        let _ = dir;
        Ok("file_synced_directory_flush_not_guaranteed")
    }
}

pub(crate) fn preserve_permissions(original: &File, replacement: &File) -> Result<()> {
    let metadata = original.metadata()?;
    if metadata.permissions().readonly() {
        return Err(FileError::Permission);
    }
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        use std::os::unix::fs::MetadataExt;
        if metadata.mode() & 0o7000 != 0 {
            return Err(FileError::Permission);
        }
        let current = replacement.metadata()?;
        if (metadata.uid(), metadata.gid()) != (current.uid(), current.gid())
            && unsafe { libc::fchown(replacement.as_raw_fd(), metadata.uid(), metadata.gid()) } != 0
        {
            return Err(FileError::Permission);
        }
        replacement.set_permissions(metadata.permissions())?;
    }
    #[cfg(windows)]
    {
        let _ = replacement;
    }
    Ok(())
}

pub(crate) fn remove_owned(path: &Path) {
    let _ = fs::remove_file(path);
}
