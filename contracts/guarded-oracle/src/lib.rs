#![no_std]
//! Damper guarded oracle.
//!
//! A SEP-40 price feed that sits between a lending pool and its upstream oracles.
//! It takes the median of several fresh upstream prices, refuses to quote when too few
//! are available, and (per asset, when a policy is set) bounds how fast the quote may
//! move away from a stored anchor price. Collateral assets bound upward moves and let
//! drops through, so a manipulated price cannot be borrowed against while a real crash
//! still reaches the pool and liquidations keep working.

mod aggregate;
mod types;

#[cfg(test)]
mod replay;
#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, panic_with_error, Address, Env, Vec,
};

pub use types::{Anchor, Asset, Config, OracleError, Policy, PriceData, Source, UNBOUNDED};

pub const MAX_SOURCES: u32 = 7;
const MAX_DECIMALS: u32 = 18;

/// `poke` is a no-op more often than this: moving the anchor resets the elapsed time
/// that widens the band, so unlimited pokes could freeze it.
pub const MIN_POKE_SECS: u64 = 300;
const MAX_WINDOW_SECS: u64 = 7 * 24 * 3600;

pub(crate) const DAY_LEDGERS: u32 = 17_280;
const TTL_THRESHOLD: u32 = 30 * DAY_LEDGERS;
const TTL_EXTEND_TO: u32 = 60 * DAY_LEDGERS;

/// Client for the subset of SEP-40 we read from upstream feeds.
#[contractclient(name = "UpstreamClient")]
pub trait Upstream {
    fn decimals(env: Env) -> u32;
    fn lastprice(env: Env, asset: Asset) -> Option<PriceData>;
}

#[contracttype]
#[derive(Clone)]
pub(crate) enum DataKey {
    Config,
    Assets,
    Sources(Asset),
    Policy(Asset),
    Anchor(Asset),
    Keeper,
}

#[contract]
pub struct GuardedOracle;

#[contractimpl]
impl GuardedOracle {
    pub fn __constructor(
        env: Env,
        admin: Address,
        base: Asset,
        decimals: u32,
        resolution: u32,
        max_age: u64,
        min_sources: u32,
    ) {
        if decimals > MAX_DECIMALS || max_age == 0 || min_sources == 0 || min_sources > MAX_SOURCES
        {
            panic_with_error!(&env, OracleError::InvalidConfig);
        }
        let config = Config {
            admin,
            base,
            decimals,
            resolution,
            max_age,
            min_sources,
        };
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage()
            .instance()
            .set(&DataKey::Assets, &Vec::<Asset>::new(&env));
    }

    // ---------------------------------------------------------------- admin

    /// Replace the upstream feeds for `asset`. Each entry is (upstream oracle, the
    /// upstream's identifier for this asset). Upstream decimals are read and cached here.
    pub fn set_sources(env: Env, asset: Asset, feeds: Vec<(Address, Asset)>) {
        let config = load_config(&env);
        config.admin.require_auth();

        if feeds.is_empty() {
            panic_with_error!(&env, OracleError::NoSources);
        }
        if feeds.len() > MAX_SOURCES {
            panic_with_error!(&env, OracleError::TooManySources);
        }
        if feeds.len() < config.min_sources {
            panic_with_error!(&env, OracleError::InvalidConfig);
        }

        let mut sources = Vec::new(&env);
        for (oracle, upstream_asset) in feeds.iter() {
            let decimals = UpstreamClient::new(&env, &oracle).decimals();
            if decimals > MAX_DECIMALS {
                panic_with_error!(&env, OracleError::DecimalsOutOfRange);
            }
            sources.push_back(Source {
                oracle,
                asset: upstream_asset,
                decimals,
            });
        }

        let key = DataKey::Sources(asset.clone());
        env.storage().persistent().set(&key, &sources);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);

        let mut assets: Vec<Asset> = env.storage().instance().get(&DataKey::Assets).unwrap();
        if !assets.contains(&asset) {
            assets.push_back(asset);
            env.storage().instance().set(&DataKey::Assets, &assets);
        }
        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
    }

    pub fn sources(env: Env, asset: Asset) -> Vec<Source> {
        env.storage()
            .persistent()
            .get(&DataKey::Sources(asset))
            .unwrap_or(Vec::new(&env))
    }

    /// Set how fast the quote for `asset` may move away from its anchor. After this the
    /// asset returns no price until an anchor exists (`seed_anchor`).
    pub fn set_policy(env: Env, asset: Asset, policy: Policy) {
        load_config(&env).admin.require_auth();
        if policy.window_secs == 0 || policy.window_secs > MAX_WINDOW_SECS {
            panic_with_error!(&env, OracleError::InvalidPolicy);
        }
        let key = DataKey::Policy(asset);
        env.storage().persistent().set(&key, &policy);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
    }

    /// Admin: set the anchor for `asset` to the current unclamped median. This is the one
    /// moment the clamp trusts the upstreams outright, so run it when the market is calm.
    pub fn seed_anchor(env: Env, asset: Asset) {
        load_config(&env).admin.require_auth();
        let Some(p) = Self::raw_price(env.clone(), asset.clone()) else {
            panic_with_error!(&env, OracleError::NoPrice);
        };
        store_anchor(&env, &asset, p.price);
    }

    /// Admin: set the address allowed to call `poke`. Until set, it is the admin.
    pub fn set_keeper(env: Env, keeper: Address) {
        load_config(&env).admin.require_auth();
        env.storage().instance().set(&DataKey::Keeper, &keeper);
    }

    pub fn keeper(env: Env) -> Address {
        load_keeper(&env)
    }

    /// Keeper only: move the anchor to the current clamped quote, so the anchor tracks real
    /// price changes at the policy's rate. Calls closer together than `MIN_POKE_SECS` do
    /// nothing.
    ///
    /// This is not permissionless on purpose. Each poke moves the anchor to the quote, so
    /// whoever pokes controls how fast it compounds. Poked every five minutes under a held
    /// 5%/hour pump, the anchor reaches 3.3x in a day; unpoked, the band stops at the
    /// window and the quote at 2.2x. An attacker must not be able to choose the former.
    /// If the keeper stops, the anchor goes stale, which only makes quotes more cautious.
    pub fn poke(env: Env, asset: Asset) {
        load_keeper(&env).require_auth();
        let Some(anchor) = load_anchor(&env, &asset) else {
            panic_with_error!(&env, OracleError::NoPrice);
        };
        if env.ledger().timestamp().saturating_sub(anchor.timestamp) < MIN_POKE_SECS {
            return;
        }
        let Some(quote) = Self::lastprice(env.clone(), asset.clone()) else {
            panic_with_error!(&env, OracleError::NoPrice);
        };
        store_anchor(&env, &asset, quote.price);
        keep_alive(&env, &asset);
    }

    pub fn policy(env: Env, asset: Asset) -> Option<Policy> {
        env.storage().persistent().get(&DataKey::Policy(asset))
    }

    pub fn anchor(env: Env, asset: Asset) -> Option<Anchor> {
        load_anchor(&env, &asset)
    }

    /// The median before any clamp, for monitoring and for `seed_anchor`.
    pub fn raw_price(env: Env, asset: Asset) -> Option<PriceData> {
        median_price(&env, asset)
    }

    pub fn config(env: Env) -> Config {
        load_config(&env)
    }

    // ---------------------------------------------------------------- SEP-40

    pub fn base(env: Env) -> Asset {
        load_config(&env).base
    }

    pub fn assets(env: Env) -> Vec<Asset> {
        env.storage().instance().get(&DataKey::Assets).unwrap()
    }

    pub fn decimals(env: Env) -> u32 {
        load_config(&env).decimals
    }

    pub fn resolution(env: Env) -> u32 {
        load_config(&env).resolution
    }

    /// Historical lookups are not supported yet; consumers such as Blend only call
    /// `lastprice` and `decimals`.
    pub fn price(_env: Env, _asset: Asset, _timestamp: u64) -> Option<PriceData> {
        None
    }

    pub fn prices(_env: Env, _asset: Asset, _records: u32) -> Option<Vec<PriceData>> {
        None
    }

    /// The median of fresh upstream prices, bounded by the asset's policy when it has one.
    /// `None` when too few upstreams answer, or when a policy exists but no anchor has
    /// been seeded. The returned timestamp is the oldest price used, so staleness checks
    /// downstream stay conservative.
    pub fn lastprice(env: Env, asset: Asset) -> Option<PriceData> {
        let median = median_price(&env, asset.clone())?;
        let Some(policy) = Self::policy(env.clone(), asset.clone()) else {
            return Some(median);
        };
        let anchor = load_anchor(&env, &asset)?;
        let elapsed = env.ledger().timestamp().saturating_sub(anchor.timestamp);
        let price = aggregate::clamp(median.price, anchor.price, elapsed, &policy);
        Some(PriceData {
            price,
            timestamp: median.timestamp,
        })
    }
}

/// Median of all fresh upstream prices, or `None` if fewer than `min_sources`
/// upstreams returned a usable price.
fn median_price(env: &Env, asset: Asset) -> Option<PriceData> {
    let config = load_config(env);
    let sources: Vec<Source> = env.storage().persistent().get(&DataKey::Sources(asset))?;
    let now = env.ledger().timestamp();

    let mut prices = Vec::new(env);
    let mut oldest = u64::MAX;
    for source in sources.iter() {
        let client = UpstreamClient::new(env, &source.oracle);
        // A reverting or malformed upstream must not take the whole feed down.
        let Ok(Ok(Some(p))) = client.try_lastprice(&source.asset) else {
            continue;
        };
        if p.timestamp > now || now - p.timestamp > config.max_age {
            continue;
        }
        let Some(normalized) = aggregate::normalize(p.price, source.decimals, config.decimals)
        else {
            continue;
        };
        prices.push_back(normalized);
        oldest = oldest.min(p.timestamp);
    }

    if prices.len() < config.min_sources {
        return None;
    }
    aggregate::median(&prices).map(|price| PriceData {
        price,
        timestamp: oldest,
    })
}

/// Everything a price lookup reads must outlive a quiet period, or pools would revert once
/// an entry is archived. The keeper's regular pokes carry the expiry forward.
fn keep_alive(env: &Env, asset: &Asset) {
    let persistent = env.storage().persistent();
    for key in [
        DataKey::Sources(asset.clone()),
        DataKey::Policy(asset.clone()),
        DataKey::Anchor(asset.clone()),
    ] {
        if persistent.has(&key) {
            persistent.extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
        }
    }
    env.storage()
        .instance()
        .extend_ttl(TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn load_keeper(env: &Env) -> Address {
    env.storage()
        .instance()
        .get(&DataKey::Keeper)
        .unwrap_or_else(|| load_config(env).admin)
}

fn load_anchor(env: &Env, asset: &Asset) -> Option<Anchor> {
    env.storage()
        .persistent()
        .get(&DataKey::Anchor(asset.clone()))
}

fn store_anchor(env: &Env, asset: &Asset, price: i128) {
    let key = DataKey::Anchor(asset.clone());
    let anchor = Anchor {
        price,
        timestamp: env.ledger().timestamp(),
    };
    env.storage().persistent().set(&key, &anchor);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_THRESHOLD, TTL_EXTEND_TO);
}

fn load_config(env: &Env) -> Config {
    env.storage().instance().get(&DataKey::Config).unwrap()
}
