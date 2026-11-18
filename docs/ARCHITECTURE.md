# Architecture

Damper is one Soroban contract, `damper-guarded-oracle`, plus a test-only mock feed. A lending pool uses it as its oracle: the pool calls `lastprice(asset)` and `decimals()` and nothing else.

```
 pool ──lastprice(asset)──▶ ┌────────────────────── guarded oracle ──────────────────────┐
                            │ 1  read each upstream feed (SEP-40), skip bad ones          │
                            │ 2  drop stale, future-dated, non-positive prices            │
                            │ 3  require min_sources fresh prices, take the median        │
                            │ 4  if the asset has a policy: bound the median around the   │
                            │    stored anchor (needs a seeded anchor, else no price)     │
                            └──────────────┬───────────────────────────▲──────────────────┘
                                           │ reads                      │ poke(asset)
                       upstream feeds ◀────┘                      keeper (off-chain)
```

## Contract surface

| Function | Caller | What it does |
| --- | --- | --- |
| `__constructor(admin, base, decimals, resolution, max_age, min_sources)` | deployer | Fixes the quote asset, price precision, freshness window and quorum |
| `set_sources(asset, [(upstream, upstream_asset)])` | admin | Replaces an asset's upstream feeds (at most 7, at least `min_sources`); reads and stores each feed's decimals |
| `set_policy(asset, policy)` | admin | Sets how fast the quote may move away from the anchor |
| `seed_anchor(asset)` | admin | Sets the anchor to the current unclamped median |
| `set_keeper(address)` | admin | Chooses who may call `poke` |
| `poke(asset)` | keeper | Moves the anchor to the current quote; no-op within 300 s of the last move |
| `lastprice`, `decimals`, `base`, `assets`, `resolution` | anyone | SEP-40 reads |
| `raw_price`, `policy`, `anchor`, `keeper`, `sources`, `config` | anyone | Monitoring views |
| `price`, `prices` | anyone | Historical SEP-40 reads; return nothing yet |

## Pricing

[`lastprice`](contracts/guarded-oracle/src/lib.rs) works in two steps.

**Median** (`median_price`). Each upstream is called with `try_lastprice`, so one that reverts or returns garbage is skipped rather than taking the feed down. A price is kept if it is not in the future, is no older than `max_age`, and is still positive after rescaling from the upstream's decimals to the oracle's. With fewer than `min_sources` survivors the answer is `None`. Otherwise it is the median (the mean of the two middle values for an even count), and the timestamp is the **oldest** price used, so a pool's own staleness check stays conservative.

**Clamp** (`aggregate::clamp`). With a policy, the median is bounded to

```
anchor × (1 + up_rate × min(elapsed, window) / 1h)    upper bound, if up_rate is bounded
anchor × (1 − down_rate × min(elapsed, window) / 1h)  lower bound, if down_rate is bounded, never below 1
```

where `elapsed` is the time since the anchor last moved and rates are in basis points per hour. The band is computed in one division so short intervals do not round to zero, and saturates instead of overflowing. A policy without a seeded anchor returns `None`.

## Storage

| Key | Storage | Written by |
| --- | --- | --- |
| `Config`, `Assets`, `Keeper` | instance | constructor, `set_sources`, `set_keeper` |
| `Sources(asset)` | persistent | `set_sources` |
| `Policy(asset)` | persistent | `set_policy` |
| `Anchor(asset)` | persistent | `seed_anchor`, `poke` |

Soroban archives entries whose time-to-live runs out, and a price lookup that touches an archived entry fails. Writes extend the entry they touch, and `poke` extends everything a lookup reads (`keep_alive`), so a running keeper carries the expiry forward. An oracle with no keeper and no admin activity for about 60 days would stop quoting until its entries are restored.

## Roles

- **Admin** chooses sources, policies and the keeper, and seeds anchors. Trusted: see the [threat model](THREAT_MODEL.md).
- **Keeper** only calls `poke`. It holds no other power, so a stolen keeper key can move an anchor at the policy's rate and nothing more.
- **Pools and everyone else** only read.

## Not built yet

Events, historical `price` / `prices`, and the manipulation-cost gate. See the [roadmap](../ROADMAP.md).

## Decisions

The reasoning behind the main choices is in [`docs/adr`](adr/README.md).
