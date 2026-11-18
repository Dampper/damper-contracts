# 0002. Asymmetric clamp around an anchor

Status: accepted, 2026-11-17

## Context

A median cannot help when several feeds derive from one market. The pool also has to keep working in a real crash: if the oracle hides a drop, liquidations stop and bad debt builds up. Manipulation and crashes need opposite treatment depending on which side of the pool an asset is on. Inflating collateral lets an attacker borrow too much; deflating a debt asset does the same.

## Decision

Each asset can carry a policy: a rate in basis points per hour for upward moves, one for downward moves, and a window. The quote is bounded to `anchor ± anchor × rate × min(elapsed, window) / 1h` in each bounded direction, where `elapsed` is the time since the anchor last moved. A direction can be left unbounded. Collateral assets bound upward moves and leave drops unbounded; debt assets do the reverse; assets used both ways bound both. An asset with a policy returns no price until its anchor is seeded.

## Consequences

- The replay shows the shared-market attack refused, and a 53% crash reaching the pool in full.
- A legitimate rally is rate-limited. That undervalues collateral for a while, which is the conservative direction for the pool.
- The window caps how wide the band can grow when nobody pokes, so a stale anchor cannot become permission for any price.
- The anchor can be walked toward a price that is held for hours. The clamp turns a one-trade exploit into one that costs the attacker time; it does not make manipulation impossible.
- Rates are placeholders until they are fitted to each asset's real volatility.
