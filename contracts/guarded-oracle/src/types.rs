use soroban_sdk::{contracterror, contracttype, Address, Symbol};

/// SEP-40 asset identifier. Layout must match the SEP-40 spec exactly so that
/// consumers built against `sep-40-oracle` (e.g. Blend pools) can decode it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Asset {
    Stellar(Address),
    Other(Symbol),
}

/// SEP-40 price record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceData {
    pub price: i128,
    pub timestamp: u64,
}

/// One upstream SEP-40 feed that prices a given asset.
/// `asset` is the identifier the *upstream* oracle uses, which may differ from ours
/// (e.g. a CEX feed keys XLM as `Other("XLM")` while we key it by its SAC address).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Source {
    pub oracle: Address,
    pub asset: Asset,
    pub decimals: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub admin: Address,
    pub base: Asset,
    pub decimals: u32,
    pub resolution: u32,
    /// Upstream prices older than this many seconds are ignored.
    pub max_age: u64,
    /// Minimum number of fresh upstream prices required to quote at all.
    pub min_sources: u32,
}

/// Sentinel for "no limit in this direction".
pub const UNBOUNDED: u32 = u32::MAX;

/// How far a quote may move away from the stored anchor, per asset.
///
/// Rates are in basis points per hour and apply to the time since the anchor was last
/// moved, capped at `window_secs`. The allowance therefore grows with time but can never
/// exceed `rate * window` between two pokes, so a price that has been pushed for hours
/// still cannot jump by more than the policy allows.
///
/// - Collateral-side assets: bound `up`, leave `down` unbounded so liquidations keep working.
/// - Debt-side assets: bound `down`, leave `up` unbounded.
/// - Assets used both ways: bound both.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Policy {
    pub up_bps_per_hour: u32,
    pub down_bps_per_hour: u32,
    pub window_secs: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Anchor {
    pub price: i128,
    pub timestamp: u64,
}

/// Cost sentinel: the price cannot be moved by trading (for example it comes from off-chain
/// exchanges). Treated as an uneconomic attack whatever the exposure.
pub const UNLIMITED: i128 = i128::MAX;

/// What the keeper reports about one asset's market, in USD with the oracle's decimals.
///
/// `up_cost` is the capital needed to push the price up about 10x and `up_exposure` what an
/// attacker could then borrow against it; `down_cost` and `down_exposure` are the same for
/// pushing it down. The oracle only trusts these to *loosen nothing*: see `Gate`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Evidence {
    pub up_cost: i128,
    pub up_exposure: i128,
    pub down_cost: i128,
    pub down_exposure: i128,
    pub timestamp: u64,
}

/// Ties an asset's clamp rate to the evidence.
///
/// A bounded direction keeps its policy rate only while fresh evidence says moving the price
/// costs at least `min_cost_ratio_bps / 10_000` times what it would let an attacker borrow.
/// With stale or missing evidence, or when the attack looks economic, the rate falls to the
/// tight rate. The gate can only tighten a policy, never loosen it, and never bounds a
/// direction the policy leaves unbounded.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Gate {
    pub tight_up_bps_per_hour: u32,
    pub tight_down_bps_per_hour: u32,
    pub min_cost_ratio_bps: u32,
    pub max_evidence_age: u64,
}

/// Everything a keeper reads for one asset, in the shape the separate `policy`,
/// `effective_policy`, `anchor`, `raw_price`, `lastprice`, `gate` and `evidence` views
/// already return, gathered into one read-only call (issue #21). Reading it costs no more
/// than one `lastprice` call: every field here is already computed, not re-derived.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssetStatus {
    pub policy: Option<Policy>,
    /// The policy actually in force right now, after the gate. `None` exactly when `policy` is.
    pub effective_policy: Option<Policy>,
    pub anchor: Option<Anchor>,
    /// The median of fresh upstream prices, before any clamp.
    pub raw_price: Option<PriceData>,
    /// What `lastprice` returns right now: the raw median, clamped when the asset has a policy.
    pub quote: Option<PriceData>,
    pub gate: Option<Gate>,
    pub evidence: Option<Evidence>,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OracleError {
    InvalidConfig = 1,
    TooManySources = 2,
    NoSources = 3,
    DecimalsOutOfRange = 4,
    InvalidPolicy = 5,
    NoPrice = 6,
    NoPolicy = 7,
    InvalidGate = 8,
    InvalidEvidence = 9,
}
