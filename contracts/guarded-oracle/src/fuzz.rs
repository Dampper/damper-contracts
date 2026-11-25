#![cfg(test)]
//! Property and stateful fuzz tests.
//!
//! The unit tests and the YieldBlox replay check chosen cases. These check rules that must hold
//! for *every* input, and let a generator hunt for the input that breaks one:
//!
//! - the pure price maths (`clamp`, the gate, the median, decimals) against independent
//!   reference formulas and algebraic laws;
//! - the whole contract, under random sequences of operations (feeds going stale or wild,
//!   policy and gate changes, evidence, pokes, time passing), with the invariants checked
//!   after every step.
//!
//! `cargo test` runs a modest number of cases so CI stays quick. For a deeper search:
//! `PROPTEST_CASES=5000 cargo test fuzz`. A failure is shrunk to a small input and saved under
//! `proptest-regressions/`; commit that file so the case is replayed from then on.

use crate::aggregate::{clamp, effective_policy, median, normalize, uneconomic};
use crate::reference::{
    cut, exact_uneconomic, reference_band, reference_clamp, reference_effective, within_exact_range,
};
use crate::test::{feed, MockFeedClient, MAX_AGE, NOW};
use crate::{
    Asset, Evidence, Gate, GuardedOracle, GuardedOracleClient, Policy, UNBOUNDED, UNLIMITED,
};
use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    vec, Address, Env, Symbol, Vec as SVec,
};

const WEEK: u64 = 7 * 24 * 3600;

fn config(default_cases: u32) -> ProptestConfig {
    let mut c = ProptestConfig::default(); // honours PROPTEST_CASES
    if std::env::var("PROPTEST_CASES").is_err() {
        c.cases = default_cases;
    }
    c
}

// ------------------------------------------------------------------ generators

fn rate() -> impl Strategy<Value = u32> {
    prop_oneof![
        Just(UNBOUNDED),
        Just(0u32),
        1u32..=2_000,
        2_000u32..=1_000_000,
        any::<u32>(),
    ]
}

fn window() -> impl Strategy<Value = u64> {
    prop_oneof![1u64..=WEEK, Just(1u64), Just(WEEK), 1u64..=3600]
}

fn policy() -> impl Strategy<Value = Policy> {
    (rate(), rate(), window()).prop_map(|(up_bps_per_hour, down_bps_per_hour, window_secs)| {
        Policy {
            up_bps_per_hour,
            down_bps_per_hour,
            window_secs,
        }
    })
}

/// Any positive price, with the extremes over-represented.
fn price() -> impl Strategy<Value = i128> {
    prop_oneof![
        4 => 1i128..=1_000_000_000_000,
        2 => 1i128..=i128::MAX,
        1 => Just(1i128),
        1 => Just(i128::MAX),
    ]
}

fn elapsed() -> impl Strategy<Value = u64> {
    prop_oneof![4 => 0u64..=7200, 2 => 0u64..=700_000, 1 => any::<u64>(), 1 => Just(0u64), 1 => Just(u64::MAX)]
}

fn gate() -> impl Strategy<Value = Gate> {
    (rate(), rate(), 1u32..=100_000, 1u64..=WEEK).prop_map(
        |(tight_up_bps_per_hour, tight_down_bps_per_hour, min_cost_ratio_bps, max_evidence_age)| {
            Gate {
                tight_up_bps_per_hour,
                tight_down_bps_per_hour,
                min_cost_ratio_bps,
                max_evidence_age,
            }
        },
    )
}

/// Policies and gates the contract must refuse: a window or an evidence age of zero or past a
/// week, a zero cost ratio. Mixed with valid ones, so rejection is tested as much as acceptance.
fn any_policy() -> impl Strategy<Value = Policy> {
    let any_window =
        prop_oneof![8 => window(), 1 => Just(0u64), 1 => Just(WEEK + 1), 1 => any::<u64>()];
    (rate(), rate(), any_window).prop_map(|(up_bps_per_hour, down_bps_per_hour, window_secs)| {
        Policy {
            up_bps_per_hour,
            down_bps_per_hour,
            window_secs,
        }
    })
}

fn any_gate() -> impl Strategy<Value = Gate> {
    let ratio = prop_oneof![8 => 1u32..=100_000, 1 => Just(0u32), 1 => any::<u32>()];
    let age =
        prop_oneof![8 => 1u64..=WEEK, 1 => Just(0u64), 1 => Just(WEEK + 1), 1 => any::<u64>()];
    (rate(), rate(), ratio, age).prop_map(
        |(tight_up_bps_per_hour, tight_down_bps_per_hour, min_cost_ratio_bps, max_evidence_age)| {
            Gate {
                tight_up_bps_per_hour,
                tight_down_bps_per_hour,
                min_cost_ratio_bps,
                max_evidence_age,
            }
        },
    )
}

fn amount() -> impl Strategy<Value = i128> {
    prop_oneof![3 => 0i128..=1_000_000_000_000, 1 => any::<i128>(), 1 => Just(UNLIMITED), 1 => Just(0i128), 1 => Just(1i128)]
}

fn evidence(now: u64) -> impl Strategy<Value = Option<Evidence>> {
    prop_oneof![
        1 => Just(None),
        4 => (amount(), amount(), amount(), amount(), 0u64..=2 * WEEK).prop_map(move |(up_cost, up_exposure, down_cost, down_exposure, age)| {
            Some(Evidence { up_cost, up_exposure, down_cost, down_exposure, timestamp: now.saturating_sub(age) })
        }),
        1 => (amount(), amount()).prop_map(move |(c, x)| Some(Evidence { up_cost: c, up_exposure: x, down_cost: c, down_exposure: x, timestamp: now + 5 })),
    ]
}

// The reference arithmetic (`mul_wide`, `exact_uneconomic`, `within_exact_range`, `cut`,
// `reference_effective`, `reference_band`, `reference_clamp`) lives in `reference.rs`, shared
// with the coverage-guided fuzz targets under `fuzz/`.

proptest! {
    #![proptest_config(config(2_000))]

    /// Whatever the inputs, including the extremes of every type, the clamp does not panic,
    /// never returns less than 1, and only ever moves a price towards the anchor.
    #[test]
    fn clamp_only_moves_a_price_towards_the_anchor(
        price in price(), anchor in price(), elapsed in elapsed(), policy in policy(),
    ) {
        let r = clamp(price, anchor, elapsed, &policy);
        prop_assert!(r >= 1);
        prop_assert!(r >= price.min(anchor), "moved below both: {r}");
        prop_assert!(r <= price.max(anchor), "moved above both: {r}");
    }

    /// In the ranges real prices live in, the clamp equals the band computed from the written
    /// rule: anchor +/- anchor * rate * min(elapsed, window) / 1 h, floored at 1.
    #[test]
    fn clamp_matches_the_reference_formula(
        price in 1i128..=100_000_000_000_000_000_000_000_000,
        anchor in 1i128..=100_000_000_000_000_000_000,
        up in prop_oneof![Just(UNBOUNDED), 0u32..=1_000_000],
        down in prop_oneof![Just(UNBOUNDED), 0u32..=1_000_000],
        elapsed in 0u64..=2 * WEEK,
        window in 1u64..=WEEK,
    ) {
        let policy = Policy { up_bps_per_hour: up, down_bps_per_hour: down, window_secs: window };
        prop_assert_eq!(clamp(price, anchor, elapsed, &policy), reference_clamp(price, anchor, elapsed, &policy));
    }

    /// A higher input price never produces a lower quote.
    #[test]
    fn clamp_is_monotone_in_the_price(
        a in price(), b in price(), anchor in price(), elapsed in elapsed(), policy in policy(),
    ) {
        let (lo, hi) = (a.min(b), a.max(b));
        prop_assert!(clamp(lo, anchor, elapsed, &policy) <= clamp(hi, anchor, elapsed, &policy));
    }

    /// Clamping a quote that was already clamped changes nothing.
    #[test]
    fn clamp_is_idempotent(price in price(), anchor in price(), elapsed in elapsed(), policy in policy()) {
        let once = clamp(price, anchor, elapsed, &policy);
        prop_assert_eq!(clamp(once, anchor, elapsed, &policy), once);
    }

    /// More time never narrows the band: the quote can only get further from the anchor.
    #[test]
    fn more_time_never_narrows_the_band(
        price in price(), anchor in price(), e1 in elapsed(), e2 in elapsed(), policy in policy(),
    ) {
        let (short, long) = (e1.min(e2), e1.max(e2));
        let d = |r: i128| (r - anchor).unsigned_abs();
        prop_assert!(d(clamp(price, anchor, short, &policy)) <= d(clamp(price, anchor, long, &policy)));
    }

    /// A direction the policy leaves unbounded never touches a price on that side, which is
    /// what keeps liquidations working in a crash.
    #[test]
    fn an_unbounded_direction_is_never_touched(
        price in price(), anchor in price(), elapsed in elapsed(), up in rate(), down in rate(), window in window(),
    ) {
        let policy = Policy { up_bps_per_hour: up, down_bps_per_hour: down, window_secs: window };
        let r = clamp(price, anchor, elapsed, &policy);
        if up == UNBOUNDED && price >= anchor { prop_assert_eq!(r, price); }
        if down == UNBOUNDED && price <= anchor { prop_assert_eq!(r, price); }
    }
}

// ------------------------------------------------------------------ the gate

proptest! {
    #![proptest_config(config(2_000))]

    /// `uneconomic` is exact arithmetic: cost * 10_000 >= exposure * ratio, with no overflow
    /// in the range real amounts live in.
    #[test]
    fn uneconomic_matches_exact_arithmetic(cost in 1i128..(1 << 60), exposure in 1i128..(1 << 60), ratio in 1u32..=u32::MAX) {
        let want = (cost as u128) * 10_000 >= (exposure as u128) * (ratio as u128);
        prop_assert_eq!(uneconomic(cost, exposure, ratio), want);
    }

    /// More cost, less exposure or a lower required ratio can only make an attack less economic.
    #[test]
    fn uneconomic_is_monotone(
        cost in 1i128..(1 << 60), exposure in 1i128..(1 << 60), ratio in 1u32..=u32::MAX,
        more_cost in 0i128..(1 << 60), less_exposure in 0i128..(1 << 60), lower_ratio in 0u32..=u32::MAX,
    ) {
        if uneconomic(cost, exposure, ratio) {
            prop_assert!(uneconomic(cost + more_cost, exposure, ratio));
            prop_assert!(uneconomic(cost, (exposure - less_exposure.min(exposure)).max(0), ratio));
            prop_assert!(uneconomic(cost, exposure, ratio - lower_ratio.min(ratio)));
        }
    }

    /// Over *every* amount, including ones that overflow 128 bits, the contract never calls an
    /// attack uneconomic when exact maths says it would pay. Beyond 128 bits it may only err towards
    /// caution. Within them it is exact.
    #[test]
    fn uneconomic_is_never_more_optimistic_than_exact_maths(
        cost in prop_oneof![1i128..=i128::MAX, 1i128..=(1i128 << 113), 1i128..=1_000_000_000_000],
        exposure in prop_oneof![any::<i128>(), 0i128..=1_000_000_000_000_000_000, 1i128..(1i128 << 95), Just(1i128)],
        ratio in prop_oneof![1u32..=u32::MAX, 1u32..=100_000, Just(1u32)],
    ) {
        let got = uneconomic(cost, exposure, ratio);
        let exact = exact_uneconomic(cost, exposure, ratio);
        prop_assert!(!got || exact, "contract says uneconomic, exact maths says it pays");
        if within_exact_range(cost, exposure) {
            prop_assert_eq!(got, exact);
        }
    }

    /// The edges: nothing to take, a price trading cannot move, and amounts large enough to
    /// overflow, none of which may panic.
    #[test]
    fn uneconomic_survives_any_amount(cost in any::<i128>(), exposure in any::<i128>(), ratio in any::<u32>()) {
        let r = uneconomic(cost, exposure, ratio);
        if exposure <= 0 || cost == UNLIMITED { prop_assert!(r); }
    }

    /// Evidence can only ever tighten a policy: never a faster rate, never a rate where the
    /// policy had none, never a bound where the policy left a direction free.
    #[test]
    fn the_gate_never_loosens_a_policy(
        policy in policy(), gate in gate(), now in 0u64..=4 * WEEK, ev in evidence(2 * WEEK),
    ) {
        let eff = effective_policy(&policy, &gate, ev.as_ref(), now);
        prop_assert!(eff.up_bps_per_hour <= policy.up_bps_per_hour);
        prop_assert!(eff.down_bps_per_hour <= policy.down_bps_per_hour);
        prop_assert_eq!(eff.window_secs, policy.window_secs);
        prop_assert_eq!(eff.up_bps_per_hour == UNBOUNDED, policy.up_bps_per_hour == UNBOUNDED);
        prop_assert_eq!(eff.down_bps_per_hour == UNBOUNDED, policy.down_bps_per_hour == UNBOUNDED);
        // each direction is either the policy's rate or the tight rate cut to the policy's
        prop_assert!(eff.up_bps_per_hour == policy.up_bps_per_hour || eff.up_bps_per_hour == policy.up_bps_per_hour.min(gate.tight_up_bps_per_hour));
        prop_assert!(eff.down_bps_per_hour == policy.down_bps_per_hour || eff.down_bps_per_hour == policy.down_bps_per_hour.min(gate.tight_down_bps_per_hour));
    }

    /// No evidence, stale evidence and evidence from the future all mean the tight rate.
    #[test]
    fn without_trustworthy_evidence_the_tight_rate_applies(
        policy in policy(), gate in gate(), now in (2 * WEEK)..=4 * WEEK, stale_by in 1u64..=WEEK, future_by in 1u64..=WEEK,
        cost in amount(), exposure in amount(),
    ) {
        let tight = |e: Option<Evidence>| {
            let eff = effective_policy(&policy, &gate, e.as_ref(), now);
            (eff.up_bps_per_hour, eff.down_bps_per_hour)
        };
        let want = (cut(policy.up_bps_per_hour, gate.tight_up_bps_per_hour), cut(policy.down_bps_per_hour, gate.tight_down_bps_per_hour));
        let mk = |timestamp| Evidence { up_cost: cost, up_exposure: exposure, down_cost: cost, down_exposure: exposure, timestamp };
        prop_assert_eq!(tight(None), want);
        prop_assert_eq!(tight(Some(mk(now - gate.max_evidence_age - stale_by))), want);
        prop_assert_eq!(tight(Some(mk(now + future_by))), want);
    }

    /// Freshness is exact: evidence exactly `max_evidence_age` old still counts, and one second
    /// older does not.
    #[test]
    fn evidence_is_fresh_exactly_up_to_the_age_limit(policy in policy(), gate in gate(), now in (2 * WEEK)..=4 * WEEK) {
        // the best evidence there is: an attack that can never pay
        let best = |timestamp| Evidence { up_cost: UNLIMITED, up_exposure: 0, down_cost: UNLIMITED, down_exposure: 0, timestamp };
        let at_limit = effective_policy(&policy, &gate, Some(&best(now - gate.max_evidence_age)), now);
        prop_assert_eq!(at_limit, policy.clone());
        let past = effective_policy(&policy, &gate, Some(&best(now - gate.max_evidence_age - 1)), now);
        prop_assert_eq!(past.up_bps_per_hour, cut(policy.up_bps_per_hour, gate.tight_up_bps_per_hour));
        prop_assert_eq!(past.down_bps_per_hour, cut(policy.down_bps_per_hour, gate.tight_down_bps_per_hour));
    }

    /// The property the whole gate exists for: with the gate, the quote is never further from
    /// the anchor than without it, and never on the other side of it.
    #[test]
    fn the_gated_quote_is_never_further_from_the_anchor_than_the_ungated_one(
        price in price(), anchor in price(), elapsed in elapsed(), policy in policy(), gate in gate(),
        now in 0u64..=4 * WEEK, ev in evidence(2 * WEEK),
    ) {
        let eff = effective_policy(&policy, &gate, ev.as_ref(), now);
        let gated = clamp(price, anchor, elapsed, &eff);
        let ungated = clamp(price, anchor, elapsed, &policy);
        prop_assert!(gated >= ungated.min(anchor) && gated <= ungated.max(anchor),
            "gated {gated} not between anchor {anchor} and ungated {ungated}");
    }
}

// ------------------------------------------------------------------ median and decimals

proptest! {
    #![proptest_config(config(300))]

    /// The median is the middle value (the floor of the mean of the two middle values for an
    /// even count), whatever the order the feeds are listed in.
    #[test]
    fn the_median_is_the_middle_value_in_any_order(values in prop::collection::vec(1i128..=i128::MAX, 1..=7)) {
        let env = Env::default();
        let mut sorted = values.clone();
        sorted.sort();
        let n = sorted.len();
        let want = if n % 2 == 1 { sorted[n / 2] } else { sorted[n / 2 - 1] + (sorted[n / 2] - sorted[n / 2 - 1]) / 2 };

        let to_vec = |v: &[i128]| { let mut s: SVec<i128> = SVec::new(&env); for x in v { s.push_back(*x); } s };
        let mut reversed = values.clone();
        reversed.reverse();
        prop_assert_eq!(median(&to_vec(&values)), Some(want));
        prop_assert_eq!(median(&to_vec(&reversed)), Some(want));
        prop_assert!(want >= sorted[0] && want <= sorted[n - 1]);
    }

    /// One manipulated feed out of three cannot move the median: replace the largest with
    /// anything at least as large as the middle one, or the smallest with anything at most as
    /// small, and the answer is unchanged. This is the YieldBlox shape.
    #[test]
    fn one_wild_feed_out_of_three_cannot_move_the_median(
        mut v in prop::collection::vec(1i128..=1_000_000_000_000_000, 3..=3),
        wild_high in 1i128..=i128::MAX, wild_low in 1i128..=1_000,
    ) {
        let env = Env::default();
        v.sort();
        let (low, mid, high) = (v[0], v[1], v[2]);
        let m = |a: i128, b: i128, c: i128| median(&vec![&env, a, b, c]).unwrap();
        prop_assert_eq!(m(low, mid, high), mid);
        prop_assert_eq!(m(low, mid, wild_high.max(mid)), mid);
        prop_assert_eq!(m(wild_low.min(mid), mid, high), mid);
    }

    /// Rescaling between decimals never panics, never returns a non-positive price, and is
    /// lossless when scaling up.
    #[test]
    fn rescaling_decimals_is_safe_and_lossless_going_up(p in price(), from in 0u32..=18, to in 0u32..=18) {
        if let Some(v) = normalize(p, from, to) {
            prop_assert!(v > 0);
            if to >= from {
                prop_assert_eq!(normalize(v, to, from), Some(p));
            }
        } // otherwise it overflowed or rounded down to nothing: refused, not wrong
        prop_assert_eq!(normalize(p, from, from), Some(p));
    }
}

// ------------------------------------------------------------------ the whole contract

/// One thing that can happen to a deployed oracle.
#[derive(Clone, Debug)]
enum Op {
    Advance(u64),
    /// Each upstream reports `price` at a timestamp `offset` seconds from now (negative: stale).
    Feeds([(i128, i64); 3]),
    Poke,
    Seed,
    SetPolicy(Policy),
    SetGate(Gate),
    Evidence(i128, i128, i128, i128),
}

fn feed_price() -> impl Strategy<Value = i128> {
    prop_oneof![
        8 => 9_000_000i128..=12_000_000,
        3 => 1i128..=1_000_000_000_000,
        1 => 1i128..=1_000_000_000_000_000,
        1 => -5i128..=0,
    ]
}

fn feed_offset() -> impl Strategy<Value = i64> {
    prop_oneof![8 => Just(0i64), 2 => -(MAX_AGE as i64) * 3..=0, 1 => 1i64..=100]
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        3 => prop_oneof![4 => 0u64..=7200, 3 => 0u64..=600, 2 => Just(crate::MIN_POKE_SECS - 1), 2 => Just(crate::MIN_POKE_SECS), 1 => Just(0u64)].prop_map(Op::Advance),
        3 => ((feed_price(), feed_offset()), (feed_price(), feed_offset()), (feed_price(), feed_offset())).prop_map(|(a, b, c)| Op::Feeds([a, b, c])),
        2 => Just(Op::Poke),
        1 => Just(Op::Seed),
        2 => any_policy().prop_map(Op::SetPolicy),
        2 => any_gate().prop_map(Op::SetGate),
        2 => (amount(), amount(), amount(), amount()).prop_map(|(a, b, c, d)| Op::Evidence(a, b, c, d)),
    ]
}

struct Rig<'a> {
    env: Env,
    oracle: GuardedOracleClient<'a>,
    asset: Asset,
    feeds: [MockFeedClient<'a>; 3],
}

fn rig<'a>() -> Rig<'a> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(NOW);
    let admin = Address::generate(&env);
    let base = Asset::Other(Symbol::new(&env, "USD"));
    let id = env.register(GuardedOracle, (admin, base, 7u32, 300u32, MAX_AGE, 2u32));
    let oracle = GuardedOracleClient::new(&env, &id);
    let asset = Asset::Stellar(Address::generate(&env));
    let feeds = [feed(&env, 7), feed(&env, 7), feed(&env, 7)];
    for f in &feeds {
        f.set(&asset, &10_600_000, &NOW);
    }
    oracle.set_sources(
        &asset,
        &vec![
            &env,
            (feeds[0].address.clone(), asset.clone()),
            (feeds[1].address.clone(), asset.clone()),
            (feeds[2].address.clone(), asset.clone()),
        ],
    );
    // start from a working, guarded configuration so most sequences reach interesting states
    oracle.set_policy(
        &asset,
        &Policy {
            up_bps_per_hour: 500,
            down_bps_per_hour: UNBOUNDED,
            window_secs: 86_400,
        },
    );
    oracle.seed_anchor(&asset);
    Rig {
        env,
        oracle,
        asset,
        feeds,
    }
}

impl Rig<'_> {
    fn now(&self) -> u64 {
        self.env.ledger().timestamp()
    }

    /// The rules that must hold in every reachable state.
    fn check(&self) -> Result<(), TestCaseError> {
        let (o, a) = (&self.oracle, &self.asset);
        let now = self.now();
        let (raw, quote, policy, anchor, eff, gate) = (
            o.raw_price(a),
            o.lastprice(a),
            o.policy(a),
            o.anchor(a),
            o.effective_policy(a),
            o.gate(a),
        );
        let held_evidence = o.evidence(a);

        if let Some(q) = &quote {
            prop_assert!(q.price > 0, "non-positive quote {}", q.price);
            prop_assert!(q.timestamp <= now, "quote from the future");
        }
        // no quorum, no quote
        if raw.is_none() {
            prop_assert!(quote.is_none());
        }
        // a policy without an anchor fails closed
        if policy.is_some() && anchor.is_none() {
            prop_assert!(quote.is_none());
        }
        // an asset with no policy is quoted raw
        if policy.is_none() {
            prop_assert_eq!(&quote, &raw);
        }
        if let (Some(p), Some(e)) = (&policy, &eff) {
            prop_assert!(
                e.up_bps_per_hour <= p.up_bps_per_hour
                    && e.down_bps_per_hour <= p.down_bps_per_hour,
                "gate loosened the policy"
            );
            match &gate {
                None => prop_assert_eq!(e, p),
                Some(g) => {
                    let want = reference_effective(p, g, held_evidence.as_ref(), now);
                    let exact = held_evidence.as_ref().is_none_or(|ev| {
                        within_exact_range(ev.up_cost, ev.up_exposure)
                            && within_exact_range(ev.down_cost, ev.down_exposure)
                    });
                    if exact {
                        // where nothing can overflow, the contract applies the gate exactly as written
                        prop_assert_eq!(e, &want);
                    } else {
                        // beyond that it may only err towards the tight rate, never away from it
                        prop_assert!(
                            e.up_bps_per_hour <= want.up_bps_per_hour
                                && e.down_bps_per_hour <= want.down_bps_per_hour,
                            "contract {e:?} looser than exact maths {want:?}"
                        );
                    }
                }
            }
        }
        if let (Some(q), Some(r), Some(p), Some(anc)) = (&quote, &raw, &eff, &anchor) {
            // the clamp only moves a price towards the anchor
            prop_assert!(
                q.price >= r.price.min(anc.price) && q.price <= r.price.max(anc.price),
                "quote {} outside [{}, {}]",
                q.price,
                r.price,
                anc.price
            );
            // and never past the band the *effective* policy allows (the policy's, narrowed by the gate)
            let secs = now - anc.timestamp;
            if p.up_bps_per_hour != UNBOUNDED {
                let hi = anc.price
                    + reference_band(anc.price, p.up_bps_per_hour, secs, p.window_secs) as i128;
                prop_assert!(q.price <= hi, "quote {} above the band {}", q.price, hi);
            }
            if p.down_bps_per_hour != UNBOUNDED {
                let allow = (reference_band(anc.price, p.down_bps_per_hour, secs, p.window_secs)
                    as i128)
                    .min(anc.price);
                prop_assert!(
                    q.price >= (anc.price - allow).max(1),
                    "quote {} below the band",
                    q.price
                );
            }
        }
        if let Some(anc) = &anchor {
            prop_assert!(anc.timestamp <= now && anc.price > 0);
        }
        Ok(())
    }

    fn apply(&self, op: &Op) -> Result<(), TestCaseError> {
        let (o, a) = (&self.oracle, &self.asset);
        let now = self.now();
        match op {
            Op::Advance(secs) => self.env.ledger().set_timestamp(now + secs),
            Op::Feeds(specs) => {
                for (f, (price, offset)) in self.feeds.iter().zip(specs) {
                    f.set(a, price, &((now as i64 + offset).max(0) as u64));
                }
            }
            Op::SetPolicy(p) => {
                let before = o.policy(a);
                let ok = o.try_set_policy(a, p).is_ok();
                prop_assert_eq!(ok, p.window_secs >= 1 && p.window_secs <= WEEK);
                if !ok {
                    prop_assert_eq!(o.policy(a), before);
                }
            }
            Op::SetGate(g) => {
                let before = o.gate(a);
                let ok = o.try_set_gate(a, g).is_ok();
                prop_assert_eq!(
                    ok,
                    g.min_cost_ratio_bps > 0
                        && g.max_evidence_age >= 1
                        && g.max_evidence_age <= WEEK
                );
                if !ok {
                    prop_assert_eq!(o.gate(a), before);
                }
            }
            Op::Evidence(uc, ux, dc, dx) => {
                let before = o.evidence(a);
                let ok = o.try_publish_evidence(a, uc, ux, dc, dx).is_ok();
                prop_assert_eq!(ok, *uc > 0 && *dc > 0 && *ux >= 0 && *dx >= 0);
                if ok {
                    prop_assert_eq!(
                        o.evidence(a),
                        Some(Evidence {
                            up_cost: *uc,
                            up_exposure: *ux,
                            down_cost: *dc,
                            down_exposure: *dx,
                            timestamp: now
                        })
                    );
                } else {
                    prop_assert_eq!(o.evidence(a), before);
                }
            }
            Op::Seed => {
                let (before, raw) = (o.anchor(a), o.raw_price(a));
                let ok = o.try_seed_anchor(a).is_ok();
                prop_assert_eq!(ok, raw.is_some());
                match (ok, raw) {
                    (true, Some(r)) => prop_assert_eq!(
                        o.anchor(a),
                        Some(crate::Anchor {
                            price: r.price,
                            timestamp: now
                        })
                    ),
                    _ => prop_assert_eq!(o.anchor(a), before),
                }
            }
            Op::Poke => {
                let (before, quote) = (o.anchor(a), o.lastprice(a));
                let ok = o.try_poke(a).is_ok();
                match (&before, &quote) {
                    // too soon after the last move: nothing changes
                    (Some(b), _) if now - b.timestamp < crate::MIN_POKE_SECS => {
                        prop_assert_eq!(o.anchor(a), before)
                    }
                    // the anchor moves to the quote the pool was reading, no further
                    (Some(_), Some(q)) => {
                        prop_assert!(ok);
                        prop_assert_eq!(
                            o.anchor(a),
                            Some(crate::Anchor {
                                price: q.price,
                                timestamp: now
                            })
                        );
                    }
                    // nothing to poke with
                    _ => {
                        prop_assert!(!ok);
                        prop_assert_eq!(o.anchor(a), before);
                    }
                }
            }
        }
        Ok(())
    }
}

/// The stateful test is far slower per case than the pure ones, so it has its own knob:
/// `DAMPER_FUZZ_STATEFUL_CASES` (default 12).
fn stateful_config() -> ProptestConfig {
    ProptestConfig {
        // Shrinking a failing sequence is silent and, unbounded, can run for an hour. Bound it so a
        // failure is reported with a reasonably small input.
        max_shrink_iters: 4_000,
        cases: std::env::var("DAMPER_FUZZ_STATEFUL_CASES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(12),
        ..ProptestConfig::default()
    }
}

proptest! {
    #![proptest_config(stateful_config())]

    /// Whatever happens to the feeds, the policy, the gate, the evidence and the clock, and in
    /// whatever order, the contract keeps its promises after every step.
    #[test]
    fn the_contract_keeps_its_invariants_under_random_operations(ops in prop::collection::vec(op(), 1..=30)) {
        let rig = rig();
        rig.check()?;
        for op in &ops {
            rig.apply(op)?;
            rig.check()?;
        }
    }
}
