//! Coverage-guided companion to `fuzz.rs`'s `uneconomic` property tests.
#![no_main]
use arbitrary::Arbitrary;
use damper_guarded_oracle::aggregate::uneconomic;
use damper_guarded_oracle::reference::{exact_uneconomic, within_exact_range};
use damper_guarded_oracle::UNLIMITED;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    cost: i128,
    exposure: i128,
    ratio: u32,
    more_cost: i128,
    less_exposure: i128,
    lower_ratio: u32,
}

/// `v`'s magnitude as a non-negative `i128`, saturating rather than wrapping: `i128::MIN`'s
/// magnitude does not fit in `i128`, and a wrapped-negative "non-negative" delta would silently
/// invert the monotonicity checks below instead of strengthening them.
fn nonneg(v: i128) -> i128 {
    i128::try_from(v.unsigned_abs()).unwrap_or(i128::MAX)
}

fuzz_target!(|i: Input| {
    let got = uneconomic(i.cost, i.exposure, i.ratio);
    let exact = exact_uneconomic(i.cost, i.exposure, i.ratio);

    // Over every amount, the contract never calls an attack uneconomic when exact arithmetic
    // says it would pay — it may only ever err towards caution, past where it is exact.
    assert!(!got || exact, "contract says uneconomic ({got}), exact arithmetic says it pays");
    if within_exact_range(i.cost, i.exposure) {
        assert_eq!(got, exact, "contract disagrees with exact arithmetic inside its exact range");
    }

    // The edges that must never panic: nothing to take, a price trading cannot move.
    if i.exposure <= 0 || i.cost == UNLIMITED {
        assert!(got, "should be uneconomic: exposure {} cost {}", i.exposure, i.cost);
    }

    // Monotone: more cost, less exposure or a lower required ratio can only make an attack
    // *more* uneconomic, from any starting point that already was.
    if got {
        let more_cost = i.cost.saturating_add(nonneg(i.more_cost));
        assert!(uneconomic(more_cost, i.exposure, i.ratio), "more cost stopped being uneconomic");
        let less_exposure = i.exposure.saturating_sub(nonneg(i.less_exposure)).max(0);
        assert!(uneconomic(i.cost, less_exposure, i.ratio), "less exposure stopped being uneconomic");
        let lower_ratio = i.ratio.saturating_sub(i.lower_ratio);
        assert!(uneconomic(i.cost, i.exposure, lower_ratio), "a lower ratio stopped being uneconomic");
    }
});
