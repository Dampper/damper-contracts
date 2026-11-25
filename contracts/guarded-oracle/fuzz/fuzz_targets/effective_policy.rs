//! Coverage-guided companion to `fuzz.rs`'s gate tests, for `effective_policy` end to end
//! rather than `uneconomic` alone: whole `Policy`/`Gate`/`Evidence` combinations, decoded
//! structurally by `arbitrary` so the fuzzer can mutate a valid combination into another valid
//! one instead of spending most of its budget on inputs the type system would have rejected.
#![no_main]
use arbitrary::Arbitrary;
use damper_guarded_oracle::aggregate::effective_policy;
use damper_guarded_oracle::reference::{reference_effective, within_exact_range};
use damper_guarded_oracle::{Evidence, Gate, Policy, UNBOUNDED};
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct RawPolicy {
    up_unbounded: bool,
    up_bps_per_hour: u32,
    down_unbounded: bool,
    down_bps_per_hour: u32,
    window_secs: u64,
}
impl From<RawPolicy> for Policy {
    fn from(p: RawPolicy) -> Self {
        Policy {
            up_bps_per_hour: if p.up_unbounded { UNBOUNDED } else { p.up_bps_per_hour },
            down_bps_per_hour: if p.down_unbounded { UNBOUNDED } else { p.down_bps_per_hour },
            window_secs: p.window_secs.max(1),
        }
    }
}

#[derive(Arbitrary, Debug)]
struct RawGate {
    tight_up_unbounded: bool,
    tight_up_bps_per_hour: u32,
    tight_down_unbounded: bool,
    tight_down_bps_per_hour: u32,
    min_cost_ratio_bps: u32,
    max_evidence_age: u64,
}
impl From<RawGate> for Gate {
    fn from(g: RawGate) -> Self {
        Gate {
            tight_up_bps_per_hour: if g.tight_up_unbounded { UNBOUNDED } else { g.tight_up_bps_per_hour },
            tight_down_bps_per_hour: if g.tight_down_unbounded { UNBOUNDED } else { g.tight_down_bps_per_hour },
            min_cost_ratio_bps: g.min_cost_ratio_bps.max(1),
            max_evidence_age: g.max_evidence_age.max(1),
        }
    }
}

#[derive(Arbitrary, Debug)]
struct RawEvidence {
    up_cost: i128,
    up_exposure: i128,
    down_cost: i128,
    down_exposure: i128,
    timestamp: u64,
}
impl From<RawEvidence> for Evidence {
    fn from(e: RawEvidence) -> Self {
        Evidence {
            up_cost: e.up_cost,
            up_exposure: e.up_exposure,
            down_cost: e.down_cost,
            down_exposure: e.down_exposure,
            timestamp: e.timestamp,
        }
    }
}

#[derive(Arbitrary, Debug)]
struct Input {
    policy: RawPolicy,
    gate: RawGate,
    evidence: Option<RawEvidence>,
    now: u64,
}

fuzz_target!(|i: Input| {
    let policy: Policy = i.policy.into();
    let gate: Gate = i.gate.into();
    let evidence: Option<Evidence> = i.evidence.map(Into::into);

    let got = effective_policy(&policy, &gate, evidence.as_ref(), i.now);

    // The gate only ever tightens: never a faster rate, never a bound on a direction the
    // policy left free, and the window never changes.
    assert!(got.up_bps_per_hour <= policy.up_bps_per_hour, "loosened up: {got:?} vs policy {policy:?}");
    assert!(got.down_bps_per_hour <= policy.down_bps_per_hour, "loosened down: {got:?} vs policy {policy:?}");
    assert_eq!(got.window_secs, policy.window_secs, "the gate touched window_secs");
    assert_eq!(got.up_bps_per_hour == UNBOUNDED, policy.up_bps_per_hour == UNBOUNDED, "bounded a free up direction, or freed a bounded one");
    assert_eq!(got.down_bps_per_hour == UNBOUNDED, policy.down_bps_per_hour == UNBOUNDED, "bounded a free down direction, or freed a bounded one");

    // Exact match against the independent reference wherever the evidence is small enough that
    // neither direction's cost-ratio check can overflow; past that, only "no looser" (above).
    let want = reference_effective(&policy, &gate, evidence.as_ref(), i.now);
    let exact = evidence.as_ref().is_none_or(|e| within_exact_range(e.up_cost, e.up_exposure) && within_exact_range(e.down_cost, e.down_exposure));
    if exact {
        assert_eq!(&got, &want, "disagrees with the reference in the exact range");
    } else {
        assert!(
            got.up_bps_per_hour <= want.up_bps_per_hour && got.down_bps_per_hour <= want.down_bps_per_hour,
            "contract {got:?} looser than exact arithmetic {want:?}"
        );
    }
});
