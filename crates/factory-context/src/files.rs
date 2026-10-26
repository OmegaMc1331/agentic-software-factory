use std::fs::{File, Metadata, OpenOptions};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use crate::ignore::IgnoreRules;
use crate::index::{forward_slash, MAX_INDEX_FILE_READ_BYTES};

pub(crate) fn relative_file<'a>(root: &Path, path: &'a Path) -> Option<&'a Path> {
    if !root.is_absolute() || !path.is_absolute() {
        return None;
    }
    if path
        .as_os_str()
        .as_encoded_bytes()
        .split(is_separator)
        .any(|part| part == b"." || part == b"..")
    {
        return None;
    }
    let relative = path.strip_prefix(root).ok()?;
    if relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        || relative.to_str().is_none()
    {
        return None;
    }
    #[cfg(windows)]
    if relative.as_os_str().as_encoded_bytes().contains(&b':') {
        return None;
    }
    Some(relative)
}

fn is_separator(byte: &u8) -> bool {
    *byte == b'/' || (cfg!(windows) && *byte == b'\\')
}

pub(crate) fn file_key(root: &Path, path: &Path) -> Option<String> {
    relative_file(root, path)?;
    let canonical = path.canonicalize().ok()?;
    Some(forward_slash(
        &relative_file(root, &canonical)?
            .components()
            .collect::<PathBuf>(),
    ))
}

pub(crate) fn is_link(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return true;
        }
    }
    false
}

pub(crate) fn repository_metadata(
    root: &Path,
    ignore: &IgnoreRules,
    path: &Path,
    directory: bool,
) -> Option<Metadata> {
    let root_metadata = std::fs::symlink_metadata(root).ok()?;
    if is_link(&root_metadata) || !root_metadata.is_dir() {
        return None;
    }
    if directory && path == root {
        return (root.canonicalize().ok()? == root).then_some(root_metadata);
    }
    let relative = relative_file(root, path)?;
    if ignore.is_ignored(relative, directory) {
        return None;
    }
    let mut current = root.to_path_buf();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        current.push(component);
        let metadata = std::fs::symlink_metadata(&current).ok()?;
        if is_link(&metadata) {
            return None;
        }
        if components.peek().is_some() {
            if !metadata.is_dir() {
                return None;
            }
        } else {
            if (directory && !metadata.is_dir()) || (!directory && !metadata.is_file()) {
                return None;
            }
            let canonical = path.canonicalize().ok()?;
            let canonical_relative = relative_file(root, &canonical)?;
            if ignore.is_ignored(canonical_relative, directory) {
                return None;
            }
            return Some(metadata);
        }
    }
    None
}

pub(crate) fn open_repository_file(root: &Path, ignore: &IgnoreRules, path: &Path) -> Option<File> {
    let before = repository_metadata(root, ignore, path, false)?;
    if before.len() > MAX_INDEX_FILE_READ_BYTES {
        return None;
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || is_link(&metadata) || metadata.len() > MAX_INDEX_FILE_READ_BYTES {
        return None;
    }
    let after = repository_metadata(root, ignore, path, false)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if [before, after]
            .iter()
            .any(|stat| stat.dev() != metadata.dev() || stat.ino() != metadata.ino())
        {
            return None;
        }
    }
    #[cfg(not(unix))]
    let _ = after;
    Some(file)
}

pub(crate) fn read_bounded(reader: impl Read, reported_size: u64) -> Option<Vec<u8>> {
    if reported_size > MAX_INDEX_FILE_READ_BYTES {
        return None;
    }
    let mut buffer = Vec::with_capacity(
        reported_size
            .saturating_add(1)
            .min(MAX_INDEX_FILE_READ_BYTES + 1) as usize,
    );
    reader
        .take(MAX_INDEX_FILE_READ_BYTES + 1)
        .read_to_end(&mut buffer)
        .ok()?;
    (buffer.len() as u64 <= MAX_INDEX_FILE_READ_BYTES).then_some(buffer)
}

pub(crate) fn read_repository_file(
    root: &Path,
    ignore: &IgnoreRules,
    path: &Path,
) -> Option<Vec<u8>> {
    let file = open_repository_file(root, ignore, path)?;
    let size = file.metadata().ok()?.len();
    read_bounded(file, size)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor, Write};
    use tempfile::TempDir;

    #[test]
    fn v02_empty_below_exact_and_above_the_read_limit() {
        for size in [
            0,
            17,
            MAX_INDEX_FILE_READ_BYTES - 1,
            MAX_INDEX_FILE_READ_BYTES,
            MAX_INDEX_FILE_READ_BYTES + 1,
        ] {
            let bytes = read_bounded(io::repeat(b'x').take(size), size);
            if size <= MAX_INDEX_FILE_READ_BYTES {
                assert_eq!(bytes.unwrap().len() as u64, size);
            } else {
                assert!(bytes.is_none());
            }
        }
    }

    #[test]
    fn v02_extra_byte_is_never_accepted_even_with_stale_metadata() {
        for reported in [0, 10, MAX_INDEX_FILE_READ_BYTES] {
            let reader = io::repeat(b'.')
                .take(MAX_INDEX_FILE_READ_BYTES)
                .chain(Cursor::new(b"z"));
            assert!(read_bounded(reader, reported).is_none());
        }
    }

    struct ChangingFile {
        reader: File,
        writer: File,
        consumed: usize,
        new_size: Option<u64>,
    }

    impl Read for ChangingFile {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.consumed > 0 {
                if let Some(size) = self.new_size.take() {
                    self.writer.set_len(size)?;
                }
            }
            let length = buffer.len().min(4096);
            let count = self.reader.read(&mut buffer[..length])?;
            self.consumed += count;
            Ok(count)
        }
    }

    #[test]
    fn v02_real_file_growth_and_shrink_during_reading() {
        let fixture = TempDir::new().unwrap();
        let path = fixture.path().join("changing.txt");
        for (before, after) in [
            (MAX_INDEX_FILE_READ_BYTES, MAX_INDEX_FILE_READ_BYTES + 1),
            (MAX_INDEX_FILE_READ_BYTES - 1, MAX_INDEX_FILE_READ_BYTES),
            (MAX_INDEX_FILE_READ_BYTES, 8192),
        ] {
            let mut writer = File::create(&path).unwrap();
            writer.write_all(&vec![b'x'; before as usize]).unwrap();
            let mut reader = ChangingFile {
                reader: File::open(&path).unwrap(),
                writer,
                consumed: 0,
                new_size: Some(after),
            };
            let result = read_bounded(&mut reader, before);
            if after > MAX_INDEX_FILE_READ_BYTES {
                assert!(result.is_none());
            } else {
                assert_eq!(result.unwrap().len() as u64, after);
            }
            assert_eq!(reader.reader.metadata().unwrap().len(), after);
        }
    }

    #[test]
    fn v02_reads_stop_at_limit_plus_one_even_for_an_endless_reader() {
        let mut reader = io::repeat(b'x').take(u64::MAX);
        assert!(read_bounded(&mut reader, 0).is_none());
        assert_eq!(reader.limit(), u64::MAX - MAX_INDEX_FILE_READ_BYTES - 1);
    }

    struct Unreadable;

    impl Read for Unreadable {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "synthetic read failure",
            ))
        }
    }

    #[test]
    fn v02_failure_during_reading_does_not_return_a_partial_match() {
        let reader = Cursor::new(b"valid match prefix").chain(Unreadable);
        assert!(read_bounded(reader, 100).is_none());
    }

    #[test]
    fn v02_unicode_and_a_match_at_the_last_valid_byte_are_preserved() {
        let tail = "\ncafé 日本語 MATCH";
        let reader = io::repeat(b'.')
            .take(MAX_INDEX_FILE_READ_BYTES - tail.len() as u64)
            .chain(Cursor::new(tail.as_bytes()));
        let bytes = read_bounded(reader, MAX_INDEX_FILE_READ_BYTES).unwrap();
        assert_eq!(bytes.len() as u64, MAX_INDEX_FILE_READ_BYTES);
        assert!(std::str::from_utf8(&bytes).unwrap().ends_with(tail));
    }

    #[test]
    fn v01_open_boundary_rejects_a_previously_valid_replaced_file() {
        let fixture = TempDir::new().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let path = root.join("ordinary.rs");
        std::fs::write(&path, "initial").unwrap();
        assert!(repository_metadata(&root, &IgnoreRules::empty(), &path, false).is_some());
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        assert!(read_repository_file(&root, &IgnoreRules::empty(), &path).is_none());
        std::fs::remove_dir(&path).unwrap();
        assert!(read_repository_file(&root, &IgnoreRules::empty(), &path).is_none());
    }

    #[test]
    fn v01_traversal_and_dot_components_are_rejected_lexically() {
        let fixture = TempDir::new().unwrap();
        let root = fixture.path().canonicalize().unwrap();
        let raw = |suffix: &str| {
            let mut path = root.as_os_str().to_os_string();
            path.push(std::path::MAIN_SEPARATOR_STR);
            path.push(if cfg!(windows) {
                suffix.replace('/', "\\")
            } else {
                suffix.to_string()
            });
            PathBuf::from(path)
        };
        for path in [
            raw("../private.txt"),
            raw("a/../../private.txt"),
            raw("./ordinary.rs"),
            raw("src/./ordinary.rs"),
            PathBuf::from("src/ordinary.rs"),
            root.clone(),
        ] {
            assert!(relative_file(&root, &path).is_none(), "{}", path.display());
        }
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/ordinary.rs"), "ordinary").unwrap();
        assert_eq!(
            file_key(&root, &root.join("src/ordinary.rs")).as_deref(),
            Some("src/ordinary.rs")
        );
        if let Some(key) = file_key(&root, &raw("src//ordinary.rs")) {
            assert_eq!(key, "src/ordinary.rs");
        }
    }
}
