use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::msg::Payment;
use cosmwasm_std::{Addr, StdResult, Storage};
use cw_storage_plus::{Item, Map, U64Key};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct Config {
    pub owner: Addr,
    pub enabled: bool,
}

pub const CONFIG: Item<Config> = Item::new("config");

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, JsonSchema)]
pub struct PaymentState {
    pub payment: Payment,
    pub paid: bool,
    pub id: u64,
}

pub const PAYMENT_COUNT: Item<u64> = Item::new("proposal_count");

// multiple-item map
pub const PAYMENTS: Map<U64Key, PaymentState> = Map::new("payments");

pub fn next_id(store: &mut dyn Storage) -> StdResult<u64> {
    let id: u64 = PAYMENT_COUNT.may_load(store)?.unwrap_or_default() + 1;
    PAYMENT_COUNT.save(store, &id)?;
    Ok(id)
}
