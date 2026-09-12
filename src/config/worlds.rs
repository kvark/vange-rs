use crate::config::Settings;
use crate::config::text::Reader;

use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

pub type Worlds = HashMap<String, String>;

pub fn load(file: File) -> Worlds {
    let mut fi = Reader::new(file);
    let count = fi.next_value::<usize>();
    (0..count)
        .map(|_| {
            fi.advance();
            fi.scan()
        })
        .collect()
}

/// `wrlds.dat` from a full install, or Fostral from the open-source tree.
pub fn load_from_settings(settings: &Settings) -> Worlds {
    load_from_path(&settings.data_path)
}

/// Same as [`load_from_settings`], keyed off a data root.
pub fn load_from_path(data_path: &Path) -> Worlds {
    try_load_from_path(data_path).unwrap_or_else(|| {
        panic!(
            "Can't find wrlds.dat or Fostral world data at {:?}",
            data_path
        )
    })
}

/// Like [`load_from_path`], but returns `None` when game data is missing
/// (e.g. CI without a Vangers tree). Prefer this in the multiplayer server.
pub fn try_load_from_path(data_path: &Path) -> Option<Worlds> {
    let wrlds = data_path.join("wrlds.dat");
    if wrlds.is_file() {
        return Some(load(File::open(wrlds).ok()?));
    }
    const FOSTRAL: &str = "thechain/fostral/world.ini";
    if data_path.join(FOSTRAL).is_file() {
        let mut worlds = HashMap::new();
        worlds.insert("Fostral".to_string(), FOSTRAL.to_string());
        return Some(worlds);
    }
    None
}

/// Canonical world key from `wrlds.dat` / Fostral fallback (preserves shipped casing).
pub fn canonical_name<'a>(worlds: &'a Worlds, name: &str) -> Option<&'a str> {
    let key = name.to_ascii_lowercase();
    worlds
        .iter()
        .find(|entry| entry.0.to_ascii_lowercase() == key)
        .map(|entry| entry.0.as_str())
}

/// Case-insensitive lookup of the relative `world.ini` path in `wrlds.dat`.
///
/// Shipped names disagree on spelling (`Glorx` in passages vs `GLORX` in
/// `wrlds.dat`), so callers must not rely on exact case.
pub fn resolve<'a>(worlds: &'a Worlds, name: &str) -> Option<&'a str> {
    let key = name.to_ascii_lowercase();
    worlds
        .iter()
        .find(|entry| entry.0.to_ascii_lowercase() == key)
        .map(|entry| entry.1.as_str())
}

/// Absolute `world.ini` if `name` resolves and the file exists.
pub fn ini_path_if_present(data_path: &Path, worlds: &Worlds, name: &str) -> Option<PathBuf> {
    let rel = resolve(worlds, name)?;
    let path = data_path.join(rel);
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_is_case_insensitive() {
        let mut worlds = Worlds::new();
        worlds.insert("GLORX".into(), "thechain/glorx/world.ini".into());
        worlds.insert("Fostral".into(), "thechain/fostral/world.ini".into());
        assert_eq!(resolve(&worlds, "Glorx"), Some("thechain/glorx/world.ini"));
        assert_eq!(
            resolve(&worlds, "fostral"),
            Some("thechain/fostral/world.ini")
        );
        assert!(resolve(&worlds, "Weexow").is_none());
    }

    #[test]
    fn ini_path_requires_file_on_disk() {
        let mut worlds = Worlds::new();
        worlds.insert("Weexow".into(), "thechain/weexow/world.ini".into());
        worlds.insert("GLORX".into(), "thechain/glorx/world.ini".into());
        let data = Path::new("/workspace/vange-data");
        if !data.join("wrlds.dat").is_file() {
            return;
        }
        assert!(ini_path_if_present(data, &worlds, "Weexow").is_none());
        assert!(ini_path_if_present(data, &worlds, "Glorx").is_some());
    }

    #[test]
    fn canonical_name_preserves_shipped_casing() {
        let mut worlds = Worlds::new();
        worlds.insert("Fostral".into(), "thechain/fostral/world.ini".into());
        worlds.insert("GLORX".into(), "thechain/glorx/world.ini".into());
        assert_eq!(canonical_name(&worlds, "fostral"), Some("Fostral"));
        assert_eq!(canonical_name(&worlds, "Glorx"), Some("GLORX"));
        assert!(canonical_name(&worlds, "Weexow").is_none());
    }

    #[test]
    fn try_load_from_path_none_without_data() {
        assert!(try_load_from_path(Path::new("/no/such/vangers/data")).is_none());
    }
}
