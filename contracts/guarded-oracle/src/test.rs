#![cfg(test)]

use crate::{aggregate, Asset, GuardedOracle, GuardedOracleClient, Policy, PriceData, UNBOUNDED};
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
fn policies_with_an_invalid_window_are_rejected() {
    // found missing by the fuzz tests: a zero window would freeze the band at nothing, and one
    // past a week would let a long-stale anchor stand for any price
    let s = setup(1);
    let ok = |window_secs| {
        s.oracle
            .try_set_policy(
                &s.asset,
                &Policy {
                    up_bps_per_hour: 500,
                    down_bps_per_hour: UNBOUNDED,
                    window_secs,
                },
            )
            .is_ok()
    };
    assert!(!ok(0));
    assert!(!ok(7 * 24 * 3600 + 1));
    assert!(!ok(u64::MAX));
    assert!(ok(1));
    assert!(ok(7 * 24 * 3600));
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

    /// `cargo-fuzz`'s `clamp` target (ADR 0006), first run: with an unconstrained `i128` anchor
    /// (the property tests' `price()`/`anchor()` strategies only ever generate positive values,
    /// so this domain had never been exercised), a negative anchor made the band arithmetic's
    /// overflow fallback produce a value with the wrong sign, and the result landed far outside
    /// `[price, anchor]` — here, positive when both inputs were deeply negative.
    #[test]
    fn a_non_positive_anchor_falls_back_to_the_floor_instead_of_a_wrong_sign() {
        let p = Policy {
            up_bps_per_hour: 3_250_700_737,
            down_bps_per_hour: 3_250_700_737,
            window_secs: 13_907_115_649_320_141_249,
        };
        let price = -82_735_320_584_698_763_665_604_806_512_822_075_126;
        let anchor = -82_735_320_584_698_763_665_604_806_512_822_074_943;
        assert_eq!(clamp(price, anchor, 13_961_653_357_748_797_889, &p), 1);
    }

    /// `cargo-fuzz`'s `clamp` target (ADR 0006), second run: with a valid (positive) anchor but
    /// both directions `UNBOUNDED`, a non-positive price skipped the only branch that floors the
    /// result (the down branch), so it passed straight through, unfloored.
    #[test]
    fn a_non_positive_price_still_floors_at_one_when_both_directions_are_unbounded() {
        let p = Policy {
            up_bps_per_hour: UNBOUNDED,
            down_bps_per_hour: UNBOUNDED,
            window_secs: u64::MAX,
        };
        assert_eq!(clamp(-246, i128::MAX / 2, u64::MAX, &p), 1);
        assert_eq!(clamp(0, 1_000_000, 0, &p), 1);
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

// ---------------------------------------------------------------- gate math

mod gate_math {
    use crate::{
        aggregate::{effective_policy, uneconomic},
        Evidence, Gate, Policy, UNBOUNDED, UNLIMITED,
    };

    const NOW: u64 = 1_000_000;

    fn gate() -> Gate {
        Gate {
            tight_up_bps_per_hour: 50,
            tight_down_bps_per_hour: 50,
            min_cost_ratio_bps: 20_000, // cost must be at least 2x the exposure
            max_evidence_age: 3600,
        }
    }
    fn both(rate: u32) -> Policy {
        Policy {
            up_bps_per_hour: rate,
            down_bps_per_hour: rate,
            window_secs: 86_400,
        }
    }
    fn evidence(
        up_cost: i128,
        up_exposure: i128,
        down_cost: i128,
        down_exposure: i128,
        ts: u64,
    ) -> Evidence {
        Evidence {
            up_cost,
            up_exposure,
            down_cost,
            down_exposure,
            timestamp: ts,
        }
    }

    #[test]
    fn cost_must_clear_the_ratio() {
        // exposure 100, ratio 2x: cost 200 is enough, 199 is not
        assert!(uneconomic(200, 100, 20_000));
        assert!(!uneconomic(199, 100, 20_000));
        // a ratio below 1x allows cost under exposure
        assert!(uneconomic(50, 100, 5_000));
    }

    #[test]
    fn nothing_to_take_or_untradeable_is_uneconomic() {
        assert!(uneconomic(1, 0, 20_000));
        assert!(uneconomic(1, -5, 20_000));
        assert!(uneconomic(UNLIMITED, i128::MAX, 20_000));
    }

    #[test]
    fn overflow_is_not_treated_as_safe() {
        assert!(!uneconomic(1_000, i128::MAX, 20_000));
        // and a huge cost does not overflow either
        assert!(uneconomic(i128::MAX - 1, 1_000, 20_000));
    }

    #[test]
    fn a_cost_too_large_to_multiply_is_still_uneconomic() {
        // found by the stateful fuzzer: cost * 10_000 overflows i128, but a cost that large dwarfs
        // any exposure, so the attack is uneconomic and the saturating product says so
        let cost = 34_028_236_692_093_846_346_337_460_743_176_822_i128;
        assert!(uneconomic(cost, 1, 1));
        assert!(uneconomic(cost, 1_000_000_000_000, 20_000));
    }

    /// `cargo-fuzz`'s `gate` target (ADR 0006): with `min_cost_ratio_bps` left unconstrained
    /// (every proptest generator restricts it to `1..=100_000`, matching `set_gate`'s own
    /// rejection of 0, so this value had never been tried), a ratio of 0 made a free (zero-cost)
    /// attack against real exposure read as uneconomic: `exposure * 0 == 0`, and
    /// `cost.saturating_mul(10_000)` is also 0 when cost is 0, so `0 >= 0` passed. Unreachable
    /// through `set_gate` today, but a free attack against something worth taking must never
    /// read as safe regardless of how it is reached.
    #[test]
    fn a_free_attack_against_real_exposure_is_never_uneconomic() {
        assert!(!uneconomic(
            0,
            13_362_324_784_046_394_687_261_146_873_856,
            0
        ));
        assert!(!uneconomic(0, 1, 1));
        assert!(!uneconomic(-1, 1, 1));
    }

    #[test]
    fn fresh_uneconomic_evidence_keeps_the_policy_rate() {
        let e = evidence(1_000, 100, 1_000, 100, NOW - 10);
        assert_eq!(
            effective_policy(&both(500), &gate(), Some(&e), NOW),
            both(500)
        );
    }

    #[test]
    fn an_economic_attack_gets_the_tight_rate() {
        let e = evidence(100, 1_000, 100, 1_000, NOW - 10);
        let p = effective_policy(&both(500), &gate(), Some(&e), NOW);
        assert_eq!((p.up_bps_per_hour, p.down_bps_per_hour), (50, 50));
    }

    #[test]
    fn missing_stale_and_future_evidence_get_the_tight_rate() {
        let deep = |ts| evidence(1_000, 100, 1_000, 100, ts);
        let tight =
            |e: Option<&Evidence>| effective_policy(&both(500), &gate(), e, NOW).up_bps_per_hour;
        assert_eq!(tight(None), 50);
        assert_eq!(
            tight(Some(&deep(NOW - 3_600))),
            500,
            "exactly at the age limit is still fresh"
        );
        assert_eq!(tight(Some(&deep(NOW - 3_601))), 50);
        assert_eq!(
            tight(Some(&deep(NOW + 1))),
            50,
            "evidence from the future is not trusted"
        );
    }

    #[test]
    fn directions_are_judged_independently() {
        let e = evidence(1_000, 100, 100, 1_000, NOW);
        let p = effective_policy(&both(500), &gate(), Some(&e), NOW);
        assert_eq!((p.up_bps_per_hour, p.down_bps_per_hour), (500, 50));
    }

    #[test]
    fn the_gate_never_loosens_a_policy() {
        // tight rate above the policy rate: the lower one wins
        let loose = Gate {
            tight_up_bps_per_hour: 900,
            ..gate()
        };
        let e = evidence(100, 1_000, 100, 1_000, NOW);
        assert_eq!(
            effective_policy(&both(500), &loose, Some(&e), NOW).up_bps_per_hour,
            500
        );
        // and good evidence cannot raise a rate above the policy's
        let deep = evidence(1_000_000, 1, 1_000_000, 1, NOW);
        assert_eq!(
            effective_policy(&both(500), &gate(), Some(&deep), NOW).up_bps_per_hour,
            500
        );
    }

    #[test]
    fn a_direction_the_policy_leaves_unbounded_stays_unbounded() {
        // collateral policy: drops pass through, so liquidations keep working whatever the evidence says
        let collateral = Policy {
            up_bps_per_hour: 500,
            down_bps_per_hour: UNBOUNDED,
            window_secs: 86_400,
        };
        let e = evidence(100, 1_000, 100, 1_000, NOW);
        let p = effective_policy(&collateral, &gate(), Some(&e), NOW);
        assert_eq!((p.up_bps_per_hour, p.down_bps_per_hour), (50, UNBOUNDED));
        assert_eq!(
            effective_policy(&collateral, &gate(), None, NOW).down_bps_per_hour,
            UNBOUNDED
        );
    }
}

mod events {
    use super::{feed, setup, Setup, NOW};
    use crate::{
        AnchorSeeded, EvidencePublished, Gate, GateSet, KeeperSet, Poked, Policy, PolicySet,
        SourcesSet, UNBOUNDED,
    };
    use soroban_sdk::{
        testutils::{Address as _, Events as _, Ledger},
        vec, Address, Event,
    };

    /// The events of the call just made, and nothing else.
    fn only<E: Event>(s: &Setup, e: E) {
        assert_eq!(
            s.env.events().all(),
            std::vec![e.to_xdr(&s.env, &s.oracle.address)]
        );
    }

    fn none(s: &Setup) {
        assert!(s.env.events().all().events().is_empty());
    }

    fn armed() -> Setup<'static> {
        let s = setup(1);
        let f = feed(&s.env, 7);
        f.set(&s.asset, &10_000_000, &NOW);
        s.oracle.set_sources(
            &s.asset,
            &vec![&s.env, (f.address.clone(), s.asset.clone())],
        );
        only(
            &s,
            SourcesSet {
                asset: s.asset.clone(),
                count: 1,
            },
        );
        s
    }

    fn policy() -> Policy {
        Policy {
            up_bps_per_hour: 500,
            down_bps_per_hour: UNBOUNDED,
            window_secs: 3600,
        }
    }

    #[test]
    fn each_admin_change_says_what_changed() {
        let s = armed();

        s.oracle.set_policy(&s.asset, &policy());
        only(
            &s,
            PolicySet {
                asset: s.asset.clone(),
                up_bps_per_hour: 500,
                down_bps_per_hour: UNBOUNDED,
                window_secs: 3600,
            },
        );

        s.oracle.set_gate(
            &s.asset,
            &Gate {
                tight_up_bps_per_hour: 100,
                tight_down_bps_per_hour: 100,
                min_cost_ratio_bps: 15_000,
                max_evidence_age: 3600,
            },
        );
        only(
            &s,
            GateSet {
                asset: s.asset.clone(),
                tight_up_bps_per_hour: 100,
                tight_down_bps_per_hour: 100,
                min_cost_ratio_bps: 15_000,
                max_evidence_age: 3600,
            },
        );

        s.oracle.seed_anchor(&s.asset);
        only(
            &s,
            AnchorSeeded {
                asset: s.asset.clone(),
                price: 10_000_000,
            },
        );

        let keeper = Address::generate(&s.env);
        s.oracle.set_keeper(&keeper);
        only(
            &s,
            KeeperSet {
                keeper: keeper.clone(),
            },
        );
    }

    #[test]
    fn a_poke_reports_the_old_and_new_anchor_and_a_skipped_poke_is_silent() {
        let s = armed();
        s.oracle.set_policy(&s.asset, &policy());
        s.oracle.seed_anchor(&s.asset);

        // inside the minimum interval nothing moves, so nothing is said
        s.env.ledger().set_timestamp(NOW + 299);
        s.oracle.poke(&s.asset);
        none(&s);

        // a 5% rise held for ten minutes: the quote is clamped, and the event shows it
        let f = crate::test::MockFeedClient::new(
            &s.env,
            &s.oracle.sources(&s.asset).get(0).unwrap().oracle,
        );
        s.env.ledger().set_timestamp(NOW + 600);
        f.set(&s.asset, &30_000_000, &(NOW + 600));
        let quote = s.oracle.lastprice(&s.asset).unwrap().price;
        assert!(quote < 30_000_000);
        s.oracle.poke(&s.asset);
        only(
            &s,
            Poked {
                asset: s.asset.clone(),
                from: 10_000_000,
                to: quote,
                after_secs: 600,
            },
        );
        assert_eq!(s.oracle.anchor(&s.asset).unwrap().price, quote);
    }

    #[test]
    fn evidence_is_published_with_its_numbers() {
        let s = armed();
        s.oracle
            .publish_evidence(&s.asset, &1_000, &2_000, &3_000, &4_000);
        only(
            &s,
            EvidencePublished {
                asset: s.asset.clone(),
                up_cost: 1_000,
                up_exposure: 2_000,
                down_cost: 3_000,
                down_exposure: 4_000,
            },
        );
    }

    #[test]
    fn a_rejected_call_leaves_no_event() {
        let s = armed();
        let bad = Policy {
            window_secs: 0,
            ..policy()
        };
        assert!(s.oracle.try_set_policy(&s.asset, &bad).is_err());
        none(&s);
        assert!(s
            .oracle
            .try_publish_evidence(&s.asset, &0, &1, &1, &1)
            .is_err());
        none(&s);
    }

    #[test]
    fn reading_a_price_emits_nothing() {
        let s = armed();
        s.oracle.lastprice(&s.asset);
        none(&s);
    }
}
