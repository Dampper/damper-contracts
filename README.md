<p align="center">
  <img src="https://raw.githubusercontent.com/Dampper/.github/main/profile/damper-wordmark.png" alt="Damper" width="420">
</p>

# Damper Contracts

[![CI](https://github.com/Dampper/damper-contracts/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/Dampper/damper-contracts/actions/workflows/ci.yml) [![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE) ![Stellar testnet](https://img.shields.io/badge/network-Stellar%20testnet-informational)

Manipulation-resistant price oracle for Blend lending pools on Stellar.

Damper is a SEP-40 price feed that a Blend pool can use as its oracle. It reads several upstream feeds, discards stale or broken ones, takes the median, and (in upcoming milestones) bounds sudden moves against a long-horizon anchor and refuses collateral prices whose market is cheaper to move than the pool is to drain.

Companion repos: [damper-backend](https://github.com/Dampper/damper-backend) (keeper and pool scanner) · [damper-frontend](https://github.com/Dampper/damper-frontend) (pool risk dashboard).

More documentation:

- [Architecture](docs/ARCHITECTURE.md): contract surface, pricing, storage and roles
- [Threat model](docs/THREAT_MODEL.md): what each party can and cannot do, and which risks are still open
- [Decision records](docs/adr/README.md): why it is built this way
- [Roadmap](ROADMAP.md) and [changelog](CHANGELOG.md)

## Why

On 2026-02-22 the YieldBlox DAO pool on Blend V2 lost about $10.2M. The pool accepted USTRY as collateral, priced by Reflector from trades on the Stellar DEX. The USTRY/USDC market had almost no volume, so a single trade moved the price from about $1.06 to $106.74, and the pool lent against the inflated value ([BlockSec](https://blocksec.com/blog/yieldblox-dao-incident-on-stellar-oracle-misconfiguration-enabled-a-10m-drain), [Rekt](https://rekt.news/yieldblox-rekt)). About $7.3M was only contained because Stellar validators quarantined the attacker's accounts ([Blockaid](https://blockaid.io/blog/73-quarantined-how-blockaid-and-stellar-validators-contained-a-10m-price-manipulation-attack)).

What made it possible, and is still true of Blend V2 today:

- The pool calls `lastprice` on a single SEP-40 contract. Its only checks are that the price is positive and under 24 hours old ([`pool.rs`](https://github.com/blend-capital/blend-contracts-v2/blob/main/pool/src/pool/pool.rs)).
- The adapter in front of Reflector passed the latest price through with no median, and its deviation check compared each price to the previous window, which had also been manipulated.
- The oracle address is set in the pool constructor and cannot be changed afterwards, so protection has to live in the oracle contract a pool is created with (or in an upgradable adapter it already points to).

## Design

```
 Blend pool ──lastprice(asset)──▶ Damper guarded oracle
                                    ├─ upstream feeds (SEP-40): Reflector DEX, Reflector CEX/FX, AMM TWAP adapters
                                    ├─ freshness + quorum filter
                                    ├─ median
                                    ├─ anchor clamp (per-asset policy)   ◀── keeper pokes the anchor
                                    └─ manipulation-cost gate  ◀── keeper          [M2]
```

| Layer | Status | What it does |
|---|---|---|
| Upstream aggregation | done | Up to 7 SEP-40 feeds per asset, each with its own asset identifier and decimals. Reverting, stale, future-dated or non-positive prices are skipped. |
| Quorum | done | Returns `None` unless at least `min_sources` fresh prices exist. |
| Median | done | One manipulated feed out of three cannot move the result. The returned timestamp is the oldest price used. |
| Anchor clamp | done | Per asset, the quote may sit at most `anchor × (1 ± rate × min(elapsed, window) / 1h)` away from a stored anchor. Collateral assets bound upward moves and let drops through; debt assets the reverse. The anchor moves only through `poke`, which only the keeper address can call and which does nothing within 300 s of the last move. |
| Per-asset policy | done | `set_policy(asset, {up_bps_per_hour, down_bps_per_hour, window_secs})`; `UNBOUNDED` disables a direction. A policy without a seeded anchor returns no price (fails closed). |
| Manipulation-cost gate | planned | The keeper publishes an estimate of what it costs to move each asset's price by x%, from SDEX order-book depth and AMM reserves. If that cost is below what the pool could lose through the asset, the upward rate tightens. |
| YieldBlox replay | done | See below. Runs against a minimal lending pool, not Blend itself. |

### YieldBlox replay

[`replay.rs`](contracts/guarded-oracle/src/replay.rs) feeds the incident's price path ($1.06 pushed to $106.74) through a minimal lending pool that values collateral with `oracle.lastprice(asset).unwrap()` and allows a borrow while `debt × liability factor ≤ collateral × collateral factor`, the same call and check Blend V2 makes. It is not Blend: contract amounts are illustrative, sized like the incident (61.2M XLM and 1M USDC, about $10.8M, against 125,000 USTRY).

| Setup | Result |
|---|---|
| Pool reads one pumped DEX feed (the incident) | drained |
| Same pool at the real price | refuses the drain, allows an honest $50k borrow |
| Damper, median of 3 feeds, only the DEX feed pumped | refused; quote stays at the median ($1.08) |
| Damper, median only, two of three feeds read the same pumped market | **drained** |
| Damper, median + anchor clamp, all feeds pumped | refused; quote is $1.0644, anchor + 41.67 bps after 300 s |
| Same, pump held 24 h, keeper poking hourly | never drained; at most about $350k borrowable at the end, 3% of the incident |
| No keeper for a month | band stops widening at the window; quote capped at $2.33 |
| Real crash (−53% in one round) | reaches the pool in full, so liquidations still work |
| Policy set, anchor not seeded | no price; borrows revert |

The fourth row is the reason the clamp exists: a median cannot help when several feeds derive from the same thin market. Mutation checks confirm the tests can fail: disabling the clamp breaks 4 replay tests, and replacing the median with "first source" breaks 3.

### Open design questions

- **Failing closed vs. open.** Blend unwraps `lastprice`; `None` reverts every pool action touching that asset, including liquidations. The clamp is meant to keep quoting a bounded price rather than return `None`, but the right behaviour when quorum is lost is still open.
- **Sustained manipulation drags the anchor.** The clamp turns a one-trade exploit into one that costs the attacker a day of holding a pumped price, with the keeper poking the anchor along at the policy rate. It does not make manipulation impossible. The manipulation-cost gate is meant to tighten the rate when holding the price is cheap.
- **Rates need real data.** 500 bps per hour is a placeholder. Reasonable rates depend on each asset's historical volatility; the backend scanner is where those measurements will come from.
- **`poke` is not permissionless.** Each poke moves the anchor to the current quote, so whoever pokes chooses how fast the anchor compounds. Under a held 5%/hour pump, poking every five minutes takes the anchor to 3.3x in a day, while an unpoked band stops at 2.2x. Letting an attacker choose the first would hand them the faster path, so only a configured keeper can poke. If the keeper stops, the anchor goes stale and quotes get more cautious.
- **Seeding is a trust point.** `seed_anchor` takes the unclamped median, so it must run while the market is calm.
- **Legitimate fast rallies.** An asset that really doubles in an hour will be undervalued as collateral for a while. That is the conservative direction for the pool and the cost to borrowers.

## Interface

Implements the SEP-40 read interface: `base`, `assets`, `decimals`, `resolution`, `price`, `prices`, `lastprice`. `price` and `prices` (historical lookups) currently return `None`; Blend only calls `lastprice` and `decimals`.

Admin: `set_sources(asset, Vec<(upstream_oracle, upstream_asset)>)`, `set_policy(asset, policy)`, `seed_anchor(asset)`, `set_keeper(address)`. Keeper (the admin until `set_keeper` is called): `poke(asset)`. Views: `sources`, `config`, `policy`, `anchor`, `keeper`, `raw_price` (the median before any clamp).

## Build and test

`contracts/mock-feed` is an admin-settable SEP-40 feed for testnet demos and keeper tests; it must never back a real pool.

Requires Rust with the `wasm32v1-none` target and [`stellar-cli`](https://github.com/stellar/stellar-cli).

```sh
cargo test
stellar contract build
```

## Testnet demo deployment

A demo deployment on Stellar testnet, with three admin-settable mock feeds standing in for upstream oracles. Throwaway keys, no real value.

| Contract | Address |
|---|---|
| Guarded oracle | `CBJLNEAOIIGMAQNWG7UZYMDLD5CR7ZTCFN3NILSWCY544WFHN65TBJQT` |
| Mock feeds | `CBEQGH2Y7YQWP3P4UYTQVTEBJ6TE7EB4LA73XUOXJZLME2MGSLCYGJTP`, `CABBBBRPUJLOQZLHBU5UUYNHXMZDURIY6I3Q44QYZSAHY6LOTEPD7QUE`, `CCOW6QR5MUQ5TYK6PAJZDOWCXROLNXWEIUGGTZ42SQL47L7ESSPDZSFB` |

It prices the testnet XLM contract with a 5%/hour upward policy and a separate keeper address. What was run against it with the [keeper](https://github.com/Dampper/damper-backend#keeper):

- the keeper's poke landed on ledger 5,791,954 once the anchor was past the 300 s minimum;
- with all three feeds set to $106.74, `raw_price` returned $106.74 while `lastprice` returned $1.0624 (the $1.06 anchor plus the allowance for 160 s), and the keeper held its pokes and raised a critical `suspicious-move` alert;
- a poke signed by the admin instead of the keeper is refused: the contract asks for the keeper's signature.

## Roadmap

- **M1** — pool scanner over mainnet Blend pools (backend) ✓, YieldBlox replay test ✓, anchor clamp ✓
- **M2** — manipulation-cost registry, keeper (anchor pokes and cost estimates), testnet deployment
- **M3** — dashboard, operator configuration UI, external review
- **M4** — NAV-attested price feeds for regulated RWAs (e.g. tokenized treasury funds), so they can be priced without relying on thin DEX markets

## Security

Unaudited and not deployed. Do not use with real funds. Please report vulnerabilities privately through GitHub security advisories on this repository.

## License

Apache-2.0
