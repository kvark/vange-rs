//! Buy and sell against beeb credits.
//!
//! A working inventory plus prices, not the original tetris item matrix.
//! Names of the default wares come from `actintItemTypes` (Nymbos, Phlegma,
//! and the other Fostral trade goods). Weapon ids match `game.lst`.

/// Whether a good is trade cargo or a gun that can go in a weapon bay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ware,
    Weapon,
}

/// One kind of good, with the prices a shop will honour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Good {
    pub id: String,
    pub kind: Kind,
    pub buy_price: i32,
    pub sell_price: i32,
}

impl Good {
    pub fn new(id: impl Into<String>, buy_price: i32, sell_price: i32) -> Self {
        Self::with_kind(id, Kind::Ware, buy_price, sell_price)
    }

    pub fn weapon(id: impl Into<String>, buy_price: i32, sell_price: i32) -> Self {
        Self::with_kind(id, Kind::Weapon, buy_price, sell_price)
    }

    fn with_kind(id: impl Into<String>, kind: Kind, buy_price: i32, sell_price: i32) -> Self {
        Good {
            id: id.into(),
            kind,
            buy_price,
            sell_price,
        }
    }

    pub fn is_weapon(&self) -> bool {
        self.kind == Kind::Weapon
    }
}

/// Weapon bays on a mechos, matching the three m3d slots.
pub const BAY_COUNT: usize = 3;

/// What the player is carrying. Cargo is the pack; bays are the guns
/// mounted on the vehicle.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    cargo: Vec<Good>,
    bays: [Option<Good>; BAY_COUNT],
}

impl Inventory {
    pub fn items(&self) -> &[Good] {
        &self.cargo
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
        self.cargo.iter().any(|g| g.id == id)
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

    /// Move a cargo weapon into a bay. Anything already in that bay
    /// goes back to cargo.
    pub fn equip(&mut self, cargo_index: usize, bay: usize) -> Result<(), ShopError> {
        if bay >= BAY_COUNT || cargo_index >= self.cargo.len() {
            return Err(ShopError::EmptyHands);
        }
        if self.cargo[cargo_index].kind != Kind::Weapon {
            return Err(ShopError::NotAWeapon);
        }
        let weapon = self.cargo.remove(cargo_index);
        if let Some(old) = self.bays[bay].replace(weapon) {
            self.cargo.push(old);
        }
        Ok(())
    }

    pub fn unequip(&mut self, bay: usize) -> Result<(), ShopError> {
        let weapon = self
            .bays
            .get_mut(bay)
            .and_then(Option::take)
            .ok_or(ShopError::EmptyHands)?;
        self.cargo.push(weapon);
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

    /// Purchase `id` if the player has the beebs. On success the good moves
    /// into the inventory and credits drop by the buy price. On failure
    /// nothing on either side changes.
    pub fn buy(
        &mut self,
        id: &str,
        inventory: &mut Inventory,
        credits: &mut i32,
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
        let good = self.stock.remove(index);
        *credits -= price;
        inventory.cargo.push(good);
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
        if index >= inventory.cargo.len() {
            return Err(ShopError::EmptyHands);
        }
        let good = inventory.cargo.remove(index);
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
            .position(|g| g.id == id)
            .ok_or(ShopError::EmptyHands)?;
        self.sell(index, inventory, credits)
    }
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
        assert!(inv.items().is_empty());
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
}
