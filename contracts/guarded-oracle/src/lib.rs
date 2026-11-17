#![no_std]
//! Damper guarded oracle.
//!
//! A SEP-40 price feed that sits between a lending pool and its upstream oracles.
//! This first version takes the median of several fresh upstream prices and refuses
//! to quote when too few are available. The anchor-relative clamp and the
//! manipulation-cost gate described in the README are built on top of this.

mod aggregate;
mod types;

#[cfg(test)]
mod test;

use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, panic_with_error, Address, Env, Vec,
};

pub use types::{Asset, Config, OracleError, PriceData, Source};

pub const MAX_SOURCES: u32 = 7;
const MAX_DECIMALS: u32 = 18;

const DAY_LEDGERS: u32 = 17_280;
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
enum DataKey {
    Config,
    Assets,
    Sources(Asset),
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

    /// Median of all fresh upstream prices, or `None` if fewer than `min_sources`
    /// upstreams returned a usable price. The returned timestamp is the oldest of the
    /// prices used, so staleness checks downstream stay conservative.
    pub fn lastprice(env: Env, asset: Asset) -> Option<PriceData> {
        let config = load_config(&env);
        let sources: Vec<Source> = env.storage().persistent().get(&DataKey::Sources(asset))?;
        let now = env.ledger().timestamp();

        let mut prices = Vec::new(&env);
        let mut oldest = u64::MAX;
        for source in sources.iter() {
            let client = UpstreamClient::new(&env, &source.oracle);
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
}

fn load_config(env: &Env) -> Config {
    env.storage().instance().get(&DataKey::Config).unwrap()
}
