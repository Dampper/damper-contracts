#![no_std]
//! A SEP-40 price feed whose prices an admin sets by hand.
//!
//! Testnet tooling only: it lets the keeper and the guarded oracle be exercised end to end
//! against prices we control, including a simulated pump. It must never back a real pool.

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Asset {
    Stellar(Address),
    Other(Symbol),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceData {
    pub price: i128,
    pub timestamp: u64,
}

#[contracttype]
enum Key {
    Admin,
    Decimals,
    Price(Asset),
}

#[contract]
pub struct MockFeed;

#[contractimpl]
impl MockFeed {
    pub fn __constructor(env: Env, admin: Address, decimals: u32) {
        env.storage().instance().set(&Key::Admin, &admin);
        env.storage().instance().set(&Key::Decimals, &decimals);
    }

    /// Publish `price` for `asset` at the current ledger time.
    pub fn set_price(env: Env, asset: Asset, price: i128) {
        let admin: Address = env.storage().instance().get(&Key::Admin).unwrap();
        admin.require_auth();
        let data = PriceData {
            price,
            timestamp: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&Key::Price(asset), &data);
    }

    pub fn decimals(env: Env) -> u32 {
        env.storage().instance().get(&Key::Decimals).unwrap()
    }

    pub fn lastprice(env: Env, asset: Asset) -> Option<PriceData> {
        env.storage().persistent().get(&Key::Price(asset))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn set_and_read() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(MockFeed, (Address::generate(&env), 7u32));
        let c = MockFeedClient::new(&env, &id);
        let a = Asset::Other(Symbol::new(&env, "XLM"));
        assert_eq!(c.lastprice(&a), None);
        c.set_price(&a, &1_600_000);
        assert_eq!(c.lastprice(&a).unwrap().price, 1_600_000);
        assert_eq!(c.decimals(), 7);
    }
}
