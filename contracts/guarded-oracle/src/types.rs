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
}
