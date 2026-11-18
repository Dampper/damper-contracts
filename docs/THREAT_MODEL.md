# Threat model

What each party can and cannot do, and which risks are still open. Damper is unaudited and runs on testnet only.

## What it defends against

A lending pool that reads one price feed lends against whatever that feed says. The reference incident: on 2026-02-22 USTRY was pushed from about $1.06 to $106.74 on a market with almost no volume, the feed Blend's YieldBlox pool used followed it, and the pool lent about $10.2M against collateral worth about 1% of that. Damper's goal is that a price a pool reads cannot rise further or faster than a policy allows, without hiding real drops.

## Parties

| Party | Trust | Can | Cannot |
| --- | --- | --- | --- |
| Admin | trusted | choose sources and policies, seed anchors, set the keeper | move a quote past what the policy allows between pokes; call `poke` unless it is also the keeper |
| Keeper | trusted to be live, not to be honest | call `poke`, which moves an anchor at most at the policy's rate | change sources, policies or the keeper; move the anchor faster by poking more often (pokes within 300 s do nothing) |
| Upstream feeds | untrusted individually, trusted as a majority | report prices | single-handedly set the median while at least three report |
| Pool | consumer | read prices | write anything |
| Anyone | untrusted | read | call any state-changing function |

## Risks and how they are handled

| # | Risk | Status |
| --- | --- | --- |
| 1 | **One feed is manipulated.** | Mitigated while at least three feeds answer: the median ignores a single outlier. If only two answer, the median is their average and one manipulated feed moves it by half; with one, it is that feed. Set `min_sources` to 3 to make the guarantee unconditional, at the price of refusing to quote when a feed drops out. |
| 2 | **Several feeds read the same thin market.** A median follows them. | Mitigated by the anchor clamp: a replay shows the same attack refused. Not eliminated, see 3. |
| 3 | **A pump held for hours.** The clamp slows the quote, and a keeper that keeps poking walks the anchor up at the policy rate. | Open. In the replay a 24-hour held pump lets an attacker borrow about 3% of the incident's amount. The planned manipulation-cost gate and the keeper's `hold` rule address it. |
| 4 | **Anyone pokes to compound the anchor.** Poked every five minutes the anchor reaches 3.3x in a day under a held 5%/hour pump, against 2.2x unpoked. | Closed: `poke` is keeper-only ([ADR 0003](adr/0003-keeper-only-anchor-updates.md)). |
| 5 | **Keeper stops.** | Accepted: the anchor stops moving and quotes get more cautious, never less. |
| 6 | **Keeper key stolen.** | Bounded: the attacker can only poke, at the policy's rate, and can be replaced with `set_keeper`. |
| 7 | **Seeding during a manipulation.** `seed_anchor` takes the unclamped median. | Open: operational. Seed when the market is calm and check `raw_price` first. |
| 8 | **Quorum lost or all feeds stale.** `lastprice` returns `None`, and a pool that unwraps it reverts every action on that asset, including liquidations. | Open and deliberate: refusing to quote is safer than quoting a price built from one feed. The keeper reports `no-quote` as critical. |
| 9 | **A legitimate rally is rate-limited.** Collateral is undervalued for a while. | Accepted: the conservative direction for the pool; the cost falls on borrowers. |
| 10 | **A real crash.** | Handled: collateral policies leave drops unbounded, so liquidations see the real price. |
| 11 | **Storage archived.** Lookups would fail. | Mitigated: `poke` extends every entry a lookup reads. Without a keeper the oracle stops quoting after about 60 days. |
| 12 | **Admin key compromised.** | Open: the admin can repoint sources or loosen a policy. A timelock or multisig on the admin is not built. |
| 13 | **Rounding.** | Handled: the band is one division, saturating, and tested at short intervals and huge anchors. |
| 14 | **A malicious upstream that panics or returns a wrong type.** | Handled: `try_lastprice` skips it. |

## Out of scope

Blend's own contracts and parameters, upstream oracle implementations, and the testnet mock feed.
