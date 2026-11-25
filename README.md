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
- [Testing](docs/TESTING.md): unit tests, the replay, property tests and stateful fuzzing, and what each checks
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
| Manipulation-cost gate | done | Per asset, the keeper publishes what it costs to push the price up or down about 10x and what that would unlock. A bounded direction keeps its policy rate only while fresh evidence says the attack is uneconomic; otherwise it falls to a tight rate. It can only tighten, and never bounds a direction the policy leaves free. |
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
| Cost gate, held pump on a thin market | at most about $122k borrowable after 24 h, against about $350k without the gate |
| Cost gate, same pump on a deep market | identical to no gate: it only acts where manipulation is cheap |
| Cost gate, evidence missing or stale | tight rate applies |
| Cost gate, best evidence a compromised keeper could publish | returns the policy rate, never looser |
| Real crash (−53% in one round) | reaches the pool in full, so liquidations still work |
| Policy set, anchor not seeded | no price; borrows revert |

The fourth row is the reason the clamp exists: a median cannot help when several feeds derive from the same thin market. Mutation checks confirm the tests can fail (16 deliberate bugs, each caught, except one that no input can distinguish; two were caught only by the fuzz tests; see [docs/TESTING.md](docs/TESTING.md)). Earlier checks: disabling the clamp breaks 4 replay tests, replacing the median with "first source" breaks 3, and for the gate, ignoring it breaks 3, keeping the policy rate on missing evidence breaks 7, letting the tight rate loosen a policy breaks 1, and never expiring evidence breaks 2.

### Open design questions

- **Failing closed vs. open.** Blend unwraps `lastprice`; `None` reverts every pool action touching that asset, including liquidations. The clamp is meant to keep quoting a bounded price rather than return `None`, but the right behaviour when quorum is lost is still open.
- **Sustained manipulation drags the anchor.** The clamp turns a one-trade exploit into one that costs the attacker a day of holding a pumped price. It does not make manipulation impossible. The cost gate tightens the rate where holding a price is cheap (about $122k borrowable after 24 hours on a thin market, against about $350k), but it rests on an estimate of cost, and a deep market can be thinned.
- **Rates need real data.** The testnet demo's XLM rates are now fitted to 180 days of real XLM/USDC trade history and the backend scanner's own risk tiers rather than picked for legibility; see [ADR 0005](docs/adr/0005-rates-fitted-to-real-data.md). A different asset, or a different market regime, needs its own pull of that analysis — nothing here is permanent, and these numbers are a demo's, not a contract default.
- **`poke` is not permissionless.** Each poke moves the anchor to the current quote, so whoever pokes chooses how fast the anchor compounds. Under a held 5%/hour pump, poking every five minutes takes the anchor to 3.3x in a day, while an unpoked band stops at 2.2x. Letting an attacker choose the first would hand them the faster path, so only a configured keeper can poke. If the keeper stops, the anchor goes stale and quotes get more cautious.
- **Seeding is a trust point.** `seed_anchor` takes the unclamped median, so it must run while the market is calm.
- **Legitimate fast rallies.** An asset that really doubles in an hour will be undervalued as collateral for a while. That is the conservative direction for the pool and the cost to borrowers.

## Interface

Implements the SEP-40 read interface: `base`, `assets`, `decimals`, `resolution`, `price`, `prices`, `lastprice`. `price` and `prices` (historical lookups) currently return `None`; Blend only calls `lastprice` and `decimals`.

Admin: `set_sources(asset, Vec<(upstream_oracle, upstream_asset)>)`, `set_policy(asset, policy)`, `seed_anchor(asset)`, `set_keeper(address)`, `set_gate(asset, gate)`. Keeper (the admin until `set_keeper` is called): `poke(asset)`, `publish_evidence(asset, up_cost, up_exposure, down_cost, down_exposure)`. Views: `sources`, `config`, `policy`, `anchor`, `keeper`, `gate`, `evidence`, `effective_policy` (the policy after the gate), `raw_price` (the median before any clamp).

## Build and test

The tests run in four layers: unit tests, the YieldBlox replay, property tests that check rules for every input, and a stateful fuzz test that throws random sequences of events at the deployed contract and checks its invariants after every step ([docs/TESTING.md](docs/TESTING.md)). A weekly workflow runs a much deeper search of these by random sampling, and separately, `cargo-fuzz` targets explore the clamp and the gate by code path instead ([ADR 0006](docs/adr/0006-coverage-guided-fuzzing.md)).

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
| Guarded oracle, with the cost gate | `CCBCV5NCMEDCW4WPWF45P36JXZ7VV3QJR5T4X3CICJPGRYRDXKH636GQ` |
| Earlier guarded oracle, before the gate | `CBJLNEAOIIGMAQNWG7UZYMDLD5CR7ZTCFN3NILSWCY544WFHN65TBJQT` |
| Mock feeds | `CBEQGH2Y7YQWP3P4UYTQVTEBJ6TE7EB4LA73XUOXJZLME2MGSLCYGJTP`, `CABBBBRPUJLOQZLHBU5UUYNHXMZDURIY6I3Q44QYZSAHY6LOTEPD7QUE`, `CCOW6QR5MUQ5TYK6PAJZDOWCXROLNXWEIUGGTZ42SQL47L7ESSPDZSFB` |

It prices the testnet XLM contract with a 15%/hour upward policy, a cost gate (tight rate 1.5%/hour, cost must be at least 10x the exposure) and a separate keeper address. These rates replaced an earlier 5%/hour / 0.5%/hour / 2x placeholder once fitted to real XLM/USDC trade history and the backend's own risk tiers; see [ADR 0005](docs/adr/0005-rates-fitted-to-real-data.md). What was run against them with the [keeper](https://github.com/Dampper/damper-backend#keeper):

- on the earlier oracle, the keeper's poke landed on ledger 5,791,954 once the anchor was past the 300 s minimum;
- on the earlier oracle, with all three feeds set to $106.74, `raw_price` returned $106.74 while `lastprice` returned $1.0624 (the $1.06 anchor plus the allowance for 160 s), and the keeper held its pokes and raised a critical `suspicious-move` alert;
- on the earlier oracle, a poke signed by the admin instead of the keeper is refused: the contract asks for the keeper's signature;
- with the rates refitted (`set_policy` at ledger 5,891,654, `set_gate` at ledger 5,891,656) and the anchor reseeded to $1.06 at ledger 5,891,679, all three feeds were pumped to $106.74 (ledger 5,891,684). With no fresh evidence the effective upward rate is the tight 1.5%/hour;
- the keeper then read XLM's real mainnet order book (about $403.5k to push the price up 10x), compared it with a $50M exposure (ratio 8.1x, under the new 10x bar) and published that evidence at ledger 5,891,698, raising `gate-tight`; the quote stayed on the tight rate;
- fresh evidence against a $30k exposure instead (ratio 134x, clearing the bar) was published at ledger 5,891,708. The effective rate returned to the policy's 15%/hour and the quote read $1.0664041 — the keeper's own evidence-freshness guard correctly refused to republish over evidence it had just sent as still fresh, so this step used `publish_evidence` directly as the keeper key instead of a second keeper pass.

## Roadmap

- **M1** — pool scanner over mainnet Blend pools (backend) ✓, YieldBlox replay test ✓, anchor clamp ✓
- **M2** — manipulation-cost gate ✓, keeper (anchor pokes and evidence), testnet deployment
- **M3** — dashboard, operator configuration UI, external review
- **M4** — NAV-attested price feeds for regulated RWAs (e.g. tokenized treasury funds), so they can be priced without relying on thin DEX markets

## Security

Unaudited and not deployed. Do not use with real funds. Please report vulnerabilities privately through GitHub security advisories on this repository.

## License

Apache-2.0
