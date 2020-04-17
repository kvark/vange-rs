use crate::config::text::Reader;

use std::collections::HashMap;
use std::fs::File;

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
