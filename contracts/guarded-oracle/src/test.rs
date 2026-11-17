#![cfg(test)]

use crate::{aggregate, Asset, GuardedOracle, GuardedOracleClient, PriceData};
use soroban_sdk::{
    contract, contractimpl, contracttype, testutils::Address as _, testutils::Ledger, vec, Address,
    Env, Symbol, Vec,
};

// ---------------------------------------------------------------- mock upstream

#[contracttype]
enum MockKey {
    Decimals,
    Price(Asset),
}

#[contract]
pub struct MockFeed;

#[contractimpl]
impl MockFeed {
    pub fn __constructor(env: Env, decimals: u32) {
        env.storage().instance().set(&MockKey::Decimals, &decimals);
    }
    pub fn set(env: Env, asset: Asset, price: i128, timestamp: u64) {
        env.storage()
            .instance()
            .set(&MockKey::Price(asset), &PriceData { price, timestamp });
    }
    pub fn decimals(env: Env) -> u32 {
        env.storage().instance().get(&MockKey::Decimals).unwrap()
    }
    pub fn lastprice(env: Env, asset: Asset) -> Option<PriceData> {
        env.storage().instance().get(&MockKey::Price(asset))
    }
}

#[contract]
pub struct RevertingFeed;

#[contractimpl]
impl RevertingFeed {
    pub fn decimals(_env: Env) -> u32 {
        7
    }
    pub fn lastprice(_env: Env, _asset: Asset) -> Option<PriceData> {
        panic!("upstream down")
    }
}

// ---------------------------------------------------------------- helpers

const NOW: u64 = 1_800_000_000;
const MAX_AGE: u64 = 900;

struct Setup<'a> {
    env: Env,
    oracle: GuardedOracleClient<'a>,
    asset: Asset,
}

fn setup(min_sources: u32) -> Setup<'static> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);

    let admin = Address::generate(&env);
    let base = Asset::Other(Symbol::new(&env, "USD"));
    let id = env.register(
        GuardedOracle,
        (admin, base, 7u32, 300u32, MAX_AGE, min_sources),
    );
    let oracle = GuardedOracleClient::new(&env, &id);
    let asset = Asset::Stellar(Address::generate(&env));
    Setup { env, oracle, asset }
}

fn feed(env: &Env, decimals: u32) -> MockFeedClient<'static> {
    let id = env.register(MockFeed, (decimals,));
    MockFeedClient::new(env, &id)
}

// ---------------------------------------------------------------- tests

#[test]
fn median_of_three_ignores_single_outlier() {
    let s = setup(2);
    let (a, b, c) = (feed(&s.env, 7), feed(&s.env, 7), feed(&s.env, 7));
    a.set(&s.asset, &10_600_000, &NOW);
    b.set(&s.asset, &10_700_000, &NOW);
    // the YieldBlox shape: one feed reports ~100x
    c.set(&s.asset, &1_067_000_000, &NOW);
    s.oracle.set_sources(
        &s.asset,
        &vec![
            &s.env,
            (a.address.clone(), s.asset.clone()),
            (b.address.clone(), s.asset.clone()),
            (c.address.clone(), s.asset.clone()),
        ],
    );
    let p = s.oracle.lastprice(&s.asset).unwrap();
    assert_eq!(p.price, 10_700_000);
}

#[test]
fn stale_and_future_prices_are_ignored() {
    let s = setup(2);
    let (a, b, c) = (feed(&s.env, 7), feed(&s.env, 7), feed(&s.env, 7));
    a.set(&s.asset, &100, &NOW);
    b.set(&s.asset, &200, &(NOW - MAX_AGE - 1));
    c.set(&s.asset, &300, &(NOW + 1));
    s.oracle.set_sources(
        &s.asset,
        &vec![
            &s.env,
            (a.address.clone(), s.asset.clone()),
            (b.address.clone(), s.asset.clone()),
            (c.address.clone(), s.asset.clone()),
        ],
    );
    // only one fresh source, quorum is two
    assert_eq!(s.oracle.lastprice(&s.asset), None);
}

#[test]
fn reverting_upstream_is_skipped() {
    let s = setup(2);
    let (a, b) = (feed(&s.env, 7), feed(&s.env, 7));
    let bad = s.env.register(RevertingFeed, ());
    a.set(&s.asset, &100, &(NOW - 10));
    b.set(&s.asset, &120, &NOW);
    s.oracle.set_sources(
        &s.asset,
        &vec![
            &s.env,
            (a.address.clone(), s.asset.clone()),
            (bad, s.asset.clone()),
            (b.address.clone(), s.asset.clone()),
        ],
    );
    let p = s.oracle.lastprice(&s.asset).unwrap();
    assert_eq!(p.price, 110);
    assert_eq!(p.timestamp, NOW - 10, "timestamp is the oldest price used");
}

#[test]
fn upstream_decimals_are_normalized() {
    let s = setup(2);
    let (a, b) = (feed(&s.env, 14), feed(&s.env, 7));
    a.set(&s.asset, &1_0000000_0000000, &NOW); // 1.0 at 14 decimals
    b.set(&s.asset, &1_0000000, &NOW); // 1.0 at 7 decimals
    s.oracle.set_sources(
        &s.asset,
        &vec![
            &s.env,
            (a.address.clone(), s.asset.clone()),
            (b.address.clone(), s.asset.clone()),
        ],
    );
    assert_eq!(s.oracle.lastprice(&s.asset).unwrap().price, 1_0000000);
}

#[test]
fn unknown_asset_has_no_price() {
    let s = setup(1);
    assert_eq!(s.oracle.lastprice(&s.asset), None);
}

#[test]
#[should_panic]
fn fewer_sources_than_quorum_is_rejected() {
    let s = setup(3);
    let a = feed(&s.env, 7);
    s.oracle.set_sources(
        &s.asset,
        &vec![&s.env, (a.address.clone(), s.asset.clone())],
    );
}

#[test]
fn median_math() {
    let env = Env::default();
    let v: Vec<i128> = vec![&env, 5, 1, 4, 2, 3];
    assert_eq!(aggregate::median(&v), Some(3));
    let v: Vec<i128> = vec![&env, 4, 1, 3, 2];
    assert_eq!(aggregate::median(&v), Some(2));
    assert_eq!(aggregate::median(&Vec::new(&env)), None);
    assert_eq!(aggregate::normalize(1, 7, 7), Some(1));
    assert_eq!(aggregate::normalize(15, 8, 7), Some(1));
    assert_eq!(aggregate::normalize(9, 8, 7), None, "rounds to zero");
    assert_eq!(aggregate::normalize(i128::MAX, 0, 18), None, "overflow");
}
