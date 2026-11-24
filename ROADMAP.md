# Roadmap

Status of the contracts. The repository README carries the design; this file tracks what is done and what is next.

## Done

- **M1.** Median and quorum over fresh upstream feeds, anchor clamp with per-asset policy, keeper-only anchor updates, YieldBlox replay, testnet demo deployment.
- **Manipulation-cost gate.** The keeper publishes market evidence and the oracle tightens its clamp where an attack would pay. It can only tighten.
- **Property tests and stateful fuzzing** of the clamp, the gate and the whole contract, with a weekly deep run.
- **Fit the rates and ratios to data.** The testnet demo's clamp rate, tight rate and cost ratio were placeholders; they are now fitted to 180 days of real XLM/USDC trade history and the backend scanner's own risk tiers ([ADR 0005](docs/adr/0005-rates-fitted-to-real-data.md)). This is one asset's numbers from one market, not a contract default — a different asset needs its own pull of the same analysis.

## Next

- **Historical lookups.** `price` and `prices` return nothing. Anything that needs time-weighted prices from Damper itself will need an on-chain history.
- **Coverage-guided fuzzing and formal verification.** The property tests sample inputs; `cargo-fuzz` targets and a verifier such as Kani would explore the arithmetic more systematically.

## Later

- **A real Blend pool on testnet using Damper as its oracle.** Blocked, not abandoned: testnet BLND has no faucet and almost no DEX depth, and the Backstop Bootstrapper contract only splits a deposit between two parties, it does not supply the tokens. Revisit if Blend's testnet BLND/USDC becomes available another way (their Discord, most likely), or if the project later wants a self-deployed mock Comet/backstop stack purely to prove the integration — that would be a sandbox we control, not Blend's real shared testnet, so it stands on its own rather than as evidence. The project's actual evidence does not depend on this: the scanner already analyzes real, live mainnet Blend pools, and the YieldBlox replay is real history.
- NAV-attested feeds for regulated real-world assets, so they can be priced without relying on thin DEX markets.
