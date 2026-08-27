//! Mechos inventory boards from `actint.inc` `invMatrix` / `MATRIX*_BODY`.
//!
//! Odd rows are drawn offset, matching the original even-r hex. Cells with
//! `matrix == 0` are not part of the vehicle. `slot_types` / `slot_nums`
//! mark weapon and device bays; everything else is cargo.

/// Bays we can hang on an m3d (the original hardpoint count).
const BAYS: usize = 3;

/// Cargo-only rectangle used by tests and a vehicle we do not know.
pub const PACK_WIDTH: i32 = 8;
pub const PACK_HEIGHT: i32 = 6;

/// One cell of a mechos board.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    /// Not on this vehicle.
    Empty,
    /// Trade cargo can sit here.
    Cargo,
    /// A weapon / device hardpoint. Index is 0..[`BAYS`].
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
    pub fn pack() -> Self {
        Layout {
            width: PACK_WIDTH,
            height: PACK_HEIGHT,
            cells: vec![Cell::Cargo; (PACK_WIDTH * PACK_HEIGHT) as usize],
        }
    }

    /// `Oxidize Monk` / `MECH00`. Default car.
    pub fn oxidize_monk() -> Self {
        from_tables(
            [
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 1, 0, 0, 0, 0, 0, 1],
                [1, 1, 0, 0, 1, 0, 1, 1],
                [0, 1, 0, 0, 0, 0, 0, 1],
                [0, 0, 1, 1, 1, 0, 0, 0],
                [0, 0, 0, 1, 1, 0, 0, 0],
                [0, 0, 1, 1, 1, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
            ],
            [
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 1, 0, 0, 0, 0, 0, 1],
                [1, 1, 0, 0, 2, 0, 1, 1],
                [0, 1, 0, 0, 0, 0, 0, 1],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
            ],
            [
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 1, 0, 0, 0, 0, 0, 2],
                [1, 1, 0, 0, 4, 0, 2, 2],
                [0, 1, 0, 0, 0, 0, 0, 2],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
            ],
        )
    }

    /// `Blade Keeper` / `MECH01`.
    pub fn blade_keeper() -> Self {
        from_tables(
            [
                [0, 0, 0, 0, 0, 0, 0, 0],
                [1, 0, 1, 1, 0, 0, 1, 0],
                [1, 1, 0, 1, 0, 0, 1, 1],
                [1, 0, 0, 0, 1, 0, 1, 0],
                [1, 1, 0, 0, 0, 0, 1, 1],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 1, 1, 1, 1, 1, 0],
                [1, 1, 1, 1, 1, 1, 1, 1],
                [0, 0, 1, 1, 1, 1, 1, 0],
                [1, 1, 1, 1, 1, 1, 1, 1],
                [0, 0, 1, 1, 1, 1, 1, 0],
                [0, 1, 1, 1, 1, 1, 1, 0],
                [0, 0, 1, 1, 0, 1, 1, 0],
                [0, 0, 1, 0, 0, 1, 0, 0],
            ],
            [
                [0, 0, 0, 0, 0, 0, 0, 0],
                [1, 0, 3, 3, 0, 0, 1, 0],
                [1, 1, 0, 3, 0, 0, 1, 1],
                [1, 0, 0, 0, 2, 0, 1, 0],
                [1, 1, 0, 0, 0, 0, 1, 1],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
            ],
            [
                [0, 0, 0, 0, 0, 0, 0, 0],
                [1, 0, 4, 4, 0, 0, 2, 0],
                [1, 1, 0, 4, 0, 0, 2, 2],
                [1, 0, 0, 0, 5, 0, 2, 0],
                [1, 1, 0, 0, 0, 0, 2, 2],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
            ],
        )
    }

    /// `Iron Shadow` / `MECH06`.
    pub fn iron_shadow() -> Self {
        from_tables(
            [
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 1, 1, 0, 0, 1, 0],
                [0, 0, 1, 1, 1, 0, 1, 1],
                [1, 0, 1, 1, 0, 0, 1, 0],
                [0, 0, 1, 1, 1, 0, 1, 1],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 1, 1, 0, 0],
                [1, 0, 0, 0, 1, 0, 0, 0],
                [1, 1, 1, 0, 0, 0, 0, 1],
                [0, 1, 1, 1, 1, 1, 1, 1],
                [0, 0, 1, 1, 1, 1, 0, 0],
                [0, 0, 1, 1, 1, 1, 0, 0],
                [1, 1, 1, 1, 1, 1, 1, 1],
                [1, 1, 0, 0, 0, 0, 1, 1],
            ],
            [
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 1, 1, 0, 0, 1, 0],
                [0, 0, 1, 1, 1, 0, 1, 1],
                [2, 0, 1, 1, 0, 0, 1, 0],
                [0, 0, 1, 1, 1, 0, 1, 1],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 3, 3, 0, 0],
                [0, 0, 0, 0, 3, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
            ],
            [
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 3, 3, 0, 0, 2, 0],
                [0, 0, 3, 3, 3, 0, 2, 2],
                [5, 0, 3, 3, 0, 0, 2, 0],
                [0, 0, 3, 3, 3, 0, 2, 2],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 4, 4, 0, 0],
                [0, 0, 0, 0, 4, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
                [0, 0, 0, 0, 0, 0, 0, 0],
            ],
        )
    }

    pub fn for_car(name: &str) -> Self {
        match name {
            "OxidizeMonk" | "Oxidize Monk" => Self::oxidize_monk(),
            "BladeKeeper" | "Blade Keeper" => Self::blade_keeper(),
            "IronShadow" | "Iron Shadow" => Self::iron_shadow(),
            _ => Self::oxidize_monk(),
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

fn from_tables(matrix: [[u8; 8]; 14], types: [[u8; 8]; 14], nums: [[u8; 8]; 14]) -> Layout {
    let mut unique = Vec::new();
    for y in 0..14 {
        for x in 0..8 {
            let n = nums[y][x];
            if matrix[y][x] != 0 && n != 0 && !unique.contains(&n) {
                unique.push(n);
            }
        }
    }
    unique.sort_unstable();
    let mut cells = Vec::with_capacity(8 * 14);
    for y in 0..14 {
        for x in 0..8 {
            if matrix[y][x] == 0 {
                cells.push(Cell::Empty);
                continue;
            }
            let n = nums[y][x];
            if n != 0
                && types[y][x] != 0
                && let Some(i) = unique.iter().position(|&u| u == n)
                && i < BAYS
            {
                cells.push(Cell::Bay(i));
                continue;
            }
            cells.push(Cell::Cargo);
        }
    }
    Layout {
        width: 8,
        height: 14,
        cells,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oxidize_monk_matches_the_original_silhouette() {
        let layout = Layout::oxidize_monk();
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
        assert!(Layout::row_offset(5));
        assert!(!Layout::row_offset(4));
    }

    #[test]
    fn a_named_car_picks_its_board() {
        assert_eq!(Layout::for_car("OxidizeMonk"), Layout::oxidize_monk());
        assert_eq!(Layout::for_car("BladeKeeper"), Layout::blade_keeper());
        assert_eq!(Layout::for_car("IronShadow"), Layout::iron_shadow());
        assert_eq!(Layout::for_car("MysteryVan"), Layout::oxidize_monk());
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
