//! Pure price math, kept free of storage so it can be unit- and fuzz-tested directly.

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
