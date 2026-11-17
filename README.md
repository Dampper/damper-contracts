# Damper — contracts

Manipulation-resistant price oracle for Blend lending pools on Stellar.

Damper is a SEP-40 price feed that a Blend pool can use as its oracle. It reads several upstream feeds, discards stale or broken ones, takes the median, and (in upcoming milestones) bounds sudden moves against a long-horizon anchor and refuses collateral prices whose market is cheaper to move than the pool is to drain.

Companion repos: [damper-backend](https://github.com/UbsOrg/damper-backend) (keeper and pool scanner) · [damper-frontend](https://github.com/UbsOrg/damper-frontend) (pool risk dashboard).

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
                                    ├─ anchor clamp (per-asset policy)            [M2]
                                    └─ manipulation-cost gate  ◀── keeper          [M2]
```

| Layer | Status | What it does |
|---|---|---|
| Upstream aggregation | done | Up to 7 SEP-40 feeds per asset, each with its own asset identifier and decimals. Reverting, stale, future-dated or non-positive prices are skipped. |
| Quorum | done | Returns `None` unless at least `min_sources` fresh prices exist. |
| Median | done | One manipulated feed out of three cannot move the result. The returned timestamp is the oldest price used. |
| Anchor clamp | planned | Bound each quote against a long-horizon anchor built from upstream history (`prices(asset, n)`), not the previous window. |
| Per-asset policy | planned | Collateral-side assets: cap upward jumps, let drops through so liquidations still work in a real crash. Debt-side assets: the mirror image. Assets used both ways get two-sided bounds sized to market depth. |
| Manipulation-cost gate | planned | The keeper publishes an estimate of what it costs to move each asset's price by x%, from SDEX order-book depth and AMM reserves. If that cost is below what the pool could lose through the asset, the upward clamp tightens to the anchor. |
| YieldBlox replay | planned | Test that replays the February 2026 price path through a Blend pool using Damper and asserts the borrow fails. |

### Open design questions

- **Failing closed vs. open.** Blend unwraps `lastprice`; `None` reverts every pool action touching that asset, including liquidations. The clamp is meant to keep quoting a bounded price rather than return `None`, but the right behaviour when quorum is lost is still open.
- **Anchor source.** Upstream history is cheap to read but inherits upstream manipulation if it lasts longer than the anchor window. Window length vs. responsiveness needs measuring against real price paths.

## Interface

Implements the SEP-40 read interface: `base`, `assets`, `decimals`, `resolution`, `price`, `prices`, `lastprice`. `price` and `prices` (historical lookups) currently return `None`; Blend only calls `lastprice` and `decimals`.

Admin: `set_sources(asset, Vec<(upstream_oracle, upstream_asset)>)`, plus `sources(asset)` and `config()` views.

## Build and test

Requires Rust with the `wasm32v1-none` target and [`stellar-cli`](https://github.com/stellar/stellar-cli).

```sh
cargo test
stellar contract build
```

## Roadmap

- **M1** — pool scanner over mainnet Blend pools (backend), YieldBlox replay test
- **M2** — anchor clamp, per-asset policy, manipulation-cost registry, testnet deployment, keeper live
- **M3** — dashboard, operator configuration UI, external review
- **M4** — NAV-attested price feeds for regulated RWAs (e.g. tokenized treasury funds), so they can be priced without relying on thin DEX markets

## Security

Unaudited and not deployed. Do not use with real funds. Please report vulnerabilities privately through GitHub security advisories on this repository.

## License

Apache-2.0
