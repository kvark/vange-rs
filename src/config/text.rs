//use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};


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

    pub fn next(&mut self) -> &str {
        self.advance();
        self.cur()
    }
}
