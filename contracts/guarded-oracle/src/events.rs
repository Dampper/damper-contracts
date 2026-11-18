//! Events: what an indexer, a dashboard or an auditor can learn without reading storage.
//!
//! Every change to what the oracle quotes, and every change to who controls it, emits one
//! event. Each carries the asset (where there is one) as a topic so a listener can follow a
//! single asset, and its data is a map of named fields. `lastprice` emits nothing: it is a
//! view that pools call on every action, and an event there would cost every user a fee.
use soroban_sdk::{contractevent, Address};

use crate::types::Asset;

/// Admin replaced the upstream feeds for an asset.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourcesSet {
    #[topic]
    pub asset: Asset,
    pub count: u32,
}

/// Admin set the clamp rates for an asset.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicySet {
    #[topic]
    pub asset: Asset,
    pub up_bps_per_hour: u32,
    pub down_bps_per_hour: u32,
    pub window_secs: u64,
}

/// Admin set the cost gate for an asset.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GateSet {
    #[topic]
    pub asset: Asset,
    pub tight_up_bps_per_hour: u32,
    pub tight_down_bps_per_hour: u32,
    pub min_cost_ratio_bps: u32,
    pub max_evidence_age: u64,
}

/// Admin changed who may poke and publish evidence.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeeperSet {
    pub keeper: Address,
}

/// Admin set an asset's anchor to the current unclamped median: the one moment the clamp
/// trusts the upstreams outright.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnchorSeeded {
    #[topic]
    pub asset: Asset,
    pub price: i128,
}

/// The keeper moved an asset's anchor to the clamped quote. Only emitted when the anchor
/// actually moved: a poke inside the minimum interval does nothing and says nothing.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Poked {
    #[topic]
    pub asset: Asset,
    pub from: i128,
    pub to: i128,
    /// Seconds the old anchor had stood.
    pub after_secs: u64,
}

/// The keeper reported what moving an asset's price costs and what it would unlock.
#[contractevent]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidencePublished {
    #[topic]
    pub asset: Asset,
    pub up_cost: i128,
    pub up_exposure: i128,
    pub down_cost: i128,
    pub down_exposure: i128,
}
