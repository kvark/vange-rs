use config::text::Reader;

use std::fs::File;

#[derive(Debug, Deserialize)]
pub struct ItemSource {
    pub item: String,
    pub escave: String,
}

#[derive(Debug, Deserialize)]
pub struct Escave {
    pub name: String,
    pub world: String,
    pub coordinates: [u32; 2],
    pub special_item: String,
    pub need_items: Vec<ItemSource>,
}

pub fn load(file: File) -> Vec<Escave> {
    let mut escaves = Vec::new();
    let mut fi = Reader::new(file);
    fi.advance();
    assert_eq!(fi.cur(), "uniVang-ParametersFile_Ver_1");

    while fi.advance() {
        let (name, world, x, y, special_item): (String, String, u32, u32, String) = fi.scan();
        info!("Escave {} in {} at {}x{}", name, world, x, y);
        let mut need_items = Vec::new();
        while { fi.advance(); fi.cur() != "none" } {
            let item = fi.scan();
            need_items.push(item);
        }
        escaves.push(Escave {
            name,
            world,
            coordinates: [x, y],
            special_item,
            need_items,
        });
    }
    escaves
}
