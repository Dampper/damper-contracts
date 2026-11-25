//! Coverage-guided companion to `fuzz.rs`'s clamp property tests. Random sampling there already
//! covers the input space well; this explores it by code path instead, which tends to find the
//! single boundary input a sampler walks past (the `proptest-regressions/` entries already show
//! that gap closing from the other side).
#![no_main]
use arbitrary::Arbitrary;
use damper_guarded_oracle::reference::reference_clamp;
use damper_guarded_oracle::{aggregate::clamp, Policy, UNBOUNDED};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    price: i128,
    anchor: i128,
    elapsed: u64,
    up_bps_per_hour: u32,
    down_bps_per_hour: u32,
    window_secs: u64,
}

fuzz_target!(|i: Input| {
    // window_secs = 0 is rejected by set_policy; the clamp itself still must not panic on it,
    // but exact-reference comparison below needs a real divisor, so give it one here.
    let policy = Policy {
        up_bps_per_hour: i.up_bps_per_hour,
        down_bps_per_hour: i.down_bps_per_hour,
        window_secs: i.window_secs.max(1),
    };

    let r = clamp(i.price, i.anchor, i.elapsed, &policy);
    assert!(r >= 1, "clamp returned {r} < 1 (unconditional, see clamp's doc comment)");

    if i.anchor <= 0 {
        // A percentage band only means something around a positive reference price; every real
        // caller guarantees one (see clamp's doc comment). Defensively this is the one case
        // where "stays between price and anchor" does not apply — neither can be trusted —
        // so the only promise left is the unconditional floor just checked.
        assert_eq!(r, 1, "a non-positive anchor should fall back to the floor, not {r}");
        return;
    }
    if i.price <= 0 {
        // Same reasoning, the other input: "stays between price and anchor" presumes a real
        // price. The floor above is still the only promise; which positive value it lands on
        // otherwise depends on the policy (the down branch's own floor, or the unconditional one
        // when down is UNBOUNDED), so there is nothing further to check here.
        return;
    }

    assert!(r >= i.price.min(i.anchor), "moved below both bounds: {r}");
    assert!(r <= i.price.max(i.anchor), "moved above both bounds: {r}");
    assert_eq!(clamp(r, i.anchor, i.elapsed, &policy), r, "clamp is not idempotent");
    if policy.up_bps_per_hour == UNBOUNDED && i.price >= i.anchor {
        assert_eq!(r, i.price, "touched a price on the unbounded up side");
    }
    if policy.down_bps_per_hour == UNBOUNDED && i.price <= i.anchor {
        assert_eq!(r, i.price, "touched a price on the unbounded down side");
    }

    // Exact match against the independent reference, in the range `anchor * rate * secs`
    // computes in u128 without overflow (real oracle prices and policies; the same envelope
    // fuzz.rs's own `clamp_matches_the_reference_formula` uses). The extremes of every type are
    // covered by the never-panics and never-moves-the-wrong-way checks above instead.
    const WEEK: u64 = 7 * 24 * 3600;
    const SAFE_PRICE: i128 = 100_000_000_000_000_000_000_000_000;
    const SAFE_ANCHOR: i128 = 100_000_000_000_000_000_000;
    let safe_rate = |r: u32| r == UNBOUNDED || r <= 1_000_000;
    if (1..=SAFE_PRICE).contains(&i.price)
        && (1..=SAFE_ANCHOR).contains(&i.anchor)
        && safe_rate(policy.up_bps_per_hour)
        && safe_rate(policy.down_bps_per_hour)
        && i.elapsed <= 2 * WEEK
        && (1..=WEEK).contains(&policy.window_secs)
    {
        assert_eq!(r, reference_clamp(i.price, i.anchor, i.elapsed, &policy), "disagrees with the reference in the exact range");
    }
});
