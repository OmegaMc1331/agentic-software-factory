use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileRef {
    pub project_id: String,
    pub root_id: String,
    pub path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RootInfo {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub read_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub file: FileRef,
    pub revision: String,
    pub content: Option<String>,
    pub size: u64,
    pub modified: String,
    pub encoding: String,
    pub bom: bool,
    pub newline: String,
    pub read_only: bool,
    pub reason: Option<String>,
    pub mode: String,
    pub durability: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub size: u64,
    pub hidden: bool,
    pub ignored: bool,
    pub protected: bool,
    pub revision: String,
    pub git: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct DirectoryPage {
    pub entries: Vec<Entry>,
    pub next: Option<String>,
    pub complete: bool,
    pub reason: Option<String>,
    pub git_available: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub schema: u32,
    pub id: String,
    pub file: FileRef,
    pub base_revision: String,
    pub base_content: String,
    pub content: String,
    pub newline: String,
    pub bom: bool,
    pub dirty: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DraftEnvelope {
    pub revision: u64,
    pub document: Draft,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Recovery {
    pub id: String,
    pub root_id: String,
    pub path: String,
    pub kind: String,
    pub phase: String,
    pub temporary: String,
    pub backup: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct MoveResult {
    pub from: FileRef,
    pub to: FileRef,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Limits {
    pub edit_bytes: usize,
    pub preview_bytes: usize,
    pub directory_entries: usize,
    pub scan_entries: usize,
    pub search_bytes: usize,
    pub search_results: usize,
    pub search_millis: u64,
    pub draft_bytes: usize,
    pub draft_records: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            edit_bytes: 1_048_576,
            preview_bytes: 65_536,
            directory_entries: 200,
            scan_entries: 50_000,
            search_bytes: 16_777_216,
            search_results: 100,
            search_millis: 2_000,
            draft_bytes: 16_777_216,
            draft_records: 128,
        }
    }
}

impl Limits {
    pub fn from_environment() -> crate::Result<Self> {
        let mut limits = Self::default();
        for (name, value) in [
            ("FACTORY_FILE_EDIT_BYTES", &mut limits.edit_bytes),
            ("FACTORY_FILE_PREVIEW_BYTES", &mut limits.preview_bytes),
            (
                "FACTORY_FILE_DIRECTORY_ENTRIES",
                &mut limits.directory_entries,
            ),
            ("FACTORY_FILE_SCAN_ENTRIES", &mut limits.scan_entries),
            ("FACTORY_FILE_SEARCH_BYTES", &mut limits.search_bytes),
            ("FACTORY_FILE_SEARCH_RESULTS", &mut limits.search_results),
            ("FACTORY_EDITOR_DRAFT_BYTES", &mut limits.draft_bytes),
            ("FACTORY_EDITOR_DRAFT_RECORDS", &mut limits.draft_records),
        ] {
            if let Ok(configured) = std::env::var(name) {
                *value = configured.parse().map_err(|_| crate::FileError::Invalid)?;
            }
        }
        if let Ok(configured) = std::env::var("FACTORY_FILE_SEARCH_MILLIS") {
            limits.search_millis = configured.parse().map_err(|_| crate::FileError::Invalid)?;
        }
        limits.validate()?;
        Ok(limits)
    }
    pub fn validate(&self) -> crate::Result<()> {
        if !(1..=1_048_576).contains(&self.edit_bytes)
            || !(1..=65_536).contains(&self.preview_bytes)
            || !(1..=500).contains(&self.directory_entries)
            || !(1..=50_000).contains(&self.scan_entries)
            || !(1..=33_554_432).contains(&self.search_bytes)
            || !(1..=200).contains(&self.search_results)
            || !(1..=5_000).contains(&self.search_millis)
            || !(1..=33_554_432).contains(&self.draft_bytes)
            || !(1..=256).contains(&self.draft_records)
        {
            return Err(crate::FileError::Invalid);
        }
        Ok(())
    }
}

pub(crate) fn hash(bytes: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(bytes.as_ref()))
}

pub(crate) fn nonce() -> String {
    rand::random::<[u8; 16]>()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn valid_token(value: &str, prefix: &str) -> bool {
    value.strip_prefix(prefix).is_some_and(|tail| {
        tail.len() == 32
            && tail
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}
