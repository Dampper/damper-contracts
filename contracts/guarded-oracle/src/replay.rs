#![cfg(test)]
//! Replay of the YieldBlox exploit (2026-02-22) against different oracle setups.
//!
//! The pool below is not Blend, it is a minimal lending pool that does the one thing that
//! mattered: value collateral with `oracle.lastprice(asset).unwrap()` and allow a borrow
//! while `debt * liability_factor <= collateral * collateral_factor`. That is the same
//! call and the same check Blend V2 makes (`pool/src/pool/pool.rs`, `load_price`).
//!
//! Price path from the incident: USTRY traded near $1.06 and was pushed to $106.74 on a
//! market with almost no volume. Amounts are illustrative but sized like the incident:
//! about 61.2M XLM and 1M USDC (roughly $10.8M) were borrowed against USTRY collateral.

use crate::test::{feed, MockFeedClient, MAX_AGE, NOW};
use crate::{
    Asset, GuardedOracle, GuardedOracleClient, Policy, PriceData, UpstreamClient, UNBOUNDED,
};
use soroban_sdk::{
    contract, contractimpl, contracttype, testutils::Address as _, testutils::Ledger, vec, Address,
    Env, Symbol,
};

const SCALAR: i128 = 10_000_000; // 7 decimals everywhere

const USTRY_REAL: i128 = 10_600_000; // $1.06
const USTRY_NAV: i128 = 10_800_000; // $1.08, an issuer-published NAV style feed
const USTRY_PUMPED: i128 = 1_067_372_828; // $106.7372828, the price left resting on the SDEX book
const XLM_PRICE: i128 = 1_600_000; // $0.16
const USDC_PRICE: i128 = SCALAR; // $1

const XLM_LIQUIDITY: i128 = 61_200_000 * SCALAR;
const USDC_LIQUIDITY: i128 = 1_000_000 * SCALAR;
const ATTACKER_USTRY: i128 = 125_000 * SCALAR; // worth about $132k at the real price

const COLLATERAL_FACTOR_BPS: i128 = 9_000;
const LIABILITY_FACTOR_BPS: i128 = 11_000;

// ------------------------------------------------------------------ mock lending pool

#[contracttype]
enum PoolKey {
    Oracle,
    Collateral,
    Deposited,
    DebtValue,
    Liquidity(Address),
    Price(Address),
}

#[contract]
pub struct MockPool;

#[contractimpl]
impl MockPool {
    pub fn __constructor(env: Env, oracle: Address, collateral: Asset) {
        env.storage().instance().set(&PoolKey::Oracle, &oracle);
        env.storage()
            .instance()
            .set(&PoolKey::Collateral, &collateral);
    }

    /// Debt assets are priced honestly here; the incident was a collateral-side attack.
    pub fn list(env: Env, asset: Address, price: i128, liquidity: i128) {
        env.storage()
            .instance()
            .set(&PoolKey::Price(asset.clone()), &price);
        env.storage()
            .instance()
            .set(&PoolKey::Liquidity(asset), &liquidity);
    }

    pub fn deposit(env: Env, amount: i128) {
        env.storage().instance().set(&PoolKey::Deposited, &amount);
    }

    pub fn borrow(env: Env, asset: Address, amount: i128) {
        let oracle: Address = env.storage().instance().get(&PoolKey::Oracle).unwrap();
        let collateral: Asset = env.storage().instance().get(&PoolKey::Collateral).unwrap();
        let deposited: i128 = env
            .storage()
            .instance()
            .get(&PoolKey::Deposited)
            .unwrap_or(0);

        // Blend: `oracle_client.lastprice(&asset).unwrap_optimized()` -- no price, no action.
        let price = UpstreamClient::new(&env, &oracle)
            .lastprice(&collateral)
            .unwrap();
        let collateral_value = deposited * price.price / SCALAR * COLLATERAL_FACTOR_BPS / 10_000;

        let liquidity: i128 = env
            .storage()
            .instance()
            .get(&PoolKey::Liquidity(asset.clone()))
            .unwrap();
        assert!(amount <= liquidity, "not enough liquidity");
        let asset_price: i128 = env
            .storage()
            .instance()
            .get(&PoolKey::Price(asset.clone()))
            .unwrap();
        let debt: i128 = env
            .storage()
            .instance()
            .get(&PoolKey::DebtValue)
            .unwrap_or(0);
        let new_debt = debt + amount * asset_price / SCALAR;
        assert!(
            new_debt * LIABILITY_FACTOR_BPS / 10_000 <= collateral_value,
            "collateral does not cover the borrow"
        );

        env.storage().instance().set(&PoolKey::DebtValue, &new_debt);
        env.storage()
            .instance()
            .set(&PoolKey::Liquidity(asset), &(liquidity - amount));
    }
}

// ------------------------------------------------------------------ scenario setup

struct World<'a> {
    env: Env,
    ustry: Asset,
    xlm: Address,
    usdc: Address,
    feeds: [MockFeedClient<'a>; 3],
}

impl<'a> World<'a> {
    fn new() -> World<'a> {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(NOW);
        let ustry = Asset::Stellar(Address::generate(&env));
        let xlm = Address::generate(&env);
        let usdc = Address::generate(&env);
        let feeds = [feed(&env, 7), feed(&env, 7), feed(&env, 7)];
        let w = World {
            env,
            ustry,
            xlm,
            usdc,
            feeds,
        };
        w.publish([USTRY_REAL, USTRY_REAL, USTRY_NAV]);
        w
    }

    /// Every feed reports a fresh price for USTRY at the current ledger time.
    fn publish(&self, prices: [i128; 3]) {
        let now = self.env.ledger().timestamp();
        for (f, p) in self.feeds.iter().zip(prices) {
            f.set(&self.ustry, &p, &now);
        }
    }

    fn advance(&self, secs: u64) {
        let t = self.env.ledger().timestamp() + secs;
        self.env.ledger().set_timestamp(t);
    }

    fn damper(&self, min_sources: u32) -> GuardedOracleClient<'a> {
        let admin = Address::generate(&self.env);
        let base = Asset::Other(Symbol::new(&self.env, "USD"));
        let id = self.env.register(
            GuardedOracle,
            (admin, base, 7u32, 300u32, MAX_AGE, min_sources),
        );
        let oracle = GuardedOracleClient::new(&self.env, &id);
        let sources = vec![
            &self.env,
            (self.feeds[0].address.clone(), self.ustry.clone()),
            (self.feeds[1].address.clone(), self.ustry.clone()),
            (self.feeds[2].address.clone(), self.ustry.clone()),
        ];
        oracle.set_sources(&self.ustry, &sources);
        oracle
    }

    /// A pool whose only collateral is USTRY, priced by `oracle`.
    fn pool(&self, oracle: &Address) -> MockPoolClient<'a> {
        let id = self
            .env
            .register(MockPool, (oracle.clone(), self.ustry.clone()));
        let pool = MockPoolClient::new(&self.env, &id);
        pool.list(&self.xlm, &XLM_PRICE, &XLM_LIQUIDITY);
        pool.list(&self.usdc, &USDC_PRICE, &USDC_LIQUIDITY);
        pool.deposit(&ATTACKER_USTRY);
        pool
    }

    /// The incident's borrow: all the XLM and USDC in the pool.
    fn drain(&self, pool: &MockPoolClient) -> bool {
        pool.try_borrow(&self.xlm, &XLM_LIQUIDITY).is_ok()
            && pool.try_borrow(&self.usdc, &USDC_LIQUIDITY).is_ok()
    }
}

fn collateral_policy() -> Policy {
    // 5% per hour up, drops pass straight through; the anchor may lag by at most 24 hours
    Policy {
        up_bps_per_hour: 500,
        down_bps_per_hour: UNBOUNDED,
        window_secs: 24 * 3600,
    }
}

// ------------------------------------------------------------------ the replay

#[test]
fn control_a_single_manipulated_feed_drains_the_pool() {
    // Reproduces the incident: the pool reads one DEX-derived feed that has been pumped.
    let w = World::new();
    w.publish([USTRY_PUMPED, USTRY_PUMPED, USTRY_PUMPED]);
    let pool = w.pool(&w.feeds[0].address);
    assert!(
        w.drain(&pool),
        "the model must reproduce the exploit, or the other tests prove nothing"
    );
}

#[test]
fn control_the_same_pool_is_fine_at_the_real_price() {
    let w = World::new();
    let pool = w.pool(&w.feeds[0].address);
    assert!(!w.drain(&pool));
    // and an honest borrow of $50k against ~$132k of collateral works
    assert!(pool.try_borrow(&w.usdc, &(50_000 * SCALAR)).is_ok());
}

#[test]
fn median_of_three_stops_a_single_pumped_feed() {
    let w = World::new();
    // only the DEX-derived feed is pumped; the AMM-TWAP style feed and the NAV feed are not
    w.publish([USTRY_PUMPED, USTRY_REAL, USTRY_NAV]);
    let oracle = w.damper(2);
    let pool = w.pool(&oracle.address);

    assert_eq!(oracle.lastprice(&w.ustry).unwrap().price, USTRY_NAV);
    assert!(!w.drain(&pool));
    assert!(pool.try_borrow(&w.usdc, &(50_000 * SCALAR)).is_ok());
}

#[test]
fn median_alone_cannot_stop_manipulation_of_a_shared_market() {
    // If two of three upstreams read the same thin market, a median follows them.
    // This is the gap the anchor clamp closes.
    let w = World::new();
    w.publish([USTRY_PUMPED, USTRY_PUMPED, USTRY_NAV]);
    let oracle = w.damper(2);
    let pool = w.pool(&oracle.address);
    assert!(w.drain(&pool));
}

#[test]
fn anchor_clamp_stops_manipulation_of_a_shared_market() {
    let w = World::new();
    let oracle = w.damper(2);
    oracle.set_policy(&w.ustry, &collateral_policy());
    oracle.seed_anchor(&w.ustry); // calm market: anchor = $1.06 -> median of real, real, nav

    // the pump arrives in every upstream at once
    w.advance(300);
    w.publish([USTRY_PUMPED, USTRY_PUMPED, USTRY_PUMPED]);

    let raw = oracle.raw_price(&w.ustry).unwrap().price;
    let quote = oracle.lastprice(&w.ustry).unwrap().price;
    assert_eq!(raw, USTRY_PUMPED, "the upstreams really do say $106.74");
    // 300s at 500 bps/hour = +41.67 bps over the $1.06 anchor: 10_600_000 * 150_000 / 36_000_000
    assert_eq!(quote, USTRY_REAL + 44_166, "got {quote}");

    let pool = w.pool(&oracle.address);
    assert!(!w.drain(&pool));
}

#[test]
fn sustained_pump_never_gets_close_to_draining_the_pool() {
    // The attacker holds the pumped price for a full day while a keeper pokes every hour.
    let w = World::new();
    let oracle = w.damper(2);
    oracle.set_policy(&w.ustry, &collateral_policy());
    oracle.seed_anchor(&w.ustry);
    let pool = w.pool(&oracle.address);

    let mut best_borrow_usd = 0i128;
    for _ in 0..24 {
        w.advance(3600);
        w.publish([USTRY_PUMPED, USTRY_PUMPED, USTRY_PUMPED]);
        oracle.poke(&w.ustry);
        assert!(!w.drain(&pool), "pool drained mid-attack");

        // most the attacker could borrow right now, in USD
        let price = oracle.lastprice(&w.ustry).unwrap().price;
        let cap = ATTACKER_USTRY * price / SCALAR * COLLATERAL_FACTOR_BPS / LIABILITY_FACTOR_BPS;
        best_borrow_usd = best_borrow_usd.max(cap / SCALAR);
    }
    // The incident took ~$10.8M. Holding the pump for 24h with the keeper poking hourly
    // lets the attacker borrow about $350k (3%), the price of waiting a day, not of one trade:
    assert!(
        best_borrow_usd < 400_000,
        "borrowable after 24h: ${best_borrow_usd}"
    );
}

#[test]
fn without_a_keeper_the_band_stops_widening_at_the_window() {
    let w = World::new();
    let oracle = w.damper(2);
    oracle.set_policy(&w.ustry, &collateral_policy());
    oracle.seed_anchor(&w.ustry);

    w.advance(30 * 24 * 3600); // a month with nobody poking
    w.publish([USTRY_PUMPED, USTRY_PUMPED, USTRY_PUMPED]);
    // 24h window x 5%/h = +120% at most: $1.06 -> $2.33
    let quote = oracle.lastprice(&w.ustry).unwrap().price;
    assert_eq!(quote, USTRY_REAL * 220 / 100);
}

#[test]
fn a_real_crash_reaches_the_pool_immediately() {
    // The clamp must not hide real drops, or liquidations stop working in a crash.
    let w = World::new();
    let oracle = w.damper(2);
    oracle.set_policy(&w.ustry, &collateral_policy());
    oracle.seed_anchor(&w.ustry);

    w.advance(300);
    w.publish([5_000_000, 5_000_000, 5_000_000]); // -53% in one round
    assert_eq!(oracle.lastprice(&w.ustry).unwrap().price, 5_000_000);
}

#[test]
fn fails_closed_until_an_anchor_exists() {
    let w = World::new();
    let oracle = w.damper(2);
    oracle.set_policy(&w.ustry, &collateral_policy());
    assert_eq!(oracle.lastprice(&w.ustry), None);
    let pool = w.pool(&oracle.address);
    assert!(pool.try_borrow(&w.usdc, &SCALAR).is_err());
}

#[test]
fn poke_cannot_be_used_to_move_the_anchor_faster_than_the_policy() {
    let w = World::new();
    let oracle = w.damper(2);
    oracle.set_policy(&w.ustry, &collateral_policy());
    oracle.seed_anchor(&w.ustry);
    let start = oracle.anchor(&w.ustry).unwrap();

    w.publish([USTRY_PUMPED, USTRY_PUMPED, USTRY_PUMPED]);
    // spamming poke inside the minimum interval does nothing
    for _ in 0..10 {
        w.advance(10);
        oracle.poke(&w.ustry);
    }
    assert_eq!(oracle.anchor(&w.ustry).unwrap(), start);

    // after the interval it moves by the policy's allowance for that time, not to $106
    w.advance(300);
    w.publish([USTRY_PUMPED, USTRY_PUMPED, USTRY_PUMPED]);
    oracle.poke(&w.ustry);
    let moved = oracle.anchor(&w.ustry).unwrap().price;
    assert!(
        moved > start.price && moved < start.price * 102 / 100,
        "moved to {moved}"
    );
}

#[test]
fn feeds_with_no_fresh_price_do_not_reach_the_clamp() {
    let w = World::new();
    let oracle = w.damper(2);
    oracle.set_policy(&w.ustry, &collateral_policy());
    oracle.seed_anchor(&w.ustry);
    w.advance(MAX_AGE + 1); // nothing republished: all upstream prices are stale
    assert_eq!(oracle.lastprice(&w.ustry), None);
}

#[allow(dead_code)]
fn _types_are_used(_: PriceData) {}

// ------------------------------------------------------------------ the gate

use crate::{Gate, UNLIMITED};

const THIN_COST: i128 = 1_200 * SCALAR; // USD to push the price up 10x on a market with almost no depth
const DEEP_COST: i128 = 50_000_000 * SCALAR;
const EXPOSURE: i128 = 10_800_000 * SCALAR; // what the pool could lend against the false price

fn gate() -> Gate {
    Gate {
        tight_up_bps_per_hour: 50,
        tight_down_bps_per_hour: UNBOUNDED,
        min_cost_ratio_bps: 20_000, // pushing the price must cost at least 2x what it unlocks
        max_evidence_age: 3600,
    }
}

/// An oracle for USTRY with the collateral policy and the gate, anchor seeded on a calm market.
fn gated<'a>(w: &World<'a>) -> GuardedOracleClient<'a> {
    let oracle = w.damper(2);
    oracle.set_policy(&w.ustry, &collateral_policy());
    oracle.set_gate(&w.ustry, &gate());
    oracle.seed_anchor(&w.ustry);
    oracle
}

/// `anchor * rate_bps * secs / (3600 * 10_000)`, the contract's band.
fn band(anchor: i128, rate_bps: i128, secs: i128) -> i128 {
    anchor * rate_bps * secs / 36_000_000
}

fn pump(w: &World) {
    w.publish([USTRY_PUMPED, USTRY_PUMPED, USTRY_PUMPED]);
}

#[test]
fn a_thin_market_gets_the_tight_rate_and_a_deep_one_the_policy_rate() {
    let w = World::new();
    let oracle = gated(&w);
    w.advance(3600);
    pump(&w);

    oracle.publish_evidence(&w.ustry, &THIN_COST, &EXPOSURE, &DEEP_COST, &EXPOSURE);
    let thin = oracle.lastprice(&w.ustry).unwrap().price;
    assert_eq!(thin, USTRY_REAL + band(USTRY_REAL, 50, 3600));

    oracle.publish_evidence(&w.ustry, &DEEP_COST, &EXPOSURE, &DEEP_COST, &EXPOSURE);
    let deep = oracle.lastprice(&w.ustry).unwrap().price;
    assert_eq!(deep, USTRY_REAL + band(USTRY_REAL, 500, 3600));
    assert!(thin < deep);
}

#[test]
fn without_fresh_evidence_the_tight_rate_applies() {
    let w = World::new();
    let oracle = gated(&w);
    w.advance(3600);
    pump(&w);
    // nothing published at all
    assert_eq!(
        oracle.lastprice(&w.ustry).unwrap().price,
        USTRY_REAL + band(USTRY_REAL, 50, 3600)
    );

    // published, then left to go stale
    oracle.publish_evidence(&w.ustry, &DEEP_COST, &EXPOSURE, &DEEP_COST, &EXPOSURE);
    w.advance(3601);
    pump(&w);
    assert_eq!(
        oracle.lastprice(&w.ustry).unwrap().price,
        USTRY_REAL + band(USTRY_REAL, 50, 3600 + 3601)
    );
}

#[test]
fn an_asset_with_no_gate_keeps_its_policy_rate() {
    let w = World::new();
    let oracle = w.damper(2);
    oracle.set_policy(&w.ustry, &collateral_policy());
    oracle.seed_anchor(&w.ustry);
    w.advance(3600);
    pump(&w);
    assert_eq!(
        oracle.lastprice(&w.ustry).unwrap().price,
        USTRY_REAL + band(USTRY_REAL, 500, 3600)
    );
    assert_eq!(oracle.effective_policy(&w.ustry), Some(collateral_policy()));
}

/// Hold the pump for 24 hours with the keeper poking hourly, and return the most the
/// attacker could have borrowed at any point, in whole dollars.
fn best_borrow_while_holding(with_gate: bool, cost: i128) -> i128 {
    let w = World::new();
    let oracle = w.damper(2);
    oracle.set_policy(&w.ustry, &collateral_policy());
    if with_gate {
        oracle.set_gate(&w.ustry, &gate());
    }
    oracle.seed_anchor(&w.ustry);
    let pool = w.pool(&oracle.address);

    let mut best = 0i128;
    for _ in 0..24 {
        w.advance(3600);
        pump(&w);
        if with_gate {
            oracle.publish_evidence(&w.ustry, &cost, &EXPOSURE, &cost, &EXPOSURE);
        }
        oracle.poke(&w.ustry);
        assert!(!w.drain(&pool), "pool drained mid-attack");
        let price = oracle.lastprice(&w.ustry).unwrap().price;
        let cap = ATTACKER_USTRY * price / SCALAR * COLLATERAL_FACTOR_BPS / LIABILITY_FACTOR_BPS;
        best = best.max(cap / SCALAR);
    }
    best
}

#[test]
fn a_held_pump_on_a_thin_market_borrows_far_less_with_the_gate() {
    let without = best_borrow_while_holding(false, 0);
    let thin = best_borrow_while_holding(true, THIN_COST);
    let deep = best_borrow_while_holding(true, DEEP_COST);

    // The incident took about $10.8M. Without the gate, a day of holding borrows about $350k.
    // With the gate on a market this thin, about a third of that. On a deep market it makes
    // no difference: the gate only acts where manipulation is cheap.
    assert!(thin < without / 2, "thin {thin} vs without {without}");
    assert!(thin < 150_000, "thin {thin}");
    assert_eq!(deep, without);
}

#[test]
fn a_real_crash_still_reaches_the_pool_through_the_gate() {
    let w = World::new();
    let oracle = gated(&w);
    w.advance(300);
    w.publish([5_000_000, 5_000_000, 5_000_000]);
    oracle.publish_evidence(&w.ustry, &THIN_COST, &EXPOSURE, &THIN_COST, &EXPOSURE);
    assert_eq!(oracle.lastprice(&w.ustry).unwrap().price, 5_000_000);
}

#[test]
fn untradeable_prices_keep_the_policy_rate_whatever_the_exposure() {
    let w = World::new();
    let oracle = gated(&w);
    w.advance(3600);
    pump(&w);
    oracle.publish_evidence(
        &w.ustry,
        &UNLIMITED,
        &(10 * EXPOSURE),
        &UNLIMITED,
        &(10 * EXPOSURE),
    );
    assert_eq!(
        oracle.lastprice(&w.ustry).unwrap().price,
        USTRY_REAL + band(USTRY_REAL, 500, 3600)
    );
}

#[test]
fn a_keeper_cannot_loosen_a_policy_with_evidence() {
    // The best evidence a (compromised) keeper can publish returns the policy rate, no more.
    let w = World::new();
    let oracle = gated(&w);
    oracle.publish_evidence(&w.ustry, &UNLIMITED, &0, &UNLIMITED, &0);
    assert_eq!(oracle.effective_policy(&w.ustry), Some(collateral_policy()));
}

#[test]
fn bad_gates_and_bad_evidence_are_rejected() {
    let w = World::new();
    let oracle = gated(&w);
    let bad_gates = [
        Gate {
            min_cost_ratio_bps: 0,
            ..gate()
        },
        Gate {
            max_evidence_age: 0,
            ..gate()
        },
        Gate {
            max_evidence_age: 8 * 24 * 3600,
            ..gate()
        },
    ];
    for g in bad_gates {
        assert!(oracle.try_set_gate(&w.ustry, &g).is_err());
    }
    assert!(
        oracle
            .try_publish_evidence(&w.ustry, &0, &1, &1, &1)
            .is_err(),
        "zero cost"
    );
    assert!(
        oracle
            .try_publish_evidence(&w.ustry, &1, &-1, &1, &1)
            .is_err(),
        "negative exposure"
    );
    assert!(
        oracle
            .try_publish_evidence(&w.ustry, &1, &1, &-5, &1)
            .is_err(),
        "negative cost"
    );
    assert!(
        oracle
            .try_publish_evidence(&w.ustry, &1, &0, &1, &0)
            .is_ok(),
        "zero exposure is allowed"
    );
}
