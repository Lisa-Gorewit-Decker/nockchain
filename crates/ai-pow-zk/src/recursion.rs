//! §recursion — integrate the ai-pow-zk composite proof with the
//! vendored `Plonky3-recursion` substrate.
//!
//! Feature-gated behind `recursion`. This module lives in `ai-pow-zk`
//! (not the recursion workspace) because it is the *caller* side of a
//! generic API: `p3_recursion`'s verifier entrypoints are generic over
//! the inner AIR, and here we instantiate them with the concrete
//! `CompositeFullAirWithLookups` + `AiPowStarkConfig`. The recursion
//! substrate stays application-agnostic.
//!
//! Staging:
//! - S2 — cross-workspace build path established.
//! - S3a (this commit) — confirm the composite AIR satisfies the
//!   recursion substrate's `RecursiveAir` bound (the trait-friction
//!   check the integration de-risk flagged as the key risk).
//! - S3b.. — build `BatchProofTargets` / `CommonDataTargets`, wire the
//!   Tip5 NPO, call `verify_batch_circuit`.

use p3_lookup::logup::LogUpGadget;
use p3_recursion::RecursiveAir;

use crate::circuit::Challenge;
use crate::{CompositeFullAirWithLookupsPinned, Val};

// Re-export the recursion verifier entrypoints so the integration
// (and tests) reach them through `ai_pow_zk::recursion::*`.
pub use p3_recursion::{verify_batch_circuit, verify_p3_uni_proof_circuit};

/// S3a — compile-time proof that the composite AIR satisfies the
/// recursion substrate's `RecursiveAir` bound.
///
/// `verify_batch_circuit` requires its inner AIR `A: RecursiveAir<
/// Val, Challenge, LG>`. `RecursiveAir` is blanket-implemented for any
/// `A: Air<InteractionSymbolicBuilder<F, EF>>` (`p3_recursion`
/// `traits/air.rs`), so this generic fn — when instantiated with
/// `CompositeFullAirWithLookupsPinned` below — forces the bound to be
/// checked by the compiler. If the composite did not conform, this
/// module would fail to compile.
fn _require_recursive_air<A>()
where
    A: RecursiveAir<Val, Challenge, LogUpGadget>,
{
}

#[allow(dead_code)]
fn _composite_conforms_to_recursive_air() {
    _require_recursive_air::<CompositeFullAirWithLookupsPinned>();
}
