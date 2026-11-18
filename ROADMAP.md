# Roadmap

Status of the contracts. The repository README carries the design; this file tracks what is done and what is next.

## Done

- **M1.** Median and quorum over fresh upstream feeds, anchor clamp with per-asset policy, keeper-only anchor updates, YieldBlox replay, testnet demo deployment.

## Next

- **Manipulation-cost gate.** The keeper publishes an estimate of what it costs to move each asset's price; the oracle tightens its upward rate when that cost is below what the pool could lose through the asset. Needs an on-chain registry and a keeper write path.
- **Events.** The oracle emits none yet. Anchor moves, policy changes and keeper changes should be observable without reading storage.
- **Historical lookups.** `price` and `prices` return nothing. Anything that needs time-weighted prices from Damper itself will need an on-chain history.
- **Fuzz and invariant tests** for the clamp: a quote never exceeds `anchor × (1 + rate × window)` in a bounded direction, whatever the upstreams report.
- **Testnet with a real Blend pool** using Damper as its oracle, then an external review.

## Later

- NAV-attested feeds for regulated real-world assets, so they can be priced without relying on thin DEX markets.
