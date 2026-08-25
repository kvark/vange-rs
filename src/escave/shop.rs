//! Buy and sell against beeb credits.
//!
//! A working inventory plus prices, not the original tetris item matrix.
//! Names of the default wares come from `actintItemTypes` (Nymbos, Phlegma,
//! and the other Fostral trade goods).

/// One kind of good, with the prices a shop will honour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Good {
    pub id: String,
    pub buy_price: i32,
    pub sell_price: i32,
}

impl Good {
    pub fn new(id: impl Into<String>, buy_price: i32, sell_price: i32) -> Self {
        Good {
            id: id.into(),
            buy_price,
            sell_price,
        }
    }
}

/// What the player is carrying. Order is insertion order; selling uses an
/// index into this list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Inventory {
    items: Vec<Good>,
}

impl Inventory {
    pub fn items(&self) -> &[Good] {
        &self.items
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.items.iter().any(|g| g.id == id)
    }
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
        inventory.items.push(good);
        Ok(())
    }

    /// Sell the inventory slot `index` back to the shop. Credits go up by
    /// that good's sell price.
    pub fn sell(
        &mut self,
        index: usize,
        inventory: &mut Inventory,
        credits: &mut i32,
    ) -> Result<(), ShopError> {
        if index >= inventory.items.len() {
            return Err(ShopError::EmptyHands);
        }
        let good = inventory.items.remove(index);
        *credits += good.sell_price;
        self.stock.push(good);
        Ok(())
    }

    /// Sell the first inventory item with this id, if any.
    pub fn sell_id(
        &mut self,
        id: &str,
        inventory: &mut Inventory,
        credits: &mut i32,
    ) -> Result<(), ShopError> {
        let index = inventory
            .items
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
