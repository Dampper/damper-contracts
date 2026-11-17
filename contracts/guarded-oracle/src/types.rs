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

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum OracleError {
    InvalidConfig = 1,
    TooManySources = 2,
    NoSources = 3,
    DecimalsOutOfRange = 4,
}
