# 0004. A cost gate that only tightens

Status: accepted, 2026-11-18

## Context

The clamp ([0002](0002-asymmetric-anchor-clamp.md)) turns a one-trade exploit into one that costs time, but its rate is a fixed number. A rate that is right for a deep, liquid market is too loose for a market a single trader can move, and one that is right for the thin market is too tight for everything else. The scanner can measure the difference: the capital needed to push a price up or down about 10x, and what that would let an attacker borrow.

That evidence comes from off-chain data, so it has to come from the keeper. A keeper that can say anything must not be able to make the oracle less safe than the policy the admin chose.

## Decision

Per asset, an optional **gate** carries a tight rate for each direction, a required cost ratio (pushing the price must cost at least that multiple of what it unlocks) and a maximum evidence age. The keeper publishes **evidence**: cost and exposure for each direction, stamped with the ledger time.

A bounded direction keeps the policy rate only while fresh evidence says an attack is uneconomic. In every other case (no evidence, stale evidence, evidence from the future, an attack that would pay) it uses `min(policy rate, tight rate)`. A direction the policy leaves unbounded stays unbounded, so collateral can still fall freely and liquidations keep working. A cost of `UNLIMITED` means trading cannot move the price (for example an off-exchange feed) and counts as uneconomic whatever the exposure.

## Consequences

- **The gate can only tighten.** The best evidence a compromised keeper can publish returns the policy rate, which is where the oracle would be with no gate at all.
- **It fails towards caution.** A dead keeper means stale evidence, which means the tight rate. This matches the existing choice that a dead keeper makes quotes more cautious, never less.
- **It acts only where manipulation is cheap.** In the replay, holding a pump for 24 hours on a thin market lets an attacker borrow about $122k instead of about $350k; on a deep market the result is identical.
- **Tight rates are paid by borrowers.** A real rally on a thin market is tracked slowly, so collateral is undervalued for longer. That is the intended trade.
- **Evidence is a model, not a measurement.** The keeper estimates cost from the asset's Stellar DEX order book and assumes the worst case: that every upstream reads that market. If some upstreams read other venues, the real cost is higher and the gate is more cautious than it needs to be.
- **The keeper now writes more often.** One `publish_evidence` per asset per run, and the evidence must be fresher than `max_evidence_age`.
