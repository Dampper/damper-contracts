# Changelog

All notable changes to this repository are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Nothing has been released yet.

## [Unreleased]

### Added

- Guarded SEP-40 oracle: median over up to seven upstream feeds per asset, skipping stale, future-dated, reverting and non-positive prices, with a minimum quorum.
- Per-asset anchor clamp with a policy in basis points per hour and a window cap. Policies fail closed until the anchor is seeded.
- Keeper role: only the keeper (the admin until `set_keeper`) can call `poke`, which is a no-op within 300 seconds of the last move.
- `poke` extends the expiry of every entry a price lookup reads.
- Replay of the February 2026 YieldBlox exploit against a minimal lending pool, with controls and mutation checks.
- `damper-mock-feed`: admin-settable SEP-40 feed for testnet demos.
- Testnet demo deployment, documented in the README.
