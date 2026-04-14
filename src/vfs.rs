//! In-memory virtual filesystem populated from zip archives.
//!
//! Paths are case-folded and slash-normalized at mount time so that
//! INI references like `Palette File = HARMONY.PAL` work regardless of
//! the casing used inside the zip.

use std::collections::HashMap;
use std::sync::Arc;

#[derive(Default, Clone)]
pub struct Vfs {
    entries: HashMap<String, Arc<[u8]>>,
}

#[derive(Debug)]
pub enum VfsError {
    Zip(zip::result::ZipError),
    Io(std::io::Error),
}

impl From<zip::result::ZipError> for VfsError {
    fn from(e: zip::result::ZipError) -> Self {
        VfsError::Zip(e)
    }
}
impl From<std::io::Error> for VfsError {
    fn from(e: std::io::Error) -> Self {
        VfsError::Io(e)
    }
}

impl std::fmt::Display for VfsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            VfsError::Zip(ref e) => write!(f, "zip error: {}", e),
            VfsError::Io(ref e) => write!(f, "io error: {}", e),
        }
    }
}
impl std::error::Error for VfsError {}

impl Vfs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mount a zip archive from its raw bytes. Later mounts override
    /// earlier entries with the same normalized path.
    pub fn mount_zip(&mut self, bytes: &[u8]) -> Result<(), VfsError> {
        use std::io::Read as _;
        let reader = std::io::Cursor::new(bytes);
        let mut zip = zip::ZipArchive::new(reader)?;
        for i in 0..zip.len() {
            let mut file = zip.by_index(i)?;
            if file.is_dir() {
                continue;
            }
            let key = normalize(file.name());
            let mut buf = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut buf)?;
            self.entries.insert(key, Arc::from(buf));
        }
        Ok(())
    }

    /// Insert raw bytes at a path directly. Useful for tests or for
    /// synthesizing small files without a zip wrapper.
    pub fn insert(&mut self, path: &str, bytes: impl Into<Arc<[u8]>>) {
        self.entries.insert(normalize(path), bytes.into());
    }

    pub fn read(&self, path: &str) -> Option<Arc<[u8]>> {
        self.entries.get(&normalize(path)).cloned()
    }

    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(&normalize(path))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
}

fn normalize(p: &str) -> String {
    p.trim_start_matches("./")
        .trim_start_matches('/')
        .replace('\\', "/")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_handles_common_cases() {
        assert_eq!(normalize("Foo/BAR.txt"), "foo/bar.txt");
        assert_eq!(normalize("./foo/bar"), "foo/bar");
        assert_eq!(normalize("/foo/bar"), "foo/bar");
        assert_eq!(normalize("foo\\bar"), "foo/bar");
    }

    #[test]
    fn insert_and_read_roundtrip() {
        let mut vfs = Vfs::new();
        vfs.insert("Foo/Bar.txt", b"hello".to_vec());
        assert_eq!(vfs.read("foo/bar.txt").unwrap().as_ref(), b"hello");
        assert!(vfs.contains("FOO/BAR.TXT"));
    }
}
