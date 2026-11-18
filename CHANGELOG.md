# Changelog

All notable changes to this repository are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Nothing has been released yet.

## [Unreleased]

### Added

- Guarded SEP-40 oracle: median over up to seven upstream feeds per asset, skipping stale, future-dated, reverting and non-positive prices, with a minimum quorum.
- Per-asset anchor clamp with a policy in basis points per hour and a window cap. Policies fail closed until the anchor is seeded.
- Keeper role: only the keeper (the admin until `set_keeper`) can call `poke`, which is a no-op within 300 seconds of the last move.
- Manipulation-cost gate: `set_gate`, `publish_evidence` (keeper only), `effective_policy`. A bounded direction keeps its policy rate only while fresh evidence says an attack is uneconomic, otherwise it uses a tight rate. It can only tighten and never bounds a free direction.
- Property tests and a stateful fuzz test (`proptest`): the clamp, the gate, the median and the decimals are checked for every input, and the deployed contract's invariants are checked after every step of random operation sequences. A weekly workflow runs a deeper search.
- Events for every state change (`sources_set`, `policy_set`, `gate_set`, `anchor_seeded`, `poked`, `evidence_published`, `keeper_set`), so anchor moves and configuration changes can be followed without reading storage. `lastprice` stays silent.
- `poke` extends the expiry of every entry a price lookup reads.
- Replay of the February 2026 YieldBlox exploit against a minimal lending pool, with controls and mutation checks.
- `damper-mock-feed`: admin-settable SEP-40 feed for testnet demos.
- Testnet demo deployment, documented in the README.
