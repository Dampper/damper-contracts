# 0006. Coverage-guided fuzzing of the pure price math, alongside the property tests

Status: accepted, 2026-11-25

## Context

[TESTING.md](../TESTING.md) listed `cargo-fuzz` / libFuzzer as not yet covered. The property tests (`fuzz.rs`) already sample the input space of `clamp`, `uneconomic` and `effective_policy` randomly, at up to 20,000 cases a property; that caught real gaps before (two deliberate bugs were found only by fuzzing, not the unit tests — see TESTING.md's mutation table). Random sampling still has a blind spot a coverage-guided fuzzer does not: it has no way to steer itself toward the one input that flips a branch, where libFuzzer's instrumentation does exactly that, by code path rather than by chance.

## Decision

`contracts/guarded-oracle/fuzz/` is a `cargo-fuzz` crate with three libFuzzer targets, each exercising the same functions the property tests already scrutinize most and checking the same invariants:

- **`clamp`**: never panics, never returns below 1, only ever moves a price towards the anchor, idempotent, leaves an unbounded direction untouched, and matches `reference_clamp` exactly in the range both sides compute without overflow.
- **`gate`**: `uneconomic` never more optimistic than exact arithmetic, exact within `within_exact_range`, monotone (more cost, less exposure or a lower ratio can only make an attack less economic, checked by deriving three further values from the same input rather than needing a second generator).
- **`effective_policy`**: the gate never loosens a policy, the window is untouched, `UNBOUNDED` is preserved exactly, and the result matches `reference_effective` exactly within the exact range, no looser than it outside that range.

The reference arithmetic the property tests already wrote (`mul_wide`, `exact_uneconomic`, `within_exact_range`, `cut`, `reference_effective`, `reference_band`) moved out of `fuzz.rs` (`#[cfg(test)]`, invisible outside the crate) into [`src/reference.rs`](../../contracts/guarded-oracle/src/reference.rs) (`pub mod`, an ordinary part of the crate's Rust API), so both the property tests and these fuzz targets check against the one reference rather than two copies that could silently drift apart. This has no effect on the deployed contract: Soroban's on-chain interface is only what `#[contractimpl]` exposes, not the crate's Rust-level visibility.

`arbitrary`'s derive builds structured `Policy` / `Gate` / `Evidence` combinations directly from the fuzzer's bytes, rather than three independent fields `effective_policy`'s target would have to assemble by hand, so a mutation that changes one byte tends to land on another valid combination instead of a value the type would have rejected anyway.

## What the first run found

Within seconds, not the full budget: `clamp` and `gate` each found a real input — never a mutant introduced to check the tests work, a genuine first run — that violated an invariant the property tests have never once failed on in 20,000-case runs. Both share the same shape and the same root cause: the property tests' `price()`/`anchor()`/`amount()` generators are all documented as producing realistic (positive, in-range) values, matching what every real caller passes; `arbitrary`'s derive on a plain `i128`/`u32` field has no such restriction, and decodes straight to the type's full range.

- **`clamp`**, twice: a negative anchor broke `band`'s overflow-fallback sign, landing the quote far outside `[price, anchor]`; separately, a non-positive price with both directions `UNBOUNDED` skipped the only branch that floors the result. Both fixed in `aggregate::clamp` (a non-positive anchor now returns the floor outright; the floor is now unconditional, not just the down branch's).
- **`gate`**, once: a zero `min_cost_ratio_bps` — already rejected by `set_gate`, so unreachable through it — made a free attack against real exposure read as *uneconomic* in `aggregate::uneconomic`, the opposite of the function's own meaning. The independent reference (`exact_uneconomic`) already had the missing guard; `uneconomic` now has it too.
- **`effective_policy`** ran clean through its full budget on every attempt.

None of the three is reachable from a valid contract state today — `normalize`/`median` only ever produce positive prices, and `set_gate` already validates the ratio — so nothing deployed was ever at risk. They are still real logic gaps, not fuzz-harness artifacts: each is a case where the function's behavior silently stopped matching its own doc comment (or, for `uneconomic`, its own name) once given input outside the range every existing test happened to restrict itself to. See TESTING.md for the fix details and the regression test each one got. After all three fixes, a further clean run of all three targets found nothing further: `clamp` 4,073,419 executions, `gate` 4,427,813, `effective_policy` 2,152,054, each over 300 seconds from a fresh corpus.

## Consequences

- The three fixes are small (two short-circuits, one added guard) and change behavior for no input any property test, the stateful test, or a real caller has ever produced — confirmed by rerunning the full suite after each (no new failures) and by rebuilding the wasm (26,161 bytes, 33 bytes over the pre-fix build).
- These targets reach the *pure* math only, with no `Env`: the stateful property test already covers the host-facing `#[contractimpl]` entry points (`poke`, `set_policy`, `set_gate`, `publish_evidence`, ...) under random operation sequences, but not coverage-guided, since libFuzzer driving a full Soroban test `Env` per iteration is a different scale of problem (host setup cost per run, and `arbitrary` would need to decode entire operation sequences, not single structured values). Left for later; not something this change does.
- A crash here is a counterexample the property tests' own generators happened not to produce, same severity as a property test failure, and gets the same treatment: a regression test with the input the fuzzer found, not just a silent fix — exactly what the three new `test.rs` cases are.
- The fuzz crate is its own Cargo workspace (`[workspace]` in `fuzz/Cargo.toml`), excluded from the parent's, with its own target directory. It needs nightly Rust (for the sanitizer instrumentation `cargo-fuzz` builds with) and `cargo-fuzz` itself; neither is needed for `cargo test`, `cargo build` or the deployed contract.
