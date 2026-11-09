//! An application's key-value storage.
//!
//! One namespace per application, shared by its backend and its tool calls —
//! the only state a tool call can leave behind, since its instance is gone
//! when it returns. The quota (the manifest's `storage_mb`) is enforced by
//! the store itself, inside the same lock as the write, so two concurrent
//! writes cannot both fit into space only one of them has.
//!
//! Size is keys plus values. Counting only values would make a store of a
//! million empty values free.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::sync::lock;

/// The longest key, in bytes. Bounded so that [`FileKv`] can use the key
/// itself (hex-encoded) as a file name on every common filesystem.
pub const MAX_KEY_BYTES: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KvError {
    /// The write would take the namespace past its quota.
    QuotaExceeded {
        quota: u64,
        needed: u64,
    },
    InvalidKey(String),
    Io(String),
}

impl std::fmt::Display for KvError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::QuotaExceeded { quota, needed } => write!(
                f,
                "storage quota exceeded: this write needs {needed} bytes in total and the \
                 application's `storage_mb` allows {quota}"
            ),
            Self::InvalidKey(why) => write!(f, "invalid key: {why}"),
            Self::Io(e) => write!(f, "storage error: {e}"),
        }
    }
}

impl std::error::Error for KvError {}

/// One application's storage.
pub trait KvStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, KvError>;
    /// Write `value`, or delete the key when it is `None`. Refused when the
    /// namespace would grow past `quota` bytes; a write that does not grow
    /// it (an overwrite with something no larger, a delete) always succeeds,
    /// so an application over a lowered quota can still make room.
    fn put(&self, key: &str, value: Option<&[u8]>, quota: u64) -> Result<(), KvError>;
    /// Bytes in use, keys and values.
    fn used_bytes(&self) -> u64;
    /// Drop everything: what uninstalling means.
    fn clear(&self) -> Result<(), KvError>;
}

fn check_key(key: &str) -> Result<(), KvError> {
    if key.is_empty() {
        return Err(KvError::InvalidKey("empty".into()));
    }
    if key.len() > MAX_KEY_BYTES {
        return Err(KvError::InvalidKey(format!(
            "{} bytes, more than {MAX_KEY_BYTES}",
            key.len()
        )));
    }
    Ok(())
}

fn entry_size(key: &str, value_len: usize) -> u64 {
    (key.len() + value_len) as u64
}

/// The size after a write, or the refusal. `old` is the existing entry's size.
fn admit(used: u64, old: Option<u64>, new: Option<u64>, quota: u64) -> Result<u64, KvError> {
    let after = used - old.unwrap_or(0) + new.unwrap_or(0);
    if after > used && after > quota {
        return Err(KvError::QuotaExceeded {
            quota,
            needed: after,
        });
    }
    Ok(after)
}

/// Storage that lives as long as the process: for tests, and for a host
/// that has nowhere to put it.
#[derive(Default)]
pub struct MemoryKv {
    inner: Mutex<(HashMap<String, Vec<u8>>, u64)>,
}

impl MemoryKv {
    pub fn new() -> Self {
        Self::default()
    }
}

impl KvStore for MemoryKv {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, KvError> {
        check_key(key)?;
        Ok(lock(&self.inner).0.get(key).cloned())
    }

    fn put(&self, key: &str, value: Option<&[u8]>, quota: u64) -> Result<(), KvError> {
        check_key(key)?;
        let mut guard = lock(&self.inner);
        let (map, used) = &mut *guard;
        let old = map.get(key).map(|v| entry_size(key, v.len()));
        let new = value.map(|v| entry_size(key, v.len()));
        *used = admit(*used, old, new, quota)?;
        match value {
            Some(v) => map.insert(key.to_string(), v.to_vec()),
            None => map.remove(key),
        };
        Ok(())
    }

    fn used_bytes(&self) -> u64 {
        lock(&self.inner).1
    }

    fn clear(&self) -> Result<(), KvError> {
        *lock(&self.inner) = Default::default();
        Ok(())
    }
}

/// Storage in a directory: one file per key, named by the key in hex, written
/// to a temporary file and renamed into place so a crash leaves either the old
/// value or the new one.
///
/// The directory must be the application's alone and outside anything its
/// `fs` capability preopens — otherwise the component could rewrite it
/// directly and walk straight past the quota.
pub struct FileKv {
    dir: PathBuf,
    /// Size of every entry, so a write knows what it replaces without a
    /// `stat`, plus the total.
    sizes: Mutex<(HashMap<String, u64>, u64)>,
}

impl FileKv {
    /// Open (creating it if needed) the store in `dir`, counting what is
    /// already there. Leftover temporary files from an interrupted write are
    /// removed.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, KvError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|e| io(&dir, e))?;
        let mut sizes = HashMap::new();
        let mut used = 0;
        for entry in std::fs::read_dir(&dir).map_err(|e| io(&dir, e))? {
            let entry = entry.map_err(|e| io(&dir, e))?;
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".tmp") {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let Some(key) = hex::decode(&name)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
            else {
                continue;
            };
            let len = entry.metadata().map_err(|e| io(&path, e))?.len();
            let size = entry_size(&key, len as usize);
            used += size;
            sizes.insert(key, size);
        }
        Ok(Self {
            dir,
            sizes: Mutex::new((sizes, used)),
        })
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(hex::encode(key))
    }
}

fn io(path: &Path, e: std::io::Error) -> KvError {
    KvError::Io(format!("{}: {e}", path.display()))
}

impl KvStore for FileKv {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>, KvError> {
        check_key(key)?;
        let path = self.path(key);
        match std::fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io(&path, e)),
        }
    }

    fn put(&self, key: &str, value: Option<&[u8]>, quota: u64) -> Result<(), KvError> {
        check_key(key)?;
        let mut guard = lock(&self.sizes);
        let (sizes, used) = &mut *guard;
        let old = sizes.get(key).copied();
        let new = value.map(|v| entry_size(key, v.len()));
        let after = admit(*used, old, new, quota)?;
        let path = self.path(key);
        match value {
            Some(v) => {
                let tmp = path.with_extension("tmp");
                std::fs::write(&tmp, v).map_err(|e| io(&tmp, e))?;
                std::fs::rename(&tmp, &path).map_err(|e| io(&path, e))?;
                sizes.insert(key.to_string(), new.unwrap_or(0));
            }
            None => {
                match std::fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io(&path, e)),
                }
                sizes.remove(key);
            }
        }
        *used = after;
        Ok(())
    }

    fn used_bytes(&self) -> u64 {
        lock(&self.sizes).1
    }

    fn clear(&self) -> Result<(), KvError> {
        let mut guard = lock(&self.sizes);
        for key in guard.0.keys() {
            let path = self.path(key);
            match std::fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io(&path, e)),
            }
        }
        *guard = Default::default();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exercise(kv: &dyn KvStore) {
        assert_eq!(kv.get("a").unwrap(), None);
        kv.put("a", Some(b"12345"), 100).unwrap();
        assert_eq!(kv.get("a").unwrap().as_deref(), Some(&b"12345"[..]));
        assert_eq!(kv.used_bytes(), 6, "key and value both count");

        // 6 + (1 + 94) = 101 > 100.
        let err = kv.put("b", Some(&[0; 94]), 100).unwrap_err();
        assert_eq!(
            err,
            KvError::QuotaExceeded {
                quota: 100,
                needed: 101
            }
        );
        assert_eq!(kv.get("b").unwrap(), None, "a refused write leaves nothing");
        kv.put("b", Some(&[0; 93]), 100).unwrap();
        assert_eq!(kv.used_bytes(), 100);

        // Over a lowered quota, a shrinking write and a delete still work.
        kv.put("b", Some(&[0; 10]), 50).unwrap();
        kv.put("b", None, 0).unwrap();
        assert_eq!(kv.used_bytes(), 6);

        assert!(matches!(
            kv.put("", Some(b"x"), 100),
            Err(KvError::InvalidKey(_))
        ));
        let long = "k".repeat(MAX_KEY_BYTES + 1);
        assert!(matches!(kv.get(&long), Err(KvError::InvalidKey(_))));

        kv.clear().unwrap();
        assert_eq!(kv.used_bytes(), 0);
        assert_eq!(kv.get("a").unwrap(), None);
    }

    #[test]
    fn memory_storage_enforces_its_quota() {
        exercise(&MemoryKv::new());
    }

    #[test]
    fn file_storage_enforces_its_quota() {
        let dir = tempfile::tempdir().unwrap();
        exercise(&FileKv::open(dir.path().join("kv")).unwrap());
    }

    #[test]
    fn file_storage_survives_reopening_and_recounts_what_it_holds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kv");
        let key = "k".repeat(MAX_KEY_BYTES);
        {
            let kv = FileKv::open(&path).unwrap();
            kv.put(&key, Some(b"value"), 1000).unwrap();
            kv.put("other", Some(b"x"), 1000).unwrap();
        }
        // An interrupted write leaves a temporary file behind.
        std::fs::write(path.join("deadbeef.tmp"), b"half").unwrap();

        let kv = FileKv::open(&path).unwrap();
        assert_eq!(kv.get(&key).unwrap().as_deref(), Some(&b"value"[..]));
        assert_eq!(kv.used_bytes(), (MAX_KEY_BYTES + 5 + 5 + 1) as u64);
        assert!(!path.join("deadbeef.tmp").exists());
    }
}
