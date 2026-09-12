//! World-to-world passages from original `passages.prm`.
//!
//! Authoring format (comment in the file) has more fields; the shipped
//! `data-0` file is the short form: id, from-world, to-world, x, y.

use crate::config::Settings;
use crate::config::text::Reader;

use std::fs::File;
use std::io::Read;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Passage {
    pub id: String,
    pub from_world: String,
    pub to_world: String,
    pub coordinates: (i32, i32),
}

impl Passage {
    /// Reach used when the short prm omits InRadius.
    pub const DEFAULT_REACH: i32 = 160;
}

pub fn load_optional(settings: &Settings) -> Vec<Passage> {
    if settings.check_path("passages.prm") {
        load(settings.open_relative("passages.prm"))
    } else {
        Vec::new()
    }
}

pub fn load(file: File) -> Vec<Passage> {
    load_reader(file)
}

pub fn load_reader<R: Read>(reader: R) -> Vec<Passage> {
    let mut out = Vec::new();
    let mut fi = Reader::new(reader);
    fi.advance();
    assert_eq!(fi.cur(), "uniVang-ParametersFile_Ver_1");
    while fi.advance() {
        let (id, from_world, to_world, x, y): (String, String, String, i32, i32) = fi.scan();
        out.push(Passage {
            id,
            from_world,
            to_world,
            coordinates: (x, y),
        });
    }
    out
}

/// Passages that leave `world` (case-insensitive name match).
pub fn from_world<'a>(all: &'a [Passage], world: &str) -> Vec<&'a Passage> {
    let key = world.to_ascii_lowercase();
    all.iter()
        .filter(|p| p.from_world.to_ascii_lowercase() == key)
        .collect()
}

/// Look up a passage by `passages.prm` id (case-insensitive).
pub fn by_id<'a>(list: &'a [Passage], id: &str) -> Option<&'a Passage> {
    let key = id.to_ascii_lowercase();
    list.iter().find(|p| p.id.to_ascii_lowercase() == key)
}

/// Arrival pad on `dest_world` when hopping from `from_world`: the reverse
/// passage (`from=dest`, `to=old`).
pub fn arrival_coords(all: &[Passage], dest_world: &str, from_world: &str) -> Option<(i32, i32)> {
    let dest = dest_world.to_ascii_lowercase();
    let from = from_world.to_ascii_lowercase();
    all.iter()
        .find(|p| {
            p.from_world.to_ascii_lowercase() == dest && p.to_world.to_ascii_lowercase() == from
        })
        .map(|p| p.coordinates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_fostral_rows() {
        let src = r#"uniVang-ParametersFile_Ver_1
F2G Fostral Glorx 810 4630
F2W Fostral Weexow 1040 11530
G2F Glorx Fostral 1420 7575
"#;
        let list = load_reader(src.as_bytes());
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].id, "F2G");
        assert_eq!(list[0].to_world, "Glorx");
        assert_eq!(list[0].coordinates, (810, 4630));
        let fostral = from_world(&list, "fostral");
        assert_eq!(fostral.len(), 2);
    }

    #[test]
    fn finds_passage_by_id() {
        let src = r#"uniVang-ParametersFile_Ver_1
F2G Fostral Glorx 810 4630
"#;
        let list = load_reader(src.as_bytes());
        assert_eq!(by_id(&list, "f2g").map(|p| p.to_world.as_str()), Some("Glorx"));
        assert!(by_id(&list, "nope").is_none());
    }

    #[test]
    fn resolves_reverse_arrival_for_f2g() {
        let src = r#"uniVang-ParametersFile_Ver_1
F2G Fostral Glorx 810 4630
F2W Fostral Weexow 1040 11530
G2F Glorx Fostral 1420 7575
"#;
        let list = load_reader(src.as_bytes());
        assert_eq!(
            arrival_coords(&list, "Glorx", "Fostral"),
            Some((1420, 7575))
        );
        assert_eq!(
            arrival_coords(&list, "glorx", "fostral"),
            Some((1420, 7575))
        );
        assert_eq!(arrival_coords(&list, "Weexow", "Fostral"), None);
    }
}
