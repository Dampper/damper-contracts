//! Independent reference arithmetic for the clamp and the gate, written from their written
//! rules rather than from the contract's code, with exact (256-bit where needed) precision.
//!
//! One copy, used by both the property tests (`fuzz.rs`, `#[cfg(test)]`) and the
//! coverage-guided fuzz targets (`fuzz/fuzz_targets/`, an ordinary external crate that can only
//! see this crate's public API) — a second copy would drift, and would be the reference against
//! which "the reference agrees" stops meaning anything. It has no effect on the deployed
//! contract's on-chain interface: Soroban only exposes what `#[contractimpl]` methods return.
use crate::{Evidence, Gate, Policy, UNBOUNDED, UNLIMITED};

/// `anchor * rate_bps * secs / (3600 s/h * 10_000 bps)`, as the contract's band is defined.
const DEN: u128 = 36_000_000;

/// `a * b` as exact 128-bit halves `(hi, lo)`: `a*b = hi * 2^128 + lo`. `i128::MAX * u32::MAX`
/// does not fit in 128 bits, so `uneconomic`'s `cost * 10_000` and `exposure * ratio` need this
/// to check exactly rather than risk the same overflow the contract itself must avoid.
pub fn mul_wide(a: u128, b: u128) -> (u128, u128) {
    const MASK: u128 = u64::MAX as u128;
    let (a0, a1) = (a & MASK, a >> 64);
    let (b0, b1) = (b & MASK, b >> 64);
    let (p00, p01, p10, p11) = (a0 * b0, a0 * b1, a1 * b0, a1 * b1);
    let mid = (p00 >> 64) + (p01 & MASK) + (p10 & MASK);
    let lo = (p00 & MASK) | (mid << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    (hi, lo)
}

/// `cost * 10_000 >= exposure * ratio`, exactly, for any `i128` amounts. An exposure of zero or
/// less means there is nothing to take; `UNLIMITED` is a price trading cannot move.
pub fn exact_uneconomic(cost: i128, exposure: i128, ratio: u32) -> bool {
    if cost == UNLIMITED || exposure <= 0 {
        return true;
    }
    if cost <= 0 {
        return false;
    }
    mul_wide(cost as u128, 10_000) >= mul_wide(exposure as u128, ratio as u128)
}

/// The amounts for which the contract is exact. `cost * 10_000` saturates, and a saturated
/// product is still at least any `exposure * ratio` that fits in 128 bits, so a huge cost is
/// handled exactly. Only an exposure large enough for `exposure * ratio` to overflow (above
/// 2^95, given a ratio up to 2^32) can make the contract err towards the tight rate.
pub fn within_exact_range(cost: i128, exposure: i128) -> bool {
    cost == UNLIMITED || exposure < (1i128 << 95)
}

/// A bounded direction is cut to the tight rate; a free direction stays free.
pub fn cut(policy_rate: u32, tight: u32) -> u32 {
    if policy_rate == UNBOUNDED {
        UNBOUNDED
    } else {
        policy_rate.min(tight)
    }
}

/// The gate's rule, written from its description in the docs rather than from the contract's
/// code: a bounded direction keeps its policy rate only while *fresh* evidence says an attack
/// costs at least `min_cost_ratio_bps / 10_000` times what it unlocks; otherwise it is cut to
/// the tight rate. A direction the policy leaves free stays free.
pub fn reference_effective(
    policy: &Policy,
    gate: &Gate,
    ev: Option<&Evidence>,
    now: u64,
) -> Policy {
    let fresh = ev.filter(|e| e.timestamp <= now && now - e.timestamp <= gate.max_evidence_age);
    let direction = |policy_rate: u32, tight: u32, cost_exposure: Option<(i128, i128)>| -> u32 {
        if policy_rate == UNBOUNDED {
            return UNBOUNDED;
        }
        let keeps = match cost_exposure {
            None => false,
            Some((cost, exposure)) => exact_uneconomic(cost, exposure, gate.min_cost_ratio_bps),
        };
        if keeps {
            policy_rate
        } else {
            cut(policy_rate, tight)
        }
    };
    Policy {
        up_bps_per_hour: direction(
            policy.up_bps_per_hour,
            gate.tight_up_bps_per_hour,
            fresh.map(|e| (e.up_cost, e.up_exposure)),
        ),
        down_bps_per_hour: direction(
            policy.down_bps_per_hour,
            gate.tight_down_bps_per_hour,
            fresh.map(|e| (e.down_cost, e.down_exposure)),
        ),
        window_secs: policy.window_secs,
    }
}

/// The band the policy allows, written out independently of the contract's code.
pub fn reference_band(anchor: i128, rate: u32, secs: u64, window: u64) -> u128 {
    let secs = secs.min(window) as u128;
    anchor as u128 * rate as u128 * secs / DEN
}

/// `clamp`'s reference, for an anchor and price known to be non-negative and within a range
/// `u128` arithmetic covers without overflow (real oracle prices; not `i128::MIN`, not every
/// representable `i128`, which `fuzz.rs`'s own property tests cover with the contract's own
/// saturating arithmetic instead).
pub fn reference_clamp(price: i128, anchor: i128, elapsed: u64, policy: &Policy) -> i128 {
    let mut want = price;
    if policy.up_bps_per_hour != UNBOUNDED {
        let hi = anchor
            + reference_band(anchor, policy.up_bps_per_hour, elapsed, policy.window_secs) as i128;
        want = want.min(hi);
    }
    if policy.down_bps_per_hour != UNBOUNDED {
        // moving down by 100% or more would reach zero, so the allowance is capped at the anchor
        let allow = (reference_band(
            anchor,
            policy.down_bps_per_hour,
            elapsed,
            policy.window_secs,
        ) as i128)
            .min(anchor);
        want = want.max((anchor - allow).max(1));
    }
    want
}
