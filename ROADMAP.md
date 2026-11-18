# Roadmap

Status of the contracts. The repository README carries the design; this file tracks what is done and what is next.

## Done

- **M1.** Median and quorum over fresh upstream feeds, anchor clamp with per-asset policy, keeper-only anchor updates, YieldBlox replay, testnet demo deployment.
- **Manipulation-cost gate.** The keeper publishes market evidence and the oracle tightens its clamp where an attack would pay. It can only tighten.
- **Property tests and stateful fuzzing** of the clamp, the gate and the whole contract, with a weekly deep run.

## Next

- **Fit the rates and ratios to data.** The clamp rates, tight rates and the required cost ratio are placeholders. Fit them per asset from price history and measured depth.
- **Events.** The oracle emits none yet. Anchor moves, policy changes and keeper changes should be observable without reading storage.
- **Historical lookups.** `price` and `prices` return nothing. Anything that needs time-weighted prices from Damper itself will need an on-chain history.
- **Coverage-guided fuzzing and formal verification.** The property tests sample inputs; `cargo-fuzz` targets and a verifier such as Kani would explore the arithmetic more systematically.
- **Testnet with a real Blend pool** using Damper as its oracle, then an external review.

## Later

- NAV-attested feeds for regulated real-world assets, so they can be priced without relying on thin DEX markets.
