# Contributing to Damper Contracts

Thanks for helping. This page covers setup, checks and what a pull request needs. For the shared rules (labels, commit messages, code of conduct) see the [organisation guide](https://github.com/Dampper/.github/blob/main/CONTRIBUTING.md).

## Finding something to work on

Look for issues labelled `good first issue` or `help wanted`. Each one lists what to do, acceptance criteria and where to look. Comment to get assigned before you start.

Good places to begin if no issue fits:

- a test for a case the replay does not cover (see [`replay.rs`](contracts/guarded-oracle/src/replay.rs));
- a sharper edge-case test for the clamp math in [`aggregate.rs`](contracts/guarded-oracle/src/aggregate.rs);
- a docs fix.

## Working on a Stellar Wave issue

Pick an issue marked for the Wave, comment to be assigned, and open your pull request against it. Keep to the issue's acceptance criteria; if you find the issue is wrong, say so in the issue rather than widening the change. Pull requests are reviewed against the acceptance criteria and the checks below.

## Local setup

You need Rust (stable) with the `wasm32v1-none` target and the [Stellar CLI](https://developers.stellar.org/docs/tools/cli/install-cli) v25.2 or newer. `soroban-sdk` 29 refuses to build a contract with plain `cargo build`, so the wasm is built through the CLI.

```sh
rustup target add wasm32v1-none
cargo install --locked stellar-cli
```

## Making a change

```sh
make test     # unit tests, including the YieldBlox replay
make lint     # clippy with warnings as errors
make fmt      # format
make build    # optimized wasm through stellar contract build
make check    # all of the above
```

CI runs the same commands. A pull request needs:

- **A test that fails without the change**, for any change to how a price is computed, bounded or refused. The replay in `replay.rs` is the template: it states the scenario and asserts what a pool would see.
- **A property, where one fits.** If the change adds a rule the oracle should keep for every input (a bound, an ordering, a refusal), state it as a property in [`fuzz.rs`](contracts/guarded-oracle/src/fuzz.rs) and, if it is a rule about the whole contract, as a check in the stateful test. Run `PROPTEST_CASES=5000 cargo test fuzz` before opening the pull request. If a property fails, commit the saved `proptest-regressions/` file with the fix. See [docs/TESTING.md](docs/TESTING.md).
- **A mutation check for new guards.** If you add a protection, break it on purpose, confirm the test fails, and say so in the pull request.
- **Docs.** If a design choice changed, update the matching decision record in [`docs/adr`](docs/adr/README.md) or add a new one.

## Code guidelines

- The crate is `#![no_std]`; there is no printing, no allocation-heavy code, no floating point.
- Arithmetic on prices is checked or saturating. Say which and why in a comment when it is not obvious.
- Anything a price lookup reads must have its storage expiry extended somewhere that runs regularly (see `keep_alive`).
- Keep the SEP-40 types in `types.rs` byte-compatible with the spec; pools decode them.

## Commit messages

Imperative subject under about 70 characters, then a body that says why.

## Security

Do not open public issues for vulnerabilities. See [SECURITY.md](https://github.com/Dampper/.github/blob/main/SECURITY.md).
