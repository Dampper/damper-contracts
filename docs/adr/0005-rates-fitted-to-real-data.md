# 0005. Fit the clamp and gate rates to measured data, not placeholders

Status: accepted, 2026-11-24

## Context

[0002](0002-asymmetric-anchor-clamp.md) and [0004](0004-cost-gate-tightens-never-loosens.md) left their numbers as placeholders: a 500 bps/hour policy rate, a 50 bps/hour tight rate, and a 2x cost-to-exposure ratio, picked to make the testnet demo legible, not derived from anything. A rate set too low clamps genuine price moves, which is the wrong failure for a collateral asset (it just undervalues collateral, conservative but costly); a rate set too high gives an attacker room. Neither number had ever been checked against how XLM/USDC actually moves.

## What the real data showed

180 days of Stellar mainnet `trade_aggregations` for XLM/USDC (Horizon, hourly resolution, 4,000 buckets), using the volume-weighted price per bucket rather than its high/low:

| Percentile of hour-over-hour \|move\| | Value |
| --- | --- |
| p50 | 0.34% |
| p90 | 1.15% |
| p99 | 2.85% |
| p99.9 | 5.52% |
| max (of 4,000 hours) | 7.07% |

Two things fell out of pulling this:

- **The placeholder tight rate (50 bps/hour = 0.5%/hour) sat *below* the p90 figure (1.15%/hour).** One hour in ten, completely ordinary XLM volatility would already have been clamped by what was meant to be an "attack is cheap, be careful" rate — the tight rate was tighter than real markets, before any manipulation entered the picture.
- **The placeholder policy rate (500 bps/hour = 5%/hour) sat *below* the single worst hour observed in 4,000 (7.07%/hour).** A genuine, unmanipulated crash or rally at the top of this real distribution would have been rate-limited by the "normal" rate, not just the tight one.

Pulling the raw high/low instead of the VWAP first gave hourly ranges up to 515% — not real volatility, but a recurring artifact: a resting SDEX offer at exactly 1.0 XLM/USDC (an old, essentially dust-sized peg offer, nowhere near the real ~0.17–0.18 price) gets hit for a few dollars every so often and prints as that bucket's high, while the hour's actual trading and its VWAP stay unmoved. It is a second, independent demonstration of the project's central claim, on top of the YieldBlox replay: a single thin print is not a price, and anything that reads one as though it were will get burned by exactly this kind of leftover resting offer, not only by someone actively attacking it.

## Decision

The testnet demo (`CCBCV5NCMEDCW4WPWF45P36JXZ7VV3QJR5T4X3CICJPGRYRDXKH636GQ`, XLM) was moved to:

- **Policy rate: 1,500 bps/hour (15%/hour).** About 2x the worst of 4,000 real hours — comfortably clear of real volatility's tail, including the kind of single-print artifact above, while still bounding a sustained attack far below what unlimited manipulation could do (the YieldBlox replay and the gate's own market-cost evidence both show real attacks priced in the hundreds of thousands to tens of millions of dollars for a 10x move, not 15%).
- **Tight rate: 150 bps/hour (1.5%/hour).** A little above the p90 figure (1.15%/hour) — typical organic movement still gets through even while the gate is tight — and exactly 10x below the new policy rate, so the gate has real teeth once it engages.
- **Cost ratio: 10x (`min_cost_ratio_bps` 100,000, up from 20,000).** Not derived from this volatility data; aligned instead to [damper-backend's own `tier()`](https://github.com/Dampper/damper-backend/blob/main/src/score.ts) function, which already classifies a reserve as `low` risk exactly when its measured cost is at least 10x its exposure. The gate now keeps the fast rate only when fresh evidence clears the same bar this project already uses everywhere else to call something low risk, rather than a separately-invented number.

These three apply to this one asset on testnet; they are not contract constants. A pool operator sets its own per-asset policy and gate, and should run the same kind of analysis — real trade history for that asset's volatility, the scanner's own tiering for the cost ratio — rather than copy these numbers for something with a different liquidity profile.

The change was made live against the deployed testnet oracle (`set_policy` at ledger 5,891,654, `set_gate` at ledger 5,891,656), and the demo walkthrough in the README was re-run from a fresh anchor under the new rates rather than edited to match; see the README for the resulting ledgers and quotes.

## Consequences

- The demo's numbers are now explicable from outside the project: anyone can pull the same Horizon data and check them, rather than trusting a chosen-for-legibility round number.
- The tight rate no longer binds on ordinary volatility (previously true about 10% of the time); the policy rate no longer falls short of a real crash the project's own data already contains one instance of.
- The cost ratio is the same number this project already treats as "low risk" to a scanned pool, not a second, independently-chosen threshold that happens to disagree with the first.
- This is one asset's numbers from one market. A different asset's volatility, or XLM's volatility after a regime change, would need its own pull of this analysis; nothing here is permanent.
