//! Pure price math, kept free of storage so it can be unit- and fuzz-tested directly.

use crate::types::{Evidence, Gate, Policy, UNBOUNDED, UNLIMITED};
use soroban_sdk::Vec;

/// Rescale `price` from `from` decimals to `to` decimals.
/// Returns `None` on overflow or if the result would be non-positive.
pub fn normalize(price: i128, from: u32, to: u32) -> Option<i128> {
    let out = if to >= from {
        price.checked_mul(10i128.checked_pow(to - from)?)?
    } else {
        price / 10i128.checked_pow(from - to)?
    };
    (out > 0).then_some(out)
}

/// Median of a small set of prices. Even-length sets average the two middle values
/// (rounding down). Returns `None` for an empty set.
pub fn median(values: &Vec<i128>) -> Option<i128> {
    let n = values.len();
    if n == 0 {
        return None;
    }
    // Insertion sort: source sets are capped at MAX_SOURCES, so O(n^2) is fine
    // and avoids pulling an allocator-backed sort into the contract.
    let mut sorted = values.clone();
    for i in 1..n {
        let mut j = i;
        while j > 0 && sorted.get_unchecked(j - 1) > sorted.get_unchecked(j) {
            let a = sorted.get_unchecked(j - 1);
            let b = sorted.get_unchecked(j);
            sorted.set(j - 1, b);
            sorted.set(j, a);
            j -= 1;
        }
    }
    let mid = n / 2;
    if n % 2 == 1 {
        Some(sorted.get_unchecked(mid))
    } else {
        let lo = sorted.get_unchecked(mid - 1);
        let hi = sorted.get_unchecked(mid);
        // lo + (hi - lo) / 2 cannot overflow for positive sorted inputs
        Some(lo + (hi - lo) / 2)
    }
}

/// Bound `price` to the band the policy allows around `anchor`.
///
/// `elapsed` is the time since the anchor was last moved. The band is
/// `anchor * (1 +/- rate * min(elapsed, window) / 1h)` with the rate in basis points.
/// A lower bound of zero is lifted to 1 so the quote never becomes non-positive.
pub fn clamp(price: i128, anchor: i128, elapsed: u64, policy: &Policy) -> i128 {
    let secs = elapsed.min(policy.window_secs) as u128;
    let mut out = price;

    if policy.up_bps_per_hour != UNBOUNDED {
        let hi = anchor.saturating_add(band(anchor, policy.up_bps_per_hour, secs));
        out = out.min(hi);
    }
    if policy.down_bps_per_hour != UNBOUNDED {
        // moving down by 100% or more would reach zero; cap at 100%
        let lo = anchor - band(anchor, policy.down_bps_per_hour, secs).min(anchor);
        out = out.max(lo.max(1));
    }
    out
}

/// `anchor * rate_bps * secs / (3600 * 10_000)`, done in a single division so short
/// intervals do not round to zero, and saturating instead of overflowing.
fn band(anchor: i128, rate_bps: u32, secs: u128) -> i128 {
    const DEN: i128 = 3600 * 10_000;
    let num = (rate_bps as u128)
        .saturating_mul(secs)
        .min(i128::MAX as u128) as i128;
    match anchor.checked_mul(num) {
        Some(v) => v / DEN,
        None => (anchor / DEN).saturating_mul(num),
    }
}

/// True when moving the price costs at least `min_ratio_bps / 10_000` times what it would let
/// an attacker borrow. Nothing to take, or a price that trading cannot move, is uneconomic.
pub fn uneconomic(cost: i128, exposure: i128, min_ratio_bps: u32) -> bool {
    if cost == UNLIMITED || exposure <= 0 {
        return true;
    }
    match exposure.checked_mul(min_ratio_bps as i128) {
        Some(required) => cost.saturating_mul(10_000) >= required,
        // exposure so large that the product overflows: not uneconomic
        None => false,
    }
}

/// The policy that applies right now: the admin's policy, with each bounded direction cut to
/// the gate's tight rate unless fresh evidence says an attack in that direction is uneconomic.
pub fn effective_policy(
    policy: &Policy,
    gate: &Gate,
    evidence: Option<&Evidence>,
    now: u64,
) -> Policy {
    let fresh =
        evidence.filter(|e| e.timestamp <= now && now - e.timestamp <= gate.max_evidence_age);
    let pick = |policy_rate: u32, tight: u32, cost: Option<i128>, exposure: Option<i128>| {
        if policy_rate == UNBOUNDED {
            return UNBOUNDED;
        }
        match (cost, exposure) {
            (Some(c), Some(x)) if uneconomic(c, x, gate.min_cost_ratio_bps) => policy_rate,
            _ => policy_rate.min(tight),
        }
    };
    Policy {
        up_bps_per_hour: pick(
            policy.up_bps_per_hour,
            gate.tight_up_bps_per_hour,
            fresh.map(|e| e.up_cost),
            fresh.map(|e| e.up_exposure),
        ),
        down_bps_per_hour: pick(
            policy.down_bps_per_hour,
            gate.tight_down_bps_per_hour,
            fresh.map(|e| e.down_cost),
            fresh.map(|e| e.down_exposure),
        ),
        window_secs: policy.window_secs,
    }
}
