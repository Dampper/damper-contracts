# Testing

The oracle is tested in five layers, from specific to general. A change to how a price is computed, bounded or refused should be covered by each layer it touches.

| Layer | Where | What it answers |
| --- | --- | --- |
| Unit tests | [`test.rs`](../contracts/guarded-oracle/src/test.rs) | Does each function do what it says in the cases we thought of? |
| Replay | [`replay.rs`](../contracts/guarded-oracle/src/replay.rs) | Would the February 2026 YieldBlox exploit have worked against it? |
| Property tests | [`fuzz.rs`](../contracts/guarded-oracle/src/fuzz.rs) | Does a rule hold for *every* input a realistic generator can produce, including the extremes of every type? |
| Stateful fuzzing | [`fuzz.rs`](../contracts/guarded-oracle/src/fuzz.rs) | Does the deployed contract keep its promises under any sequence of events? |
| Coverage-guided fuzzing | [`fuzz/fuzz_targets/`](../contracts/guarded-oracle/fuzz/fuzz_targets/) | Does a rule hold over the *whole* input a type allows, including combinations a hand-written generator never considered reachable? ([ADR 0006](adr/0006-coverage-guided-fuzzing.md)) |

## Running them

```sh
cargo test                                   # everything, at a depth that keeps CI quick (about a minute)
cargo test fuzz                              # only the property and stateful tests
PROPTEST_CASES=20000 cargo test fuzz         # a deeper search of the pure properties
DAMPER_FUZZ_STATEFUL_CASES=250 cargo test fuzz  # more random operation sequences against the contract
```

When a property fails, `proptest` shrinks the input to a small one, prints it, and saves it under `contracts/guarded-oracle/proptest-regressions/`. Commit that file with the fix: from then on the case is replayed on every run. A scheduled workflow runs the deep search weekly and uploads that file if it finds something.

```sh
cd contracts/guarded-oracle
cargo install cargo-fuzz                    # needs a nightly toolchain
cargo +nightly fuzz run clamp -- -max_total_time=300
cargo +nightly fuzz run gate -- -max_total_time=300
cargo +nightly fuzz run effective_policy -- -max_total_time=300
```

When `cargo-fuzz` finds a crash it writes the input to `fuzz/artifacts/<target>/` and prints a `cargo fuzz run <target> <path>` command that replays exactly that input. Turn the replayed input into a unit test in `test.rs` (see the three regression tests below) rather than only keeping the artifact file, so the case is visible without re-running the fuzzer and survives a corpus reset.

## What the property tests check

**The clamp** (`aggregate::clamp`), over every `i128` price and anchor, every `u64` elapsed time and every `u32` rate:

- it never panics and never returns less than 1;
- it only ever moves a price *towards* the anchor, never past it and never away;
- in the ranges real prices live in, it equals an independently written formula: anchor ± anchor × rate × min(elapsed, window) / 1 hour, with the downward allowance capped at the anchor and the result floored at 1;
- a higher input price never gives a lower quote (monotone);
- clamping twice changes nothing (idempotent);
- more time never narrows the band;
- a direction the policy leaves unbounded never touches a price on that side. This is what keeps liquidations working in a crash.

**The gate** (`aggregate::uneconomic`, `effective_policy`):

- `uneconomic` is exact arithmetic (cost × 10,000 ≥ exposure × ratio) and survives amounts large enough to overflow;
- more cost, less exposure, or a lower required ratio can only make an attack less economic;
- evidence can never loosen a policy: no faster rate, no rate where the policy had none, no bound where the policy left a direction free;
- missing, stale and future-dated evidence all give the tight rate;
- **the quote with the gate is never further from the anchor than the quote without it, and never on the other side.** This is the property the gate exists for.

**The median and decimals:** the median is the middle value in any order; one wild feed out of three cannot move it (the YieldBlox shape); rescaling decimals never panics and is lossless going up.

## What the stateful test checks

It deploys the oracle with three feeds, a collateral policy and a seeded anchor, then applies random sequences of up to 30 operations: time passing, feeds reporting wild, zero, negative, stale or future-dated prices, policy and gate changes (valid and invalid), evidence, seeding and poking. After every step it checks:

- a quote is positive and never from the future;
- no quorum, no quote; a policy with no anchor, no quote; an asset with no policy is quoted raw;
- the quote lies between the raw median and the anchor, and never outside the policy's band, which is recomputed independently;
- the effective policy is never looser than the policy, and equals it when there is no gate;
- every state-changing call succeeds exactly when its inputs are valid, and a failed call changes nothing;
- `poke` is a no-op inside the minimum interval, and otherwise moves the anchor to exactly the quote a pool was reading.

## Checking the tests themselves

A passing test only means something if it can fail. When adding a protection, break it on purpose and confirm a test catches it, then say so in the pull request. The 16 deliberate bugs below were each applied to the contract in turn, and the unit and replay tests were run separately from the property and fuzz tests, to see what fuzzing adds.

| Bug introduced | Caught by |
| --- | --- |
| Band denominator ten times too small | unit + fuzz |
| Elapsed time not capped by the window | unit + fuzz |
| Downward bound applied when the policy leaves it free | unit + fuzz |
| Upward bound subtracts instead of adds | unit + fuzz |
| Tight rate allowed to loosen a policy | unit + fuzz |
| Gate bounds a direction the policy leaves free | unit + fuzz |
| Evidence from the future trusted | unit + fuzz |
| Evidence accepted one second past its age limit | unit + fuzz (needed a boundary property) |
| Cost product wraps instead of saturating | unit + fuzz (needed a wider generator) |
| Overflow counted as "uneconomic" | unit + fuzz |
| Even-length median takes the upper middle value | unit + fuzz |
| Zero-cost evidence accepted | unit + fuzz |
| Poke allowed inside the minimum interval | unit + fuzz (needed time steps biased to the boundary) |
| **Nothing to take counted as "not uneconomic"** | **fuzz only** |
| **A policy with a zero window accepted** | **fuzz only** |
| Quote allowed to reach zero | not detectable: the floor of 1 is dead code for a positive price, and the contract only passes positive prices |

Two of these were missed by the unit tests and found by the fuzz tests, and the fuzz tests themselves missed three on the first attempt, until their generators were fixed to produce boundary cases, invalid configurations and extreme magnitudes. Writing a generator that can reach the failing input matters as much as stating the property.

## What the fuzzing found

- **Overflow semantics of the gate.** The stateful test compares the live contract with an independently written reference of the gate. The first mismatch looked like a contract bug and was a bug in the reference, but it showed exactly how the contract behaves at absurd magnitudes: `cost × 10,000` saturates, which is correct (a cost that large dwarfs any exposure), while an exposure so large that `exposure × ratio` overflows (above about 10^38 in oracle units) is counted as *not* uneconomic, which errs towards the tight rate. The reference now does exact 256-bit arithmetic, and the property is: the contract is exact wherever nothing overflows, and never more optimistic than exact maths anywhere. The case is kept in `proptest-regressions/` and as the unit test `a_cost_too_large_to_multiply_is_still_uneconomic`.
- **No bug in the contract's price logic, at the domain the property tests explore.** At 20,000 cases per property and 250 random operation sequences, none of the rules failed. That domain is "any positive price, any policy, any elapsed time" — realistic by construction, since every generator was written to match what the contract's own callers actually produce.
- **Three bugs in the domain the property tests don't explore, found by `cargo-fuzz`'s first real runs** (ADR 0006). Every property test's `price()`/`anchor()` strategy is documented "any **positive** price"; `clamp`'s and `gate`'s fuzz targets decode a plain `i128`/`u32`, which includes values no property test, and no real caller, ever produces:
  - A **negative anchor** made `band`'s overflow fallback compute a value with the wrong sign, landing the clamped quote far outside `[price, anchor]` (positive, when both inputs were deeply negative). Unreachable — `store_anchor`'s only callers always pass a positive value — but `clamp` now returns the floor of 1 outright for a non-positive anchor rather than running arithmetic that assumes a sign the input does not have.
  - A **non-positive price with both directions `UNBOUNDED`** skipped the only branch that floors the result, so it passed through unfloored. Also unreachable in practice, and also fixed: the floor is now unconditional, not just the down branch's.
  - A **zero `min_cost_ratio_bps`** (already rejected by `set_gate`, so unreachable through it) made a free, non-positive-cost attack against real exposure read as *uneconomic* in `aggregate::uneconomic` — the opposite of what the function means by that word. The independent reference (`exact_uneconomic`) already had the guard the real function was missing; `uneconomic` now has it too.

  All three share a shape: coverage-guided fuzzing decodes bytes straight into a function's full parameter type, with no generator standing between it and a value no caller was ever expected to pass. That is exactly where it earns its keep over random sampling of a realistic domain — and exactly why each fix is paired with a regression test using the literal input the fuzzer found (`a_non_positive_anchor_falls_back_to_the_floor_instead_of_a_wrong_sign`, `a_non_positive_price_still_floors_at_one_when_both_directions_are_unbounded`, `a_free_attack_against_real_exposure_is_never_uneconomic`), not left as fuzzer-only coverage.

## Not covered yet

- Formal verification of the arithmetic (for example with Kani or a Soroban-specific verifier) — fuzzing samples, even coverage-guided; it does not prove.
- Coverage-guided fuzzing of the host-facing entry points (the `#[contractimpl]` methods with a real `Env`). The stateful property test already covers them, but by random sampling, not by code path.
- Anything involving Blend's real contracts. The replay uses a minimal lending pool that makes the same oracle call and collateral check.
