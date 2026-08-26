//! Loader and session for the original diagen files.
//!
//! A room is three files that share a name: `{name}.text` (counselor
//! phrases, grouped into *molecules*), `{name}.query` (player questions
//! and their answers), and `{name}.dil` (which molecule plays when, gated
//! by an access expression). English strings are kept; Russian dual-language
//! twins are skipped the way `dgFile::getElement(DGF_DUAL)` does.
//!
//! A session steps counselor phrases, involves `{link}` queries as it goes,
//! and answers a listed query from the `.query` file. Gift/command side
//! effects of `doCMD` are not run.

use std::collections::HashMap;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

/// One counselor line, plus the query names `{braced}` on it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Atom {
    pub text: String,
    pub links: Vec<String>,
}

/// A player question and the counselor's answers, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub name: String,
    pub answers: Vec<Atom>,
}

/// One cell of the `.dil` grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
    pub name: String,
    pub access: String,
}

/// Everything one escave's counselor knows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Room {
    pub name: String,
    pub prefix: String,
    pub postfix: String,
    pub molecules: HashMap<String, Vec<Atom>>,
    pub queries: HashMap<String, Query>,
    pub cells: Vec<Cell>,
    pub start: (i32, i32),
}

impl Room {
    /// Load `{dir}/{name}.text`, `.query`, and `.dil`.
    pub fn load(dir: &Path, name: &str) -> io::Result<Self> {
        let text = read_lossy(&dir.join(format!("{name}.text")))?;
        let query = read_lossy(&dir.join(format!("{name}.query")))?;
        let dil = read_lossy(&dir.join(format!("{name}.dil")))?;
        Ok(Self::parse(name, &text, &query, &dil))
    }

    pub fn parse(name: &str, text: &str, query: &str, dil: &str) -> Self {
        let (prefix, postfix, queries) = parse_queries(query);
        Room {
            name: name.to_string(),
            prefix,
            postfix,
            molecules: parse_text(text),
            queries,
            cells: parse_dil(dil),
            start: (0, 0),
        }
    }

    fn molecule(&self, name: &str) -> Option<&[Atom]> {
        self.molecules.get(name).map(|a| a.as_slice())
    }

    fn opening_molecule(&self) -> Option<&str> {
        let at_start = self
            .cells
            .iter()
            .find(|c| (c.x, c.y) == self.start && access_ok(&c.access));
        if let Some(cell) = at_start {
            return Some(cell.name.as_str());
        }
        self.cells
            .iter()
            .find(|c| access_ok(&c.access))
            .map(|c| c.name.as_str())
    }
}

/// A conversation in progress.
pub struct Session {
    room: Room,
    molecule: Option<String>,
    phrase: usize,
    visible: Vec<String>,
    ended: bool,
    last: Option<String>,
}

impl Session {
    pub fn start(room: Room) -> Self {
        let molecule = room.opening_molecule().map(str::to_string);
        Session {
            room,
            molecule,
            phrase: 0,
            visible: Vec::new(),
            ended: false,
            last: None,
        }
    }

    pub fn room(&self) -> &Room {
        &self.room
    }

    pub fn ended(&self) -> bool {
        self.ended
    }

    pub fn last_phrase(&self) -> Option<&str> {
        self.last.as_deref()
    }

    /// Query names the counselor has involved so far, in first-seen order.
    pub fn queries(&self) -> &[String] {
        &self.visible
    }

    /// How a listed query is asked: the `.query` file wraps names with
    /// `"What is"` / `"counselor?"`.
    pub fn query_prompt(&self, name: &str) -> String {
        let prefix = self.room.prefix.trim();
        let postfix = self.room.postfix.trim();
        if prefix.is_empty() && postfix.is_empty() {
            name.to_string()
        } else if prefix.is_empty() {
            format!("{name} {postfix}")
        } else if postfix.is_empty() {
            format!("{prefix} {name}")
        } else {
            format!("{prefix} {name} {postfix}")
        }
    }

    /// Next counselor line, or `None` once the opening molecule is spent.
    pub fn next_phrase(&mut self) -> Option<String> {
        let name = self.molecule.as_ref()?;
        let atom = {
            let atoms = self.room.molecule(name)?;
            if self.phrase >= atoms.len() {
                self.ended = true;
                self.last = None;
                return None;
            }
            let atom = atoms[self.phrase].clone();
            self.phrase += 1;
            if self.phrase >= atoms.len() {
                self.ended = true;
            }
            atom
        };
        for link in atom.links.iter() {
            self.involve(link);
        }
        self.last = Some(atom.text.clone());
        Some(atom.text)
    }

    /// Answer a listed query from the `.query` file. Unknown or uninvolved
    /// names yield `None`.
    pub fn answer(&mut self, name: &str) -> Option<String> {
        if !self.visible.iter().any(|q| q == name) {
            return None;
        }
        let atom = self.room.queries.get(name)?.answers.first()?.clone();
        for link in atom.links.iter() {
            self.involve(link);
        }
        self.last = Some(atom.text.clone());
        Some(atom.text)
    }

    fn involve(&mut self, name: &str) {
        if self.room.queries.contains_key(name) && !self.visible.iter().any(|q| q == name) {
            self.visible.push(name.to_string());
        }
    }
}

fn read_lossy(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)
        .map_err(|e| io::Error::new(ErrorKind::NotFound, format!("{}: {e}", path.display())))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn access_ok(s: &str) -> bool {
    let t = s.trim();
    t.is_empty() || t == "true"
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Quoted(String),
    Subject(String),
    Link(String),
    Word(String),
}

impl Token {
    fn as_str(&self) -> &str {
        match *self {
            Token::Quoted(ref s)
            | Token::Subject(ref s)
            | Token::Link(ref s)
            | Token::Word(ref s) => s,
        }
    }

    fn subject(tok: &Token) -> Option<&str> {
        match *tok {
            Token::Subject(ref s) => Some(s.as_str()),
            _ => None,
        }
    }
}

fn tokenize(src: &str) -> Vec<Token> {
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let mut out = Vec::new();
    while i < chars.len() {
        if chars[i] == '/' && i + 1 < chars.len() && chars[i + 1] == '*' {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            continue;
        }
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '"' {
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '"' {
                i += 1;
            }
            out.push(Token::Quoted(chars[start..i].iter().collect()));
            if i < chars.len() {
                i += 1;
            }
            continue;
        }
        if c == '[' {
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != ']' {
                i += 1;
            }
            out.push(Token::Subject(strip_mood(
                &chars[start..i].iter().collect::<String>(),
            )));
            if i < chars.len() {
                i += 1;
            }
            continue;
        }
        if c == '{' {
            i += 1;
            let start = i;
            while i < chars.len() && chars[i] != '}' {
                i += 1;
            }
            out.push(Token::Link(chars[start..i].iter().collect()));
            if i < chars.len() {
                i += 1;
            }
            continue;
        }
        let start = i;
        while i < chars.len()
            && !chars[i].is_whitespace()
            && chars[i] != '"'
            && chars[i] != '['
            && chars[i] != '{'
        {
            i += 1;
        }
        out.push(Token::Word(chars[start..i].iter().collect()));
    }
    out
}

fn strip_mood(name: &str) -> String {
    match name.find(':') {
        Some(i) => name[..i].to_string(),
        None => name.to_string(),
    }
}

/// Keep this token when reading a dual-language stream: subjects always,
/// otherwise the first alphanumeric character has to be ASCII so Russian
/// (or lossy-decoded) twins drop out.
fn keep_dual(tok: &Token) -> bool {
    match *tok {
        Token::Subject(_) => true,
        Token::Word(ref s) if s.starts_with('@') || s.starts_with('$') => true,
        Token::Quoted(ref s) | Token::Link(ref s) | Token::Word(ref s) => is_english(s),
    }
}

fn is_english(s: &str) -> bool {
    match s.chars().find(|c| c.is_alphanumeric()) {
        Some(c) => c.is_ascii_alphanumeric(),
        None => true,
    }
}

struct Cursor<'a> {
    tokens: &'a [Token],
    i: usize,
    dual: bool,
}

impl<'a> Cursor<'a> {
    fn new(tokens: &'a [Token], dual: bool) -> Self {
        Cursor { tokens, i: 0, dual }
    }

    fn next(&mut self) -> Option<&'a Token> {
        while self.i < self.tokens.len() {
            let t = &self.tokens[self.i];
            self.i += 1;
            if self.dual && !keep_dual(t) {
                continue;
            }
            return Some(t);
        }
        None
    }

    fn peek(&mut self) -> Option<&'a Token> {
        let saved = self.i;
        let t = self.next();
        self.i = saved;
        t
    }
}

fn parse_text(src: &str) -> HashMap<String, Vec<Atom>> {
    let tokens = tokenize(src);
    let mut cur = Cursor::new(&tokens, true);
    let mut molecules = HashMap::new();
    while let Some(tok) = cur.next() {
        let Token::Subject(ref name) = *tok else {
            continue;
        };
        let name = name.clone();
        let mut atoms = Vec::new();
        while let Some(tok) = cur.peek() {
            match *tok {
                Token::Subject(_) => break,
                Token::Quoted(ref s) => {
                    let text = s.clone();
                    cur.next();
                    atoms.push(Atom {
                        text,
                        links: Vec::new(),
                    });
                }
                Token::Link(ref s) => {
                    let link = s.clone();
                    cur.next();
                    if let Some(last) = atoms.last_mut() {
                        last.links.push(link);
                    }
                }
                Token::Word(_) => {
                    cur.next();
                }
            }
        }
        molecules.insert(name, atoms);
    }
    molecules
}

fn parse_queries(src: &str) -> (String, String, HashMap<String, Query>) {
    let tokens = tokenize(src);
    let mut cur = Cursor::new(&tokens, true);
    // Dual-language files lead with the Russian wrapper, then the English
    // `"What is"` / `"counselor?"`. Lossy CP1251 turns Cyrillic into `�`,
    // which `is_english` cannot tell from punctuation, so require a letter.
    let prefix = next_english_quoted(&mut cur);
    let postfix = next_english_quoted(&mut cur);

    let mut queries = HashMap::new();
    while let Some(tok) = cur.next() {
        let Token::Subject(ref first) = *tok else {
            continue;
        };
        let first = first.clone();
        let second = match cur.peek().and_then(Token::subject) {
            Some(s) => {
                cur.next();
                s.to_string()
            }
            None => first.clone(),
        };
        let name = english_name(&first, &second);
        let mut answers = Vec::new();
        while let Some(tok) = cur.peek() {
            match *tok {
                Token::Subject(_) => break,
                Token::Quoted(ref s) => {
                    let text = s.clone();
                    cur.next();
                    answers.push(Atom {
                        text,
                        links: Vec::new(),
                    });
                }
                Token::Link(ref s) => {
                    let link = s.clone();
                    cur.next();
                    if let Some(last) = answers.last_mut() {
                        last.links.push(link);
                    }
                }
                Token::Word(_) => {
                    cur.next();
                }
            }
        }
        queries.insert(name.clone(), Query { name, answers });
    }
    (prefix, postfix, queries)
}

fn next_english_quoted(cur: &mut Cursor<'_>) -> String {
    while let Some(tok) = cur.peek() {
        let Token::Quoted(ref s) = *tok else {
            break;
        };
        let s = s.clone();
        cur.next();
        if s.chars().any(|c| c.is_ascii_alphabetic()) {
            return s;
        }
    }
    String::new()
}

fn english_name(first: &str, second: &str) -> String {
    if is_english(second) {
        second.to_string()
    } else if is_english(first) {
        first.to_string()
    } else {
        second.to_string()
    }
}

fn parse_dil(src: &str) -> Vec<Cell> {
    let tokens = tokenize(src);
    let mut cur = Cursor::new(&tokens, false);
    let _sx = token_i32(cur.next());
    let _sy = token_i32(cur.next());
    let n = token_i32(cur.next()).max(0) as usize;
    let _skip = cur.next();
    let mut cells = Vec::with_capacity(n);
    for _ in 0..n {
        let x = token_i32(cur.next());
        let y = token_i32(cur.next());
        let name = cur.next().map(Token::as_str).unwrap_or("").to_string();
        let _ty = cur.next();
        let _wait = cur.next();
        let _looping = cur.next();
        let access = cur.next().map(Token::as_str).unwrap_or("").to_string();
        let _post = cur.next();
        let _start = cur.next();
        cells.push(Cell { x, y, name, access });
    }
    cells
}

fn token_i32(tok: Option<&Token>) -> i32 {
    tok.and_then(|t| t.as_str().trim().parse().ok())
        .unwrap_or(0)
}

/// Directories the original (and this repo's fixtures) may keep dialog in.
pub fn search_dirs(data_path: &Path) -> Vec<std::path::PathBuf> {
    vec![
        data_path.join("data"),
        data_path.to_path_buf(),
        Path::new("../Vangers/data/data").to_path_buf(),
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/escave"),
    ]
}

pub fn find_room(data_path: &Path, name: &str) -> Option<Room> {
    for dir in search_dirs(data_path) {
        if dir.join(format!("{name}.text")).exists() {
            return Room::load(&dir, name).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_dir() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/escave")
    }

    #[test]
    fn podish_loads_from_original_format_files() {
        let room = Room::load(&fixture_dir(), "Podish").expect("Podish fixtures");
        assert!(
            room.molecules.contains_key("Leepky Introduction"),
            "missing introduction molecule, have {:?}",
            room.molecules.keys().collect::<Vec<_>>()
        );
        assert!(
            room.queries.contains_key("mechos"),
            "missing mechos query, have {:?}",
            room.queries.keys().collect::<Vec<_>>()
        );
        assert!(
            room.cells.iter().any(|c| c.name == "Leepky Introduction"),
            "introduction cell missing"
        );
    }

    #[test]
    fn stepping_the_introduction_yields_counselor_text() {
        let room = Room::load(&fixture_dir(), "Podish").unwrap();
        let intro: Vec<String> = room
            .molecules
            .get("Leepky Introduction")
            .expect("introduction")
            .iter()
            .map(|a| a.text.clone())
            .collect();
        assert!(!intro.is_empty());
        let mut session = Session::start(room);
        let phrase = session.next_phrase().expect("no opening phrase");
        assert!(
            intro.iter().any(|t| t == &phrase),
            "phrase not from the introduction data: {phrase:?}"
        );
        assert!(
            phrase.contains("pilgarlic") || phrase.contains("Welcome") || !phrase.is_empty(),
            "unexpected introduction: {phrase:?}"
        );
    }

    #[test]
    fn a_listed_query_answers_from_the_query_file() {
        let room = Room::load(&fixture_dir(), "Podish").unwrap();
        let mechos = room
            .queries
            .get("mechos")
            .expect("mechos query")
            .answers
            .clone();
        assert!(!mechos.is_empty());
        let mut session = Session::start(room);
        // Walk the introduction so `{mechos}` links get involved.
        while session.next_phrase().is_some() {}
        assert!(
            !session.queries().is_empty(),
            "introduction involved no queries"
        );
        assert!(
            session.queries().iter().any(|q| q == "mechos"),
            "mechos was not involved: {:?}",
            session.queries()
        );
        let prompt = session.query_prompt("mechos");
        assert!(
            prompt.contains("mechos") && prompt.contains("What is"),
            "query wrapper missing: {prompt:?}"
        );
        let answer = session.answer("mechos").expect("no answer");
        assert!(
            mechos.iter().any(|a| a.text == answer),
            "answer not from the query data: {answer:?}"
        );
        assert_eq!(
            session.last_phrase(),
            Some(answer.as_str()),
            "Ask must replace the counselor line with the answer"
        );
    }
}
