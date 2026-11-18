//! Pure price math, kept free of storage so it can be unit- and fuzz-tested directly.

use crate::types::{Policy, UNBOUNDED};
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
