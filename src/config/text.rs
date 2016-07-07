use std::fmt::Debug;
use std::io::{BufRead, BufReader, Read};
use std::str::FromStr;


pub struct Reader<I> {
    input: BufReader<I>,
    line: String,
}

impl<I: Read> Reader<I> {
    pub fn new(input: I) -> Reader<I> {
        Reader {
            input: BufReader::new(input),
            line: String::new(),
        }
    }

    pub fn cur(&self) -> &str {
        self.line.trim_right()
    }

    pub fn advance(&mut self) {
        loop {
            self.line.clear();
            self.input.read_line(&mut self.line).unwrap();
            if self.line.starts_with("/*") {
                while !self.cur().ends_with("*/") {
                    self.advance();
                }
            } else if !self.cur().is_empty() && !self.line.starts_with("//") {
                break
            }
        }
    }

    pub fn next_value<T>(&mut self) -> T where
        T: FromStr,
        T::Err: Debug,
    {
        self.advance();
        self.cur().parse().unwrap()
    }

    pub fn next_entry<T>(&mut self) -> (&str, Vec<T>) where
        T: FromStr,
        T::Err: Debug,
    {
        self.advance();
        let mut tokens = self.line.split_whitespace();
        let name = tokens.next().unwrap();
        let data = tokens.map(|t| t.parse().unwrap()).collect();
        (name, data)
    }
}
