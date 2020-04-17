use serde::Deserialize;
use serde_scan;

use std::fmt::Debug;
use std::io::{BufRead, BufReader, Read};
use std::str::FromStr;

pub struct Reader<I> {
    input: BufReader<I>,
    line: String,
}

impl<I: Read> Reader<I> {
    pub fn new(input: I) -> Self {
        Reader {
            input: BufReader::new(input),
            line: String::new(),
        }
    }

    pub fn cur(&self) -> &str {
        self.line.trim_end()
    }

    pub fn advance(&mut self) -> bool {
        loop {
            self.line.clear();
            match self.input.read_line(&mut self.line) {
                Ok(0) => return false,
                Ok(_) => (),
                Err(_) => continue, // non-UTF8 (rus)
            };
            if self.line.starts_with("/*") {
                while !self.cur().ends_with("*/") {
                    self.advance();
                }
            } else if !self.cur().is_empty() && !self.line.starts_with("//") {
                return true;
            }
        }
    }

    pub fn next_value<T>(&mut self) -> T
    where
        T: FromStr,
        T::Err: Debug,
    {
        self.advance();
        self.cur().parse().unwrap()
    }

    pub fn next_key_value<T>(&mut self, key: &str) -> T
    where
        T: FromStr,
        T::Err: Debug,
    {
        self.advance();
        let mut tokens = self.line.split_whitespace();
        let name = tokens.next().unwrap();
        assert_eq!(name, key);
        tokens.next().unwrap().parse().unwrap()
    }

    pub fn next_entry<T>(&mut self) -> (&str, Vec<T>)
    where
        T: FromStr,
        T::Err: Debug,
    {
        self.advance();
        let mut tokens = self.line.split_whitespace();
        let name = tokens.next().unwrap();
        let data = tokens.map(|t| t.parse().unwrap()).collect();
        (name, data)
    }

    pub fn scan<'a, T: Deserialize<'a>>(&'a mut self) -> T {
        serde_scan::from_str(&self.line).expect(&format!("Unable to scan line: {}", self.line))
    }
}
