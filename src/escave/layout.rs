//! Mechos inventory boards from original `actint/actint.inc`.
//!
//! The 1998 game compiled `invMatrix` / `MATRIX*_BODY` into the actint
//! script. Those macros are the data; we parse them at load.

use std::collections::HashMap;
use std::path::Path;

/// Bays we can hang on an m3d (the original hardpoint count).
const BAYS: usize = 3;

/// Open rectangle used by shop unit tests that are not about a mechos.
#[cfg(test)]
const PACK_WIDTH: i32 = 8;
#[cfg(test)]
const PACK_HEIGHT: i32 = 6;

/// One cell of a mechos board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Empty,
    Cargo,
    Bay(usize),
}

/// Occupancy and hardpoints of one mechos type.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub width: i32,
    pub height: i32,
    cells: Vec<Cell>,
}

impl Layout {
    pub(crate) fn empty() -> Self {
        Layout {
            width: 0,
            height: 0,
            cells: Vec::new(),
        }
    }

    #[cfg(test)]
    pub fn pack() -> Self {
        Layout {
            width: PACK_WIDTH,
            height: PACK_HEIGHT,
            cells: vec![Cell::Cargo; (PACK_WIDTH * PACK_HEIGHT) as usize],
        }
    }

    pub fn cell(&self, x: i32, y: i32) -> Cell {
        if x < 0 || y < 0 || x >= self.width || y >= self.height {
            return Cell::Empty;
        }
        self.cells[(y * self.width + x) as usize]
    }

    pub fn is_cargo(&self, x: i32, y: i32) -> bool {
        matches!(self.cell(x, y), Cell::Cargo)
    }

    pub fn bay_at(&self, x: i32, y: i32) -> Option<usize> {
        match self.cell(x, y) {
            Cell::Bay(i) => Some(i),
            _ => None,
        }
    }

    /// Odd rows are shifted half a cell, original even-r hex.
    pub fn row_offset(y: i32) -> bool {
        y & 1 == 1
    }
}

/// All `MATRIX*_BODY` boards, keyed by mechos name (`OxidizeMonk`, `Ripper`, …).
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    by_key: HashMap<String, Layout>,
}

impl Catalog {
    pub fn load(data_path: &Path) -> Self {
        for dir in search_dirs(data_path) {
            let actint = dir.join("actint.inc");
            let names = dir.join("a_str.inc");
            if let (Ok(a), Ok(n)) = (std::fs::read(&actint), std::fs::read(&names)) {
                log::info!("mechos boards from {}", actint.display());
                return Self::parse(&String::from_utf8_lossy(&a), &String::from_utf8_lossy(&n));
            }
        }
        Catalog::default()
    }

    pub fn parse(actint: &str, names: &str) -> Self {
        let defs = defines(actint);
        let labels = mech_names(names);
        let mut by_key = HashMap::new();
        for (name, body) in &defs {
            let Some(id) = matrix_id(name, "_BODY") else {
                continue;
            };
            let expanded = expand_matrix(&defs, body);
            let Some(layout) = parse_grid(&expanded) else {
                continue;
            };
            if let Some(label) = labels.get(&id) {
                by_key.insert(key(label), layout.clone());
            }
            by_key.insert(format!("mech{id:02}"), layout);
        }
        Catalog { by_key }
    }

    pub fn layout_for(&self, car: &str) -> Option<&Layout> {
        let k = key(car);
        if let Some(layout) = self.by_key.get(&k) {
            return Some(layout);
        }
        let stripped = k.strip_prefix("the").unwrap_or(&k);
        self.by_key.get(stripped)
    }
}

fn search_dirs(data_path: &Path) -> Vec<std::path::PathBuf> {
    vec![
        data_path.join("actint"),
        Path::new("../Vangers/data/actint").to_path_buf(),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../Vangers/data/actint"),
    ]
}

fn key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn matrix_id(define: &str, suffix: &str) -> Option<u32> {
    let rest = define.strip_prefix("MATRIX")?.strip_suffix(suffix)?;
    rest.parse().ok()
}

fn defines(src: &str) -> HashMap<String, String> {
    let src = strip_comments(src);
    let mut out = HashMap::new();
    let mut pending: Option<(String, String)> = None;
    for raw in src.lines() {
        if let Some((name, mut body)) = pending.take() {
            let continued = raw.trim_end().ends_with('\\');
            let piece = raw.trim_end().trim_end_matches('\\');
            body.push(' ');
            body.push_str(piece.trim());
            if continued {
                pending = Some((name, body));
            } else {
                out.insert(name, body);
            }
            continue;
        }
        let line = raw.trim();
        let Some(rest) = line.strip_prefix("#define") else {
            continue;
        };
        let rest = rest.trim();
        let Some(split) = rest.find(|c: char| c.is_whitespace()) else {
            continue;
        };
        let name = rest[..split].to_string();
        let body = rest[split + 1..].trim();
        if body.ends_with('\\') {
            pending = Some((name, body.trim_end_matches('\\').trim().to_string()));
        } else {
            out.insert(name, body.to_string());
        }
    }
    out
}

fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut chars = src.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            while let Some(c) = chars.next() {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            for c in chars.by_ref() {
                if c == '\n' {
                    out.push('\n');
                    break;
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

fn expand_matrix(defs: &HashMap<String, String>, body: &str) -> String {
    let mut out = body.to_string();
    for _ in 0..8 {
        let mut next = String::with_capacity(out.len());
        let mut changed = false;
        let mut rest = out.as_str();
        while let Some(at) = rest.find('$') {
            next.push_str(&rest[..at]);
            rest = &rest[at + 1..];
            let n = rest
                .find(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .unwrap_or(rest.len());
            let name = &rest[..n];
            rest = &rest[n..];
            if let Some(rep) = defs.get(name) {
                next.push_str(rep);
                changed = true;
            }
        }
        next.push_str(rest);
        out = next;
        if !changed {
            break;
        }
    }
    out
}

fn mech_names(src: &str) -> HashMap<u32, String> {
    let mut out = HashMap::new();
    for (name, body) in defines(src) {
        let Some(id) = name
            .strip_prefix("MECH")
            .and_then(|s| s.strip_suffix("_NAME1"))
            .and_then(|s| s.parse().ok())
        else {
            continue;
        };
        if let Some(label) = quoted(&body) {
            out.insert(id, label);
        }
    }
    out
}

fn quoted(s: &str) -> Option<String> {
    let start = s.find('"')?;
    let end = s[start + 1..].find('"')?;
    Some(s[start + 1..start + 1 + end].to_string())
}

enum Tok<'a> {
    Word(&'a str),
    Num(u8),
    BraceOpen,
    BraceClose,
}

fn parse_grid(body: &str) -> Option<Layout> {
    let tokens = tokenize(body);
    let mut width = 0i32;
    let mut height = 0i32;
    let mut matrix = Vec::new();
    let mut types = Vec::new();
    let mut nums = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        match tokens[i] {
            Tok::Word("msize_x") if i + 1 < tokens.len() => {
                if let Tok::Num(n) = tokens[i + 1] {
                    width = n as i32;
                    i += 2;
                    continue;
                }
            }
            Tok::Word("msize_y") if i + 1 < tokens.len() => {
                if let Tok::Num(n) = tokens[i + 1] {
                    height = n as i32;
                    i += 2;
                    continue;
                }
            }
            Tok::Word("matrix") => {
                matrix = brace_nums(&tokens, &mut i);
                continue;
            }
            Tok::Word("slot_types") => {
                types = brace_nums(&tokens, &mut i);
                continue;
            }
            Tok::Word("slot_nums") => {
                nums = brace_nums(&tokens, &mut i);
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    if width <= 0 || height <= 0 {
        return None;
    }
    let n = (width * height) as usize;
    if matrix.len() != n {
        return None;
    }
    types.resize(n, 0);
    nums.resize(n, 0);
    Some(from_tables(width, height, &matrix, &types, &nums))
}

fn tokenize(body: &str) -> Vec<Tok<'_>> {
    let mut out = Vec::new();
    let mut rest = body;
    while !rest.is_empty() {
        let c = rest.chars().next().unwrap();
        if c.is_whitespace() {
            rest = rest[c.len_utf8()..].trim_start();
            continue;
        }
        if c == '{' {
            out.push(Tok::BraceOpen);
            rest = &rest[1..];
            continue;
        }
        if c == '}' {
            out.push(Tok::BraceClose);
            rest = &rest[1..];
            continue;
        }
        if c == '$' {
            let n = rest[1..]
                .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .map(|i| i + 1)
                .unwrap_or(rest.len());
            rest = &rest[n..];
            continue;
        }
        if c == '"' {
            if let Some(end) = rest[1..].find('"') {
                rest = &rest[end + 2..];
            } else {
                break;
            }
            continue;
        }
        if c.is_ascii_digit() {
            let n = rest.find(|ch: char| !ch.is_ascii_digit()).unwrap_or(rest.len());
            if let Ok(v) = rest[..n].parse::<u8>() {
                out.push(Tok::Num(v));
            }
            rest = &rest[n..];
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let n = rest
                .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .unwrap_or(rest.len());
            out.push(Tok::Word(&rest[..n]));
            rest = &rest[n..];
            continue;
        }
        rest = &rest[c.len_utf8()..];
    }
    out
}

fn brace_nums(tokens: &[Tok<'_>], i: &mut usize) -> Vec<u8> {
    let mut nums = Vec::new();
    while *i < tokens.len() && !matches!(tokens[*i], Tok::BraceOpen) {
        *i += 1;
    }
    if *i >= tokens.len() {
        return nums;
    }
    *i += 1;
    while *i < tokens.len() {
        match tokens[*i] {
            Tok::BraceClose => {
                *i += 1;
                break;
            }
            Tok::Num(n) => nums.push(n),
            _ => {}
        }
        *i += 1;
    }
    nums
}

fn from_tables(width: i32, height: i32, matrix: &[u8], types: &[u8], nums: &[u8]) -> Layout {
    let mut unique = Vec::new();
    for i in 0..matrix.len() {
        let n = nums[i];
        if matrix[i] != 0 && n != 0 && !unique.contains(&n) {
            unique.push(n);
        }
    }
    unique.sort_unstable();
    let mut cells = Vec::with_capacity(matrix.len());
    for i in 0..matrix.len() {
        if matrix[i] == 0 {
            cells.push(Cell::Empty);
            continue;
        }
        let n = nums[i];
        if n != 0
            && types[i] != 0
            && let Some(bay) = unique.iter().position(|&u| u == n)
            && bay < BAYS
        {
            cells.push(Cell::Bay(bay));
            continue;
        }
        cells.push(Cell::Cargo);
    }
    Layout {
        width,
        height,
        cells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTINT: &str = r#"
#define MATRIX00_BODY		msize_x 8			\
				msize_y 14			\
				matrix {			\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
					0 1 0 0 0 0 0 1 	\
					 1 1 0 0 1 0 1 1	\
					0 1 0 0 0 0 0 1 	\
					 0 0 1 1 1 0 0 0	\
					0 0 0 1 1 0 0 0 	\
					 0 0 1 1 1 0 0 0	\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
				}				\
				slot_types {			\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
					0 1 0 0 0 0 0 1 	\
					 1 1 0 0 2 0 1 1	\
					0 1 0 0 0 0 0 1 	\
					 0 0 0 0 0 0 0 0	\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
				}				\
				slot_nums {			\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
					0 1 0 0 0 0 0 2 	\
					 1 1 0 0 4 0 2 2	\
					0 1 0 0 0 0 0 2 	\
					 0 0 0 0 0 0 0 0	\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
					0 0 0 0 0 0 0 0 	\
					 0 0 0 0 0 0 0 0	\
				}				\
				id	$MECH00_NAME
"#;

    const NAMES: &str = r#"
#define MECH00_NAME1	"Oxidize Monk"
#define MECH01_NAME1	"Blade Keeper"
"#;

    #[test]
    fn actint_macros_are_the_oxidize_monk_board() {
        let cat = Catalog::parse(ACTINT, NAMES);
        let layout = cat.layout_for("OxidizeMonk").expect("Oxidize Monk");
        assert_eq!(layout.width, 8);
        assert_eq!(layout.height, 14);
        let occupied = (0..14)
            .flat_map(|y| (0..8).map(move |x| (x, y)))
            .filter(|&(x, y)| layout.cell(x, y) != Cell::Empty)
            .count();
        assert_eq!(occupied, 17);
        assert_eq!(layout.cell(0, 0), Cell::Empty);
        assert_eq!(layout.cell(1, 4), Cell::Bay(0));
        assert_eq!(layout.cell(7, 4), Cell::Bay(1));
        assert_eq!(layout.cell(4, 5), Cell::Bay(2));
        assert!(layout.is_cargo(3, 7));
        assert_eq!(cat.layout_for("Oxidize Monk"), Some(layout));
        assert!(Layout::row_offset(5));
        assert!(!Layout::row_offset(4));
    }

    #[test]
    fn original_actint_inc_names_the_boards() {
        let dir = Path::new("../Vangers/data/actint");
        let actint = dir.join("actint.inc");
        let names = dir.join("a_str.inc");
        if !actint.is_file() {
            return;
        }
        let cat = Catalog::parse(
            &String::from_utf8_lossy(&std::fs::read(actint).unwrap()),
            &String::from_utf8_lossy(&std::fs::read(names).unwrap()),
        );
        let monk = cat.layout_for("OxidizeMonk").expect("Oxidize Monk");
        assert_eq!(monk.cell(1, 4), Cell::Bay(0));
        assert_ne!(cat.layout_for("IronShadow"), Some(monk));
        assert_eq!(cat.layout_for("TheRipper").unwrap().width, 8);
        assert_eq!(cat.layout_for("BladeKeeper").unwrap().width, 8);
    }

    #[test]
    fn the_pack_is_a_full_cargo_rectangle() {
        let pack = Layout::pack();
        assert_eq!(pack.width, PACK_WIDTH);
        assert_eq!(pack.height, PACK_HEIGHT);
        assert!(pack.is_cargo(0, 0));
        assert!(pack.is_cargo(7, 5));
        assert!(!pack.is_cargo(8, 0));
    }
}
