# 0001. Median over fresh upstream feeds, with a quorum

Status: accepted, 2026-11-17

## Context

Blend pools read one SEP-40 oracle and apply only two checks to what it returns: the price is positive and under 24 hours old. In the February 2026 YieldBlox exploit that single oracle followed one manipulated market. The pool cannot be changed, and its oracle address is fixed when it is created, so any protection has to live in the oracle.

## Decision

The oracle reads up to seven upstream feeds per asset and keeps only fresh, positive prices. It requires `min_sources` of them, and returns their median with the oldest timestamp used. An upstream that reverts or returns something unexpected is skipped.

## Consequences

- One manipulated feed out of three cannot move the quote. A replay test shows it.
- That holds only while three feeds answer. With two answering, the median is their average and one manipulated feed moves it by half; with one, it is that feed. Setting `min_sources` to 3 makes the guarantee unconditional, at the price of refusing to quote when a feed drops out.
- Two feeds that read the same thin market are not independent, and a median follows them. That gap is what [0002](0002-asymmetric-anchor-clamp.md) addresses.
- If too few feeds answer, the oracle returns nothing and the pool reverts, including liquidations. We chose refusing to quote over quoting a price built from too little.
- The returned timestamp is the oldest used, so a pool's own staleness check is never fooled by one fresh feed among stale ones.
