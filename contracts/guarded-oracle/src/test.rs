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

pub(crate) const NOW: u64 = 1_800_000_000;
pub(crate) const MAX_AGE: u64 = 900;

pub(crate) struct Setup<'a> {
    pub(crate) env: Env,
    pub(crate) oracle: GuardedOracleClient<'a>,
    pub(crate) asset: Asset,
}

pub(crate) fn setup(min_sources: u32) -> Setup<'static> {
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

pub(crate) fn feed(env: &Env, decimals: u32) -> MockFeedClient<'static> {
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

// ---------------------------------------------------------------- clamp math

mod clamp_math {
    use crate::{aggregate::clamp, Policy, UNBOUNDED};

    fn up_only(rate: u32) -> Policy {
        Policy {
            up_bps_per_hour: rate,
            down_bps_per_hour: UNBOUNDED,
            window_secs: 24 * 3600,
        }
    }

    #[test]
    fn no_time_no_upward_room() {
        assert_eq!(clamp(2_000, 1_000, 0, &up_only(500)), 1_000);
    }

    #[test]
    fn upward_room_grows_linearly_with_time() {
        // 500 bps/hour: +5% after 1h, +10% after 2h
        assert_eq!(clamp(9_999_999, 1_000_000, 3600, &up_only(500)), 1_050_000);
        assert_eq!(clamp(9_999_999, 1_000_000, 7200, &up_only(500)), 1_100_000);
    }

    #[test]
    fn room_is_capped_at_the_window() {
        let p = up_only(500);
        assert_eq!(
            clamp(i128::MAX / 2, 1_000_000, 24 * 3600, &p),
            clamp(i128::MAX / 2, 1_000_000, 365 * 24 * 3600, &p)
        );
    }

    #[test]
    fn prices_inside_the_band_are_untouched() {
        assert_eq!(clamp(1_020_000, 1_000_000, 3600, &up_only(500)), 1_020_000);
    }

    #[test]
    fn drops_pass_when_down_is_unbounded() {
        assert_eq!(clamp(1, 1_000_000, 0, &up_only(500)), 1);
    }

    #[test]
    fn downward_bound_mirrors_upward() {
        let p = Policy {
            up_bps_per_hour: UNBOUNDED,
            down_bps_per_hour: 500,
            window_secs: 24 * 3600,
        };
        assert_eq!(clamp(1, 1_000_000, 3600, &p), 950_000);
        assert_eq!(clamp(5_000_000, 1_000_000, 0, &p), 5_000_000);
    }

    #[test]
    fn downward_bound_never_reaches_zero() {
        let p = Policy {
            up_bps_per_hour: UNBOUNDED,
            down_bps_per_hour: 100_000, // would be -1000% per hour
            window_secs: 3600,
        };
        assert_eq!(clamp(1, 1_000_000, 3600, &p), 1);
    }

    #[test]
    fn short_intervals_do_not_round_to_zero() {
        // 300s at 500 bps/hour is 41.67 bps; on a 1e12 anchor that is 4.1e9, not 0
        assert_eq!(
            clamp(i128::MAX / 2, 1_000_000_000_000, 300, &up_only(500)),
            1_000_000_000_000 + 4_166_666_666
        );
    }

    #[test]
    fn huge_anchors_saturate_instead_of_overflowing() {
        let big = i128::MAX / 4;
        assert!(clamp(i128::MAX, big, 24 * 3600, &up_only(u32::MAX - 1)) >= big);
    }
}

// ---------------------------------------------------------------- keeper auth

mod keeper_auth {
    use super::{feed, setup, NOW};
    use crate::{Policy, UNBOUNDED};
    use soroban_sdk::{testutils::Address as _, testutils::Ledger, vec, Address};

    fn armed() -> (crate::test::Setup<'static>, Address) {
        let s = setup(1);
        let f = feed(&s.env, 7);
        f.set(&s.asset, &10_000_000, &NOW);
        s.oracle.set_sources(
            &s.asset,
            &vec![&s.env, (f.address.clone(), s.asset.clone())],
        );
        s.oracle.set_policy(
            &s.asset,
            &Policy {
                up_bps_per_hour: 500,
                down_bps_per_hour: UNBOUNDED,
                window_secs: 3600,
            },
        );
        s.oracle.seed_anchor(&s.asset);
        s.env.ledger().set_timestamp(NOW + 600);
        f.set(&s.asset, &10_000_000, &(NOW + 600));
        let admin = s.oracle.config().admin;
        (s, admin)
    }

    #[test]
    fn poke_carries_every_entry_a_price_lookup_reads_past_a_quiet_period() {
        use crate::{DataKey, DAY_LEDGERS};
        use soroban_sdk::testutils::storage::{Instance as _, Persistent as _};
        let (s, _) = armed();
        // 50 days pass with nobody writing: the TTLs set at deployment run low
        s.env
            .ledger()
            .set_sequence_number(s.env.ledger().sequence() + 50 * DAY_LEDGERS);
        s.oracle.poke(&s.asset);
        let id = s.oracle.address.clone();
        s.env.as_contract(&id, || {
            let p = s.env.storage().persistent();
            for key in [
                DataKey::Sources(s.asset.clone()),
                DataKey::Policy(s.asset.clone()),
                DataKey::Anchor(s.asset.clone()),
            ] {
                assert!(
                    p.get_ttl(&key) >= 59 * DAY_LEDGERS,
                    "persistent entry not extended"
                );
            }
            assert!(s.env.storage().instance().get_ttl() >= 59 * DAY_LEDGERS);
        });
    }

    #[test]
    fn keeper_defaults_to_the_admin_and_poke_requires_its_auth() {
        let (s, admin) = armed();
        assert_eq!(s.oracle.keeper(), admin);
        s.oracle.poke(&s.asset);
        let auths = s.env.auths();
        assert_eq!(auths.len(), 1);
        assert_eq!(auths[0].0, admin);
    }

    #[test]
    fn poke_requires_the_configured_keeper_not_the_admin() {
        let (s, admin) = armed();
        let keeper = Address::generate(&s.env);
        s.oracle.set_keeper(&keeper);
        assert_eq!(s.oracle.keeper(), keeper);

        s.oracle.poke(&s.asset);
        let auths = s.env.auths();
        assert_eq!(auths.len(), 1);
        assert_eq!(auths[0].0, keeper);
        assert_ne!(auths[0].0, admin);
    }
}
