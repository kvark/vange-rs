//! Buy and sell against beeb credits, on a mechos board.
//!
//! Names of the default wares come from `actintItemTypes` (Nymbos, Phlegma,
//! and the other Fostral trade goods). Weapon ids match `game.lst`.
//! Occupancy follows the vehicle's hex `invMatrix`.

/// Whether a good is trade cargo or a gun that can go in a weapon bay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ware,
    Weapon,
}

use super::layout::Layout;
use super::preview::{description_for, display_name, mesh_id};

/// One kind of good, with the prices a shop will honour and the cells
/// it covers when placed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Good {
    pub id: String,
    pub kind: Kind,
    pub buy_price: i32,
    pub sell_price: i32,
    shape: Vec<(i32, i32)>,
}

impl Good {
    pub fn new(id: impl Into<String>, buy_price: i32, sell_price: i32) -> Self {
        Self::with_kind(id, Kind::Ware, buy_price, sell_price)
    }

    pub fn weapon(id: impl Into<String>, buy_price: i32, sell_price: i32) -> Self {
        Self::with_kind(id, Kind::Weapon, buy_price, sell_price)
    }

    fn with_kind(id: impl Into<String>, kind: Kind, buy_price: i32, sell_price: i32) -> Self {
        let id = id.into();
        let shape = default_shape(&id, kind);
        Good {
            id,
            kind,
            buy_price,
            sell_price,
            shape,
        }
    }

    /// Tests and odd sizes: replace the default footprint.
    pub fn with_shape(mut self, cells: Vec<(i32, i32)>) -> Self {
        self.shape = if cells.is_empty() {
            vec![(0, 0)]
        } else {
            cells
        };
        self
    }

    pub fn is_weapon(&self) -> bool {
        self.kind == Kind::Weapon
    }

    /// Cells relative to the origin the player grabbed.
    pub fn shape(&self) -> &[(i32, i32)] {
        &self.shape
    }

    pub fn display_name(&self) -> &str {
        display_name(&self.id)
    }

    /// `game.lst` NameID for the 3D model.
    pub fn mesh_id(&self) -> &str {
        mesh_id(&self.id)
    }
}

fn default_shape(id: &str, kind: Kind) -> Vec<(i32, i32)> {
    match id {
        "Poponka" => vec![(0, 0), (1, 0), (0, 1), (1, 1)],
        "LightLaser" => vec![(0, 0), (1, 0)],
        "LightMissile" => vec![(0, 0), (1, 0), (2, 0)],
        _ if kind == Kind::Weapon => vec![(0, 0), (1, 0)],
        _ => vec![(0, 0)],
    }
}

/// Weapon bays on a mechos, matching the three m3d slots.
pub const BAY_COUNT: usize = 3;

/// A good sitting on the cargo grid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placed {
    pub good: Good,
    pub origin: (i32, i32),
}

impl Placed {
    pub fn covers(&self, cell: (i32, i32)) -> bool {
        self.good
            .shape()
            .iter()
            .any(|&(dx, dy)| (self.origin.0 + dx, self.origin.1 + dy) == cell)
    }
}

/// What the player is carrying. Cargo is the pack; bays are the guns
/// mounted on the vehicle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inventory {
    layout: Layout,
    cargo: Vec<Placed>,
    bays: [Option<Good>; BAY_COUNT],
}

impl Default for Inventory {
    fn default() -> Self {
        Inventory {
            layout: {
                #[cfg(test)]
                {
                    Layout::pack()
                }
                #[cfg(not(test))]
                {
                    Layout::empty()
                }
            },
            cargo: Vec::new(),
            bays: Default::default(),
        }
    }
}

impl Inventory {
    pub fn for_car(name: &str, catalog: &super::layout::Catalog) -> Self {
        Inventory {
            layout: catalog
                .layout_for(name)
                .cloned()
                .unwrap_or_else(Layout::empty),
            cargo: Vec::new(),
            bays: Default::default(),
        }
    }

    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    pub fn cargo(&self) -> &[Placed] {
        &self.cargo
    }

    pub fn items(&self) -> impl Iterator<Item = &Good> {
        self.cargo.iter().map(|p| &p.good)
    }

    pub fn bays(&self) -> &[Option<Good>; BAY_COUNT] {
        &self.bays
    }

    pub fn bay(&self, index: usize) -> Option<&Good> {
        self.bays.get(index).and_then(|slot| slot.as_ref())
    }

    pub fn is_empty(&self) -> bool {
        self.cargo.is_empty() && self.bays.iter().all(Option::is_none)
    }

    pub fn len(&self) -> usize {
        self.cargo.len() + self.bays.iter().filter(|slot| slot.is_some()).count()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.cargo.iter().any(|p| p.good.id == id)
            || self
                .bays
                .iter()
                .any(|slot| slot.as_ref().is_some_and(|g| g.id == id))
    }

    pub fn equipped(&self, id: &str) -> bool {
        self.bays
            .iter()
            .any(|slot| slot.as_ref().is_some_and(|g| g.id == id))
    }

    /// Which placed good owns `cell`, if any.
    pub fn occupant(&self, cell: (i32, i32)) -> Option<(usize, &Placed)> {
        self.cargo.iter().enumerate().find(|&(_, p)| p.covers(cell))
    }

    /// True when every cell of `shape` at `origin` is on the grid and
    /// empty. `ignore` is a cargo index whose cells do not count as busy
    /// (the piece being moved).
    pub fn check_fit(
        &self,
        shape: &[(i32, i32)],
        origin: (i32, i32),
        ignore: Option<usize>,
    ) -> bool {
        for &(dx, dy) in shape {
            let x = origin.0 + dx;
            let y = origin.1 + dy;
            if !self.layout.is_cargo(x, y) {
                return false;
            }
            if let Some((i, _)) = self.occupant((x, y))
                && ignore != Some(i)
            {
                return false;
            }
        }
        true
    }

    pub fn first_fit(&self, shape: &[(i32, i32)]) -> Option<(i32, i32)> {
        for y in 0..self.layout.height {
            for x in 0..self.layout.width {
                if self.check_fit(shape, (x, y), None) {
                    return Some((x, y));
                }
            }
        }
        None
    }

    /// Put `good` at `origin`. Fails without changing the pack when the
    /// footprint is off the grid or overlaps something already there.
    pub fn place(&mut self, good: Good, origin: (i32, i32)) -> Result<(), ShopError> {
        if !self.check_fit(good.shape(), origin, None) {
            return Err(ShopError::NoFit);
        }
        self.cargo.push(Placed { good, origin });
        Ok(())
    }

    pub fn remove(&mut self, index: usize) -> Result<Good, ShopError> {
        if index >= self.cargo.len() {
            return Err(ShopError::EmptyHands);
        }
        Ok(self.cargo.remove(index).good)
    }

    pub fn rearrange(&mut self, index: usize, origin: (i32, i32)) -> Result<(), ShopError> {
        let Some(piece) = self.cargo.get(index) else {
            return Err(ShopError::EmptyHands);
        };
        if !self.check_fit(piece.good.shape(), origin, Some(index)) {
            return Err(ShopError::NoFit);
        }
        self.cargo[index].origin = origin;
        Ok(())
    }

    /// Move a cargo weapon into a bay. Anything already in that bay
    /// goes back to cargo, if it fits.
    pub fn equip(&mut self, cargo_index: usize, bay: usize) -> Result<(), ShopError> {
        if bay >= BAY_COUNT || cargo_index >= self.cargo.len() {
            return Err(ShopError::EmptyHands);
        }
        if self.cargo[cargo_index].good.kind != Kind::Weapon {
            return Err(ShopError::NotAWeapon);
        }
        let saved = self.cargo[cargo_index].origin;
        let weapon = self.cargo.remove(cargo_index).good;
        let old = self.bays[bay].replace(weapon);
        if let Some(old) = old {
            match self.first_fit(old.shape()) {
                Some(origin) => self.cargo.push(Placed { good: old, origin }),
                None => {
                    let weapon = self.bays[bay].replace(old).expect("bay we just filled");
                    self.cargo.insert(
                        cargo_index.min(self.cargo.len()),
                        Placed {
                            good: weapon,
                            origin: saved,
                        },
                    );
                    return Err(ShopError::NoFit);
                }
            }
        }
        Ok(())
    }

    pub fn unequip(&mut self, bay: usize) -> Result<(), ShopError> {
        let origin = {
            let good = self
                .bays
                .get(bay)
                .and_then(Option::as_ref)
                .ok_or(ShopError::EmptyHands)?;
            self.first_fit(good.shape()).ok_or(ShopError::NoFit)?
        };
        self.unequip_at(bay, origin)
    }

    pub fn unequip_at(&mut self, bay: usize, origin: (i32, i32)) -> Result<(), ShopError> {
        let good = self
            .bays
            .get(bay)
            .and_then(Option::as_ref)
            .ok_or(ShopError::EmptyHands)?;
        if !self.check_fit(good.shape(), origin, None) {
            return Err(ShopError::NoFit);
        }
        let good = self.bays[bay].take().expect("checked");
        self.cargo.push(Placed { good, origin });
        Ok(())
    }

    /// Spawn loadout: put a weapon straight into a bay.
    pub fn load_bay(&mut self, bay: usize, good: Good) -> Result<(), ShopError> {
        if bay >= BAY_COUNT {
            return Err(ShopError::EmptyHands);
        }
        if good.kind != Kind::Weapon {
            return Err(ShopError::NotAWeapon);
        }
        self.bays[bay] = Some(good);
        Ok(())
    }
}

/// Slot ids to hang on the mechos, in bay order. Empty bays are `None`.
pub fn equipped_slot_ids(inventory: &Inventory) -> [Option<&str>; BAY_COUNT] {
    let mut ids = [None; BAY_COUNT];
    for (i, slot) in inventory.bays.iter().enumerate() {
        ids[i] = slot
            .as_ref()
            .filter(|g| g.is_weapon())
            .map(|g| g.id.as_str());
    }
    ids
}

/// Resolve each bay to a drawable, so the mechos slots can hang the gun.
pub fn mounted_meshes<M>(
    inventory: &Inventory,
    mut mesh_of: impl FnMut(&str) -> Option<M>,
) -> [Option<M>; BAY_COUNT] {
    equipped_slot_ids(inventory).map(|id| id.and_then(&mut mesh_of))
}

/// A shop's stock. Buying takes a good out of here and into the inventory;
/// selling puts it back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shop {
    stock: Vec<Good>,
}

/// Why a buy or a sell was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShopError {
    UnknownGood,
    OutOfStock,
    TooPoor,
    EmptyHands,
    NotAWeapon,
    NoFit,
}

impl std::fmt::Display for ShopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let text = match *self {
            ShopError::UnknownGood => "Not sold here",
            ShopError::OutOfStock => "Out of stock",
            ShopError::TooPoor => "Not enough beebs",
            ShopError::EmptyHands => "Nothing there",
            ShopError::NotAWeapon => "Not a gun",
            ShopError::NoFit => "No room",
        };
        f.write_str(text)
    }
}

/// Something the pointer is holding.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hand {
    Shop { id: String },
    Cargo { index: usize },
    Bay { index: usize },
}

/// Where a held good was released.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DropTarget {
    Shop,
    Cargo { origin: (i32, i32) },
    Bay { index: usize },
}

/// Stats shown when a good is selected. Prices match the Fostral counter
/// for catalog ids; a live `Good` can override them via [`preview_good`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub buy_price: i32,
    pub sell_price: i32,
    pub description: String,
}

pub fn preview_good(good: &Good) -> Preview {
    Preview {
        id: good.id.clone(),
        name: display_name(&good.id).to_string(),
        kind: good.kind,
        buy_price: good.buy_price,
        sell_price: good.sell_price,
        description: description_for(&good.id).to_string(),
    }
}

/// Catalog lookup by id. Unknown names yield `None`.
pub fn preview(id: &str) -> Option<Preview> {
    Shop::fostral()
        .stock()
        .iter()
        .find(|g| g.id == id)
        .map(preview_good)
}

impl Shop {
    pub fn new(stock: Vec<Good>) -> Self {
        Shop { stock }
    }

    /// The wares a Fostral escave puts on the counter. Buy prices sit above
    /// sell prices so a round trip costs beebs, the way the original's
    /// `price` / `sell_price` pair does.
    pub fn fostral() -> Self {
        Shop::new(vec![
            Good::new("Nymbos", 12, 6),
            Good::new("Phlegma", 20, 10),
            Good::new("Heroin", 28, 14),
            Good::new("Shrub", 24, 12),
            Good::new("Poponka", 40, 20),
            Good::new("Toxick", 16, 8),
            Good::weapon("LightLaser", 50, 25),
            Good::weapon("LightMissile", 80, 40),
        ])
    }

    pub fn stock(&self) -> &[Good] {
        &self.stock
    }

    /// Purchase `id` if the player has the beebs and a free footprint.
    /// On success the good moves into the inventory and credits drop by
    /// the buy price. On failure nothing on either side changes.
    pub fn buy(
        &mut self,
        id: &str,
        inventory: &mut Inventory,
        credits: &mut i32,
    ) -> Result<(), ShopError> {
        let origin = {
            let good = self
                .stock
                .iter()
                .find(|g| g.id == id)
                .ok_or(ShopError::OutOfStock)?;
            inventory.first_fit(good.shape()).ok_or(ShopError::NoFit)?
        };
        self.buy_at(id, inventory, credits, origin)
    }

    /// Purchase onto a chosen origin cell. Same no-op-on-failure rule.
    pub fn buy_at(
        &mut self,
        id: &str,
        inventory: &mut Inventory,
        credits: &mut i32,
        origin: (i32, i32),
    ) -> Result<(), ShopError> {
        let index = self
            .stock
            .iter()
            .position(|g| g.id == id)
            .ok_or(ShopError::OutOfStock)?;
        let price = self.stock[index].buy_price;
        if *credits < price {
            return Err(ShopError::TooPoor);
        }
        if !inventory.check_fit(self.stock[index].shape(), origin, None) {
            return Err(ShopError::NoFit);
        }
        let good = self.stock.remove(index);
        *credits -= price;
        inventory.place(good, origin).expect("fit already checked");
        Ok(())
    }

    /// Sell the cargo slot `index` back to the shop. Credits go up by
    /// that good's sell price.
    pub fn sell(
        &mut self,
        index: usize,
        inventory: &mut Inventory,
        credits: &mut i32,
    ) -> Result<(), ShopError> {
        let good = inventory.remove(index)?;
        *credits += good.sell_price;
        self.stock.push(good);
        Ok(())
    }

    /// Sell the first cargo item with this id, if any.
    pub fn sell_id(
        &mut self,
        id: &str,
        inventory: &mut Inventory,
        credits: &mut i32,
    ) -> Result<(), ShopError> {
        let index = inventory
            .cargo
            .iter()
            .position(|p| p.good.id == id)
            .ok_or(ShopError::EmptyHands)?;
        self.sell(index, inventory, credits)
    }
}

/// Drop a held good onto a shop, cargo cell, or weapon bay.
/// Failures leave shop, pack, bays, and credits unchanged.
pub fn drop_held(
    shop: &mut Shop,
    inventory: &mut Inventory,
    credits: &mut i32,
    hand: Hand,
    target: DropTarget,
) -> Result<(), ShopError> {
    match (hand, target) {
        (Hand::Shop { id }, DropTarget::Cargo { origin }) => {
            shop.buy_at(&id, inventory, credits, origin)
        }
        (Hand::Shop { id }, DropTarget::Bay { index }) => {
            buy_to_bay(shop, inventory, credits, &id, index)
        }
        (Hand::Shop { .. }, DropTarget::Shop) => Ok(()),
        (Hand::Cargo { index }, DropTarget::Shop) => shop.sell(index, inventory, credits),
        (Hand::Cargo { index }, DropTarget::Cargo { origin }) => inventory.rearrange(index, origin),
        (Hand::Cargo { index }, DropTarget::Bay { index: bay }) => inventory.equip(index, bay),
        (Hand::Bay { index }, DropTarget::Cargo { origin }) => inventory.unequip_at(index, origin),
        (Hand::Bay { index }, DropTarget::Shop) => sell_bay(shop, inventory, credits, index),
        (Hand::Bay { index }, DropTarget::Bay { index: dest }) => swap_bays(inventory, index, dest),
    }
}

fn buy_to_bay(
    shop: &mut Shop,
    inventory: &mut Inventory,
    credits: &mut i32,
    id: &str,
    bay: usize,
) -> Result<(), ShopError> {
    if bay >= BAY_COUNT {
        return Err(ShopError::EmptyHands);
    }
    let index = shop
        .stock
        .iter()
        .position(|g| g.id == id)
        .ok_or(ShopError::OutOfStock)?;
    if shop.stock[index].kind != Kind::Weapon {
        return Err(ShopError::NotAWeapon);
    }
    let price = shop.stock[index].buy_price;
    if *credits < price {
        return Err(ShopError::TooPoor);
    }
    if let Some(ref old) = inventory.bays[bay]
        && inventory.first_fit(old.shape()).is_none()
    {
        return Err(ShopError::NoFit);
    }
    let weapon = shop.stock.remove(index);
    *credits -= price;
    if let Some(old) = inventory.bays[bay].replace(weapon) {
        let origin = inventory.first_fit(old.shape()).expect("checked");
        inventory.cargo.push(Placed { good: old, origin });
    }
    Ok(())
}

fn sell_bay(
    shop: &mut Shop,
    inventory: &mut Inventory,
    credits: &mut i32,
    bay: usize,
) -> Result<(), ShopError> {
    let good = inventory
        .bays
        .get_mut(bay)
        .and_then(Option::take)
        .ok_or(ShopError::EmptyHands)?;
    *credits += good.sell_price;
    shop.stock.push(good);
    Ok(())
}

fn swap_bays(inventory: &mut Inventory, a: usize, b: usize) -> Result<(), ShopError> {
    if a >= BAY_COUNT || b >= BAY_COUNT {
        return Err(ShopError::EmptyHands);
    }
    inventory.bays.swap(a, b);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nymbos() -> Good {
        Good::new("Nymbos", 10, 6)
    }

    #[test]
    fn buying_with_enough_beebs_moves_the_good_and_debits() {
        let mut shop = Shop::new(vec![nymbos()]);
        let mut inv = Inventory::default();
        let mut credits = 100;
        shop.buy("Nymbos", &mut inv, &mut credits).unwrap();
        assert_eq!(credits, 90);
        assert!(inv.contains("Nymbos"));
        assert!(shop.stock().is_empty());
        assert_eq!(inv.cargo()[0].origin, (0, 0));
    }

    #[test]
    fn selling_credits_the_sell_price_and_returns_the_good() {
        let mut shop = Shop::new(vec![nymbos()]);
        let mut inv = Inventory::default();
        let mut credits = 100;
        shop.buy("Nymbos", &mut inv, &mut credits).unwrap();
        shop.sell(0, &mut inv, &mut credits).unwrap();
        assert_eq!(credits, 96, "buy 10, sell 6");
        assert!(inv.is_empty());
        assert_eq!(shop.stock()[0].id, "Nymbos");
    }

    #[test]
    fn buying_with_too_few_beebs_changes_nothing() {
        let mut shop = Shop::new(vec![nymbos()]);
        let mut inv = Inventory::default();
        let mut credits = 5;
        let err = shop.buy("Nymbos", &mut inv, &mut credits).unwrap_err();
        assert_eq!(err, ShopError::TooPoor);
        assert_eq!(credits, 5);
        assert!(inv.is_empty());
        assert_eq!(shop.stock().len(), 1);
    }

    #[test]
    fn too_poor_is_a_readable_refusal() {
        assert_eq!(ShopError::TooPoor.to_string(), "Not enough beebs");
    }

    #[test]
    fn fostral_stock_has_a_ware_and_a_gun() {
        let shop = Shop::fostral();
        assert!(
            shop.stock()
                .iter()
                .any(|g| g.id == "Nymbos" && g.kind == Kind::Ware),
            "expected Nymbos on the counter"
        );
        assert!(
            shop.stock()
                .iter()
                .any(|g| g.id == "LightLaser" && g.kind == Kind::Weapon),
            "expected LightLaser on the counter"
        );
    }

    #[test]
    fn buying_a_weapon_debits_like_a_ware() {
        let mut shop = Shop::fostral();
        let mut inv = Inventory::default();
        let mut credits = 200;
        let price = shop
            .stock()
            .iter()
            .find(|g| g.id == "LightLaser")
            .unwrap()
            .buy_price;
        shop.buy("LightLaser", &mut inv, &mut credits).unwrap();
        assert_eq!(credits, 200 - price);
        assert!(inv.contains("LightLaser"));
        assert!(!shop.stock().iter().any(|g| g.id == "LightLaser"));
    }

    #[test]
    fn a_bought_weapon_can_be_equipped() {
        let mut shop = Shop::fostral();
        let mut inv = Inventory::default();
        let mut credits = 200;
        shop.buy("LightLaser", &mut inv, &mut credits).unwrap();
        inv.equip(0, 0).unwrap();
        assert!(inv.equipped("LightLaser"));
        assert_eq!(inv.bay(0).map(|g| g.id.as_str()), Some("LightLaser"));
        assert!(inv.cargo().is_empty());
        assert_eq!(equipped_slot_ids(&inv)[0], Some("LightLaser"));
        let meshes = mounted_meshes(&inv, |id| Some(id.to_string()));
        assert_eq!(meshes[0].as_deref(), Some("LightLaser"));
        assert!(meshes[1].is_none());
    }

    #[test]
    fn a_weapon_left_in_cargo_is_not_equipped() {
        let mut shop = Shop::fostral();
        let mut inv = Inventory::default();
        let mut credits = 200;
        shop.buy("LightLaser", &mut inv, &mut credits).unwrap();
        assert!(!inv.equipped("LightLaser"));
        assert!(inv.bay(0).is_none());
        assert_eq!(equipped_slot_ids(&inv), [None, None, None]);
    }

    #[test]
    fn a_ware_cannot_go_in_a_weapon_bay() {
        let mut shop = Shop::fostral();
        let mut inv = Inventory::default();
        let mut credits = 200;
        shop.buy("Nymbos", &mut inv, &mut credits).unwrap();
        assert_eq!(inv.equip(0, 0).unwrap_err(), ShopError::NotAWeapon);
        assert!(inv.bay(0).is_none());
        assert!(inv.contains("Nymbos"));
    }

    #[test]
    fn a_missing_ware_is_out_of_stock() {
        let mut shop = Shop::new(vec![nymbos()]);
        let mut inv = Inventory::default();
        let mut credits = 100;
        assert_eq!(
            shop.buy("Phlegma", &mut inv, &mut credits).unwrap_err(),
            ShopError::OutOfStock
        );
        assert_eq!(credits, 100);
    }

    #[test]
    fn a_shape_that_goes_out_of_bounds_is_rejected() {
        let mut inv = Inventory::default();
        let missile = Good::weapon("LightMissile", 80, 40);
        let err = inv.place(missile.clone(), (6, 0)).unwrap_err();
        assert_eq!(err, ShopError::NoFit);
        assert!(inv.is_empty());
        assert!(
            !inv.check_fit(missile.shape(), (6, 0), None),
            "cells 6,7,8: 8 is off an 8-wide pack"
        );
    }

    #[test]
    fn overlapping_place_leaves_the_grid_unchanged() {
        let mut inv = Inventory::default();
        inv.place(nymbos(), (0, 0)).unwrap();
        let err = inv.place(Good::new("Phlegma", 20, 10), (0, 0)).unwrap_err();
        assert_eq!(err, ShopError::NoFit);
        assert_eq!(inv.cargo().len(), 1);
        assert_eq!(inv.cargo()[0].good.id, "Nymbos");
    }

    #[test]
    fn a_valid_rearrange_changes_cell_occupancy() {
        let mut shop = Shop::new(vec![nymbos()]);
        let mut inv = Inventory::default();
        let mut credits = 100;
        drop_held(
            &mut shop,
            &mut inv,
            &mut credits,
            Hand::Shop {
                id: "Nymbos".into(),
            },
            DropTarget::Cargo { origin: (0, 0) },
        )
        .unwrap();
        drop_held(
            &mut shop,
            &mut inv,
            &mut credits,
            Hand::Cargo { index: 0 },
            DropTarget::Cargo { origin: (3, 2) },
        )
        .unwrap();
        assert!(inv.occupant((0, 0)).is_none());
        assert_eq!(
            inv.occupant((3, 2)).map(|(_, p)| p.good.id.as_str()),
            Some("Nymbos")
        );
        assert_eq!(inv.cargo()[0].origin, (3, 2));
        assert_eq!(credits, 90);
    }

    #[test]
    fn dropping_shop_stock_onto_cargo_buys_at_that_cell() {
        let mut shop = Shop::fostral();
        let mut inv = Inventory::default();
        let mut credits = 100;
        drop_held(
            &mut shop,
            &mut inv,
            &mut credits,
            Hand::Shop {
                id: "Nymbos".into(),
            },
            DropTarget::Cargo { origin: (2, 1) },
        )
        .unwrap();
        assert_eq!(credits, 88);
        assert_eq!(inv.cargo()[0].origin, (2, 1));
        assert!(!shop.stock().iter().any(|g| g.id == "Nymbos"));
    }

    #[test]
    fn too_poor_buy_on_place_is_a_no_op() {
        let mut shop = Shop::fostral();
        let mut inv = Inventory::default();
        let mut credits = 1;
        let err = drop_held(
            &mut shop,
            &mut inv,
            &mut credits,
            Hand::Shop {
                id: "Nymbos".into(),
            },
            DropTarget::Cargo { origin: (0, 0) },
        )
        .unwrap_err();
        assert_eq!(err, ShopError::TooPoor);
        assert_eq!(credits, 1);
        assert!(inv.is_empty());
        assert!(shop.stock().iter().any(|g| g.id == "Nymbos"));
    }

    #[test]
    fn dropping_cargo_onto_the_shop_sells() {
        let mut shop = Shop::fostral();
        let mut inv = Inventory::default();
        let mut credits = 100;
        shop.buy("Nymbos", &mut inv, &mut credits).unwrap();
        drop_held(
            &mut shop,
            &mut inv,
            &mut credits,
            Hand::Cargo { index: 0 },
            DropTarget::Shop,
        )
        .unwrap();
        assert_eq!(credits, 94, "buy 12, sell 6");
        assert!(inv.cargo().is_empty());
        assert!(shop.stock().iter().any(|g| g.id == "Nymbos"));
    }

    #[test]
    fn dropping_a_weapon_on_a_bay_equips_it() {
        let mut shop = Shop::fostral();
        let mut inv = Inventory::default();
        let mut credits = 200;
        shop.buy("LightLaser", &mut inv, &mut credits).unwrap();
        drop_held(
            &mut shop,
            &mut inv,
            &mut credits,
            Hand::Cargo { index: 0 },
            DropTarget::Bay { index: 1 },
        )
        .unwrap();
        assert!(inv.equipped("LightLaser"));
        assert_eq!(inv.bay(1).map(|g| g.id.as_str()), Some("LightLaser"));
        assert!(inv.cargo().is_empty());
    }

    #[test]
    fn an_invalid_drop_changes_nothing() {
        let mut shop = Shop::fostral();
        let mut inv = Inventory::default();
        let mut credits = 200;
        shop.buy("Nymbos", &mut inv, &mut credits).unwrap();
        let before = (credits, inv.clone(), shop.clone());
        let err = drop_held(
            &mut shop,
            &mut inv,
            &mut credits,
            Hand::Cargo { index: 0 },
            DropTarget::Bay { index: 0 },
        )
        .unwrap_err();
        assert_eq!(err, ShopError::NotAWeapon);
        assert_eq!(credits, before.0);
        assert_eq!(inv, before.1);
        assert_eq!(shop, before.2);
    }

    #[test]
    fn nymbos_preview_has_price_kind_and_description() {
        let p = preview("Nymbos").expect("Nymbos is a Fostral ware");
        assert_eq!(p.id, "Nymbos");
        assert_eq!(p.name, "Nymbos");
        assert_eq!(p.kind, Kind::Ware);
        assert_eq!(p.buy_price, 12);
        assert_eq!(p.sell_price, 6);
        assert_eq!(p.description, "Some eleepods' stuff from Podish");
        let live = preview_good(&Good::new("Nymbos", 12, 6));
        assert_eq!(live.kind, p.kind);
        assert_eq!(live.buy_price, p.buy_price);
        assert_eq!(live.description, p.description);
        let gun = preview("LightLaser").unwrap();
        assert_eq!(gun.name, "MacHOTine Gun");
    }

    #[test]
    fn oxidize_monk_cargo_sits_on_the_mechos_not_the_corner() {
        let cat = crate::escave::Catalog::load(std::path::Path::new("../Vangers/data"));
        let mut inv = Inventory::for_car("OxidizeMonk", &cat);
        if inv.layout().height == 0 {
            return;
        }
        assert!(!inv.check_fit(&[(0, 0)], (0, 0), None));
        inv.place(nymbos(), (3, 7)).unwrap();
        assert_eq!(inv.cargo()[0].origin, (3, 7));
        assert_eq!(inv.first_fit(&[(0, 0)]), Some((2, 7)));
    }
}
