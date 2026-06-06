# AI-PoW Proof Sizes, Soundness, And Assumptions

Date: 2026-06-05
Status: current measurement and cryptographic-assumption checkpoint.

## Scope

This note answers four concrete questions for the current AI-PoW proving stack:

1. How large is the regular proof of the AI-PoW puzzle?
2. How large is the recursive proof of that proof?
3. How sound is each layer?
4. Which cryptographic assumptions are taken at each step, and are they cited?

Here "regular proof" means the Layer-0 `CompositeFullAirWithLookupsPinned`
batch-STARK produced by `composite_prove_pinned_logup`. It does not mean the
legacy/plain `MatmulProof`, which is a miner diagnostic and pre-ZKP target-hit
object, not the production block artifact.

The production recursive-certificate target is the native terminal backend
from `2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md`. The older L1
batch-STARK recursive certificate remains an important hardened verifier path
and regression target, but it is too large for the production wire budget and
must not be treated as the production block/wire artifact.

## Current Sizes

| Artifact | Current production role | Last measured size | Source |
|---|---|---:|---|
| Layer-0 composite proof | Regular STARK proof of the AI-PoW puzzle statement; consumed by recursion; diagnostic/intermediate, not persisted by consensus | `304,048` bytes / `296.9 KiB` | `RUSTFLAGS="-C target-cpu=native" cargo run -p ai-pow-zk --release --features recursion --example prod_recursion_measure -- 15`, 2026-06-05 after `CircuitConfig::PROD.pow_bits=0` |
| Hardened L1 batch-STARK recursive certificate | Soundness-hardened recursive verifier checkpoint/fallback path; not acceptable as the production wire artifact because it exceeds the size budget | L1 proof body `149.1 KiB`; full checkpoint certificate `1,135.5 KiB` legacy postcard / `5,794.7 KiB` fixed-int bincode / `358.3 KiB` gzip-best envelope | same `prod_recursion_measure 15` run |
| Relaxed L1-only batch-STARK pure-query sweep | Diagnostic for a statement-bound L1-only object that does not count proof-system PoW; not production-qualified because every measured pure-query profile misses size and time | `lb=4,nq=15,pow=0`: `226,542` bytes / `221.2 KiB`, prove `49.290s`; `lb=5,nq=12,pow=0`: `196,488` bytes / `191.9 KiB`, prove `98.009s`; `lb=6,nq=10,pow=0`: `176,362` bytes / `172.2 KiB`, prove `195.574s` | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion relaxed_l1_only_pure_query_statement_bound_candidate_size_breakdown_for_test_pearl -- --ignored --nocapture`, 2026-06-05 |
| Relaxed L1-only batch-STARK cap-height sweep | Diagnostic for the smallest measured pure-query shape, varying only MMCS cap height; not enough to hit the relaxed gate | `lb=6,nq=10,pow=0,cap=4`: `173,171` bytes / `169.1 KiB`, prove `191.448s`; `cap=6`: `187,961` bytes / `183.6 KiB`, prove `193.219s`; cap-4 opening proof alone `141,987` bytes | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion relaxed_l1_only_pure_query_lb6_cap_height_candidate_size_breakdown_for_test_pearl -- --ignored --nocapture`, 2026-06-05 |
| Relaxed L1-only batch-STARK opening-proof breakdown | Diagnostic for the best pure-query/cap-height point; identifies what must be structurally reduced | `lb=6,nq=10,pow=0,cap=4`: FRI query proofs `136,577` bytes, input leaf opened values `63,201` bytes, input Merkle paths `34,213` bytes, commit-phase sibling values `4,813` bytes, commit-phase Merkle paths `34,259` bytes | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion relaxed_l1_only_pure_query_lb6_cap4_opening_breakdown_for_test_pearl -- --ignored --nocapture`, 2026-06-05 |
| Relaxed L1-only batch-STARK FRI-shape sweep | Diagnostic for soundness-neutral final-polynomial tail/folding shape on the best pure-query/cap-height point; not enough to hit size/time | `lfp=0,mla=3`: `175,304` bytes, prove `195.531s`; `lfp=1,mla=3`: `173,481` bytes, prove `196.417s`; current `lfp=2,mla=3` remains smallest measured at `173,171` bytes | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion relaxed_l1_only_pure_query_lb6_cap4_fri_shape_sweep_for_test_pearl -- --ignored --nocapture`, 2026-06-05 |
| Pure-query L2-over-L1 batch-STARK sweep | Diagnostic for a Pearl-shaped second recursive compression layer over the statement-bound L1 proof; soundly binds the L1 statement digest and verifies the metadata-free path-pruned compact body, but still misses end-to-end time and the hard `~100 KiB` proof target | Shared L1 `lb=6,nq=10,cap=4`: `173,868` bytes, L1 prove `192.807s`; compact-body L2 `lb=4,nq=15,cap=4`: `159,945` bytes, L2 prove `12.571s`; compact-body L2 `lb=5,nq=12,cap=4`: `137,816` bytes, L2 prove `24.318s`; compact-body L2 `lb=6,nq=10,cap=4`: `126,251` bytes, L2 prove `48.074s` | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl -- --ignored --nocapture`, 2026-06-06 |
| Pure-query L2 compact-body cap-height sweep | Diagnostic for the remaining path restoration payload in the relaxed-final-layer row; verifies all compact bodies, but cap height alone cannot reach hard size | L2 `lb=5,nq=12` over shared slow L1: `cap=2` body `140,056` bytes / restoration `57,510` bytes / prove `25.422s`; `cap=4` body `137,816` bytes / restoration `50,609` bytes / prove `24.403s`; `cap=6` body `146,931` bytes / restoration `41,456` bytes / prove `24.320s`; `cap=8` body `205,134` bytes / restoration `32,936` bytes / prove `24.174s` | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion pure_query_l2_over_l1_l2_cap_height_compact_body_breakdown_for_test_pearl -- --ignored --nocapture`, 2026-06-06 |
| Pure-query L2 compact-body FRI-shape sweep | Diagnostic for soundness-neutral final-polynomial/folding shape in the relaxed-final-layer row; verifies all compact bodies, but shape tuning only saves tens of bytes | L2 `lb=5,nq=12,cap=4`: `lfp=0,mla=3` body `138,170` bytes; `lfp=1,mla=3` body `141,282` bytes; `lfp=2,mla=2` body `146,316` bytes; current `lfp=2,mla=3` body `137,816` bytes; `lfp=2,mla=4` body `137,766` bytes / prove `24.345s` | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion pure_query_l2_over_l1_l2_fri_shape_compact_body_breakdown_for_test_pearl -- --ignored --nocapture`, 2026-06-06 |
| Pure-query L2 actual-vs-frontier Merkle path diagnostic | Diagnostic for whether a true Merkle multiproof frontier can remove the remaining compact-body restoration payload; verifies the compact body and counts actual query-set siblings | L2 `lb=5,nq=12,cap=4` body `137,816` bytes / restoration `50,609` bytes; current prefix-pruned siblings `1,056`; ideal frontier siblings `1,024`; digest-only maximum savings `1,280` bytes before any frontier-position metadata or verifier changes; L2 prove `24.608s` | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion pure_query_l2_over_l1_l2_multiproof_frontier_estimate_for_test_pearl -- --ignored --nocapture`, 2026-06-06 |
| Pure-query L2 Pearl-rate high-blowup diagnostic | Diagnostic for a no-PoW analogue of Pearl's final `rate_bits=7` shape; verifies the compact body, but the smaller proof is still above hard size and far above time | L2 `lb=7,nq=9,cap=4` carries `63` pure-query Johnson bits; body `120,722` bytes / core `72,415` bytes / restoration `48,307` bytes; L2 prove `97.358s`; shared slow L1 prove `196.064s` | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion pure_query_l2_over_l1_l2_pearl_rate7_final_shape_for_test_pearl -- --ignored --nocapture`, 2026-06-06 |
| Fast-L1 pure-query L2-over-L1 batch-STARK sweep | Diagnostic for pairing the only sub-30s L1 profile with the actual compact final-layer body; verifies after the Tip5 MMCS direction-binding fix, but still misses size/time jointly | Shared fast L1 `lb=3,nq=20,cap=4`: `279,719` bytes, L1 prove `25.178s`; compact-body L2 `lb=4,nq=15,cap=4`: `174,707` bytes, L2 prove `25.245s`; compact-body L2 `lb=5,nq=12,cap=4`: `145,695` bytes, L2 prove `48.530s`; compact-body L2 `lb=6,nq=10,cap=4`: `129,804` bytes, L2 prove `98.584s` | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion pure_query_l2_over_fast_l1_statement_bound_candidate_size_breakdown_for_test_pearl -- --ignored --nocapture`, 2026-06-06 |
| Native terminal certificate fixture | Recursion-crate terminal proof over the real Tip5 verifier-circuit fixture; proves the terminal backend can be small, but is not yet the full `ai-pow-zk` composite verifier path | `85,948` bytes / `83.9 KiB`; release prove `1.492s`, verify `1.181s` | `RUSTFLAGS="-C target-cpu=native" cargo test --manifest-path crates/plonky3-recursion/recursion/Cargo.toml --release --test test_l1_outer_cert_tip5_unified terminal_production_certificate_measures_real_tip5_l0_verifier_circuit -- --nocapture`, 2026-06-05 |
| Full `ai-pow-zk` composite-verifier native terminal path | Opt-in diagnostic path for the actual composite L1 verifier circuit; not yet production-qualified because it misses both size and time gates | `lb=6,nq=10,pow=0` reduced-profile run after compact known-index proof encoding: terminal certificate `766,069` bytes / `748.1 KiB`; terminal public inputs `5,180` bytes; postcard wire certificate `771,249` bytes / `753.2 KiB`; release prove `80.829s`, verify `58.825s` | `NOCK_TERMINAL_PROFILE_PROVER=1 RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion terminal_recursive_certificate_for_pure_query_lb6_nq10_measures -- --ignored --nocapture`, 2026-06-05 |
| Full `ai-pow-zk` composite-verifier integrated-LogUp polynomial NPO candidate | Diagnostic only; attempts to replace exhaustive NPO openings with the integrated polynomial NPO backend while keeping the native terminal recursive-certificate shape | No completed size measurement. First release/native command compiled in `1m57s`, then the test binary ran for more than `7m35s` without reaching the final size/timing print and was stopped. A phase-instrumented rerun compiled in `1m42s` and showed `38.235s` primitive prove plus `51.902s` merged value-bridge prove before the integrated Tip5 LogUp subproof finished | `NOCK_TERMINAL_PROFILE_PROVER=1 RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion terminal_integrated_logup_candidate_for_pure_query_lb6_nq10_measures -- --ignored --nocapture`, 2026-06-05 |
| Full `ai-pow-zk` composite-verifier terminal relation metrics | Non-proving diagnostic for the same path | PROD baseline: `125,961` ops, `221,989` witnesses, `43,443` terminal private inputs, `14,049` NPO rows, `242,798` NPO residual components, `5,319` bytes of terminal public inputs, terminal compile `20.943s` | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion terminal_relation_metrics_for_prod_baseline_composite_are_available -- --ignored --nocapture`, 2026-06-05 |
| Full `ai-pow-zk` composite-verifier FRI-native NPO residual-zero floor | Diagnostic only; measures the actual composite L1 verifier NPO polynomial layout and the FRI-native residual-zero body. After the terminal Tip5 Merkle-direction mapping fix, this residual-zero layer verifies, but it is not by itself a production-sound recursive proof because the remaining NPO quotient/value-bridge/lookup and primitive-row relations still have to be included or replaced by documented bindings. | Layout: `14,049` NPO rows, `16,384` padded rows, `89` prover-dependent field columns / `178` basis columns, residual-zero opened-column floor `180` basis columns. Proof body `55,344` bytes with `54,023` bytes compact FRI; `0` nonzero residual values; status `verified`; prove `19.635s`, verify `11.930s`, total diagnostic wall `87.229s` | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion terminal_fri_native_residual_zero_candidate_for_prod_baseline_measures -- --ignored --nocapture`, 2026-06-06 |
| Full `ai-pow-zk` composite-verifier FRI-native merged value-bridge candidate | Diagnostic only; serializes the terminal prelude, primitive R1CS row-product proof, and one merged residual-zero/recompose/value-bridge NPO proof over the actual composite L1 verifier relation. It verifies after the direction-aware value-bridge fix, but is not production-qualified because the Tip5 lookup/AIR/LogUp binding and explicit `mmcs_bit` padding/boolean/present constraints still have to be in the final theorem. | Body `150,006` bytes / `146.5 KiB`: prelude `240`, primitive R1CS `57,501`, merged value bridge `92,265`, merged compact FRI `90,109`. After reusing the primitive sparse relation/assignment vector, preparing merged NPO data once for the prelude, switching proof calls to prelude-checked prover entry points, and replacing the value-bridge quotient's per-point interpolation with a batched coset LDE: post-prelude proof body construction is `16.159s` serial (`9.232s` primitive, `6.927s` merged value bridge), verify `47.907s`, total diagnostic wall `118.017s`, status `verified`. The full diagnostic setup still includes L0 prove `8.356s`, terminal compile `11.034s`, assignment commitment `7.522s`, prepared merged NPO root `15.682s`, and prelude construction `11.023s`. Primitive split remains assignment evaluation `55,025` bytes, including `53,586` bytes of fold-round openings and `48,462` bytes / `1,211` nodes of Merkle frontier. | `NOCK_TERMINAL_PROFILE_PROVER=1 RUST_LOG=p3_recursion::terminal=info RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion terminal_merged_value_bridge_candidate_for_prod_baseline_measures -- --ignored --nocapture`, 2026-06-06 |
| Full `ai-pow-zk` composite-verifier terminal assignment compact-FRI floor | Unsound diagnostic lower bound only; commits the terminal assignment vector as one FRI column and verifies a compact FRI opening. This does not replace the production assignment-evaluation proof because it does not prove the univariate FRI opening equals the multilinear assignment evaluation at the sparse-R1CS matrix-sumcheck point. | Proof `56,307` bytes: compact FRI `56,202`, opened values `41`, rows `222,449`, padded rows `262,144`, basis columns `2`, assignment length `222,449`; L1 trace verify `83ms`, terminal compile `11.080s`, assignment commit `7.542s`, prelude `10.917s`, FRI-floor prove `18.233s`, FRI-floor verify `9ms`, total wall `56.526s`. It is slightly larger than the current `55,025` byte Merkle-fold assignment evaluation proof, though faster to prove in isolation. | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion terminal_assignment_compact_fri_floor_for_prod_baseline_measures -- --ignored --nocapture`, 2026-06-06 |
| Recompose/coeff terminal relation lower bound | Unsound diagnostic only; disables the D=2 coefficient-control binding to quantify whether replacing that table could be a primary size lever | Production binding: `125,961` ops, `106,349` primitive ops, `14,049` NPO rows, `5,743` recompose/coeff rows. Disabled binding: `125,571` ops, same `106,349` primitive ops, `13,659` NPO rows, `0` recompose/coeff rows but `5,578` ordinary recompose rows. Net unsound saving: only `390` ops/NPO rows and no primitive arithmetic | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release --features recursion terminal_relation_metrics_recompose_ctl_lower_bound_for_prod_baseline_composite -- --ignored --nocapture`, 2026-06-06 |
| Layer-0 pinned+LogUp baseline proof breakdown | Diagnostic for the specialized AI-PoW AIR that a Pearl-shaped recursive compressor would consume; not a production wire artifact | `lb=4,nq=15,pow=0`: bincode proof `260,987` bytes / `254.9 KiB`, opening proof `229,849` bytes, prove `8.695s`; `lb=6,nq=10,pow=0`: bincode proof `199,882` bytes / `195.2 KiB`, opening proof `168,744` bytes, prove `32.314s` | `RUSTFLAGS="-C target-cpu=native" cargo test -p ai-pow-zk --release composite_pinned_logup_prod_l0_size_breakdown -- --ignored --nocapture` and `... composite_pinned_logup_lb6_nq10_l0_size_breakdown ...`, 2026-06-05 |

The active production target is therefore:

- regular Layer-0 proof: **296.9 KiB** if materialized;
- hardened batch-STARK L1 checkpoint: the submitted L1 proof body is
  **149.1 KiB**, and the full checkpoint certificate is **1.1 MiB+** before
  optional compression; soundness-relevant but too large for production wire
  use;
- native terminal recursive fixture: **85,948 bytes / 83.9 KiB**, satisfying
  the about-100 KiB and `<30s` release-proving gates for the recursion-crate
  Tip5 verifier fixture;
- full `ai-pow-zk` composite-verifier terminal integration: now has an opt-in
  API and measurement test, and decoded verification passes after canonical
  verifier-key rebuild. The best measured reduced-profile wire object is still
  **771,249 bytes / 753.2 KiB**, with release prove **80.829s** and verify
  **58.825s**, so this path misses both the about-100 KiB and `<30s` gates.
- full `ai-pow-zk` composite-verifier integrated-LogUp polynomial NPO
  candidate: now has an opt-in measurement test, but the release/native runs
  show it is far outside the proving-time gate. The first run did not complete
  after more than **7m35s** in the test binary. The phase-instrumented rerun
  showed **38.235s** primitive prove plus **51.902s** merged value-bridge prove
  before the integrated Tip5 LogUp subproof finished, so the small synthetic
  **94.0 KiB / 23.070s** checkpoint cannot be promoted as the production
  recursive path.
- full `ai-pow-zk` composite-verifier FRI-native NPO residual-zero floor: the
  actual composite relation has a plausible byte floor, with a **55,344 byte**
  residual-zero proof body and **54,023 byte** compact FRI payload over
  **89** prover-dependent field columns and **16,384** padded NPO rows. After
  fixing terminal Tip5 Merkle-direction input/hidden-lane reconstruction, the
  generated residual table has **0** nonzero values and the residual-zero FRI
  layer verifies. This is still not a valid production replacement by itself:
  residual-zero only proves committed residual columns are zero. The remaining
  production work is to include or replace the NPO row-relation quotients,
  value bridge, Tip5 lookup/AIR binding, primitive row-product checks, and
  their explicit transcript/root bindings without blowing the size or time
  gates. The value bridge must also bind Tip5 Merkle direction explicitly:
  callsite input slots are pre-swap bus limbs, but the Tip5 trace/AIR consumes
  post-direction permutation-input lanes, so `mmcs_bit=1` swaps which trace
  lanes are witness inputs versus hidden values. A production bridge must prove
  that bus-to-trace projection or commit/constrain direction-dependent
  trace-lane selectors.
- full `ai-pow-zk` composite-verifier merged value-bridge candidate: the next
  checkpoint now verifies over the actual composite relation and serializes to
  **150,006 bytes / 146.5 KiB** for `(prelude, primitive R1CS row-product,
  merged residual-zero/recompose/value-bridge)`. The merged NPO proof alone is
  **92,265 bytes** with a **90,109 byte** compact FRI payload. This is close to
  the relaxed `150 KiB` size gate, and after prover-work reuse the
  post-prelude proof body builds in **16.159s** serial
  (**9.232s** primitive, **6.927s** merged value bridge). It is still not the
  production theorem: it binds the `mmcs_bit`-selected
  bus-to-trace value projection, but the final stack must also include the
  Tip5 lookup/AIR/LogUp relation and the separate `mmcs_bit`
  zero-when-absent, booleanity, and value-padding constraints. Full diagnostic
  wall time remains **118.017s** because setup and verification are outside
  the post-prelude proof-body timer.
- specialized Layer-0 pinned+LogUp proof breakdown: the existing AI-PoW AIR
  proof is still too large to submit directly. The pure-query production
  baseline measured **260,987 bytes / 254.9 KiB** as bincode, dominated by a
  **229,849 byte** FRI opening proof. The reduced-query
  `lb=6,nq=10,pow=0` diagnostic lowers this to **199,882 bytes / 195.2 KiB**
  but raises baseline prove time from **8.695s** to **32.314s**. This supports
  the Pearl-shaped conclusion: the final artifact needs recursive compression
  of the base proof, not re-serialization of the base proof.

The Pearl comparison now points to a concrete non-Plonky2 route rather than a
parameter retune of the current terminalized verifier. Pearl's small final
certificate comes from proving a specialized work AIR, recursively compressing
that proof twice, compactly serializing only the final proof, and explicitly
binding verifier data such as the previous circuit digest and constants cap.
The Plonky3-native production candidate should therefore be an L0 pinned+LogUp
or narrower AI-PoW AIR proof, followed by an L1 verifier proof and an L2 compact
compression proof. The compact format may omit only verifier-deterministic data
that is reconstructed from pinned code/config and must have negative tests for
stale verifier polynomials, swapped caps/roots, wrong circuit digests, wrong
preprocessed commitments, wrong public inputs, and malformed compact openings.
The current `lb=6,nq=10,pow=0` result is a lower-bound diagnostic for the
generic terminal relation, not a production profile recommendation; `lb=4,nq=15`
remains the current pure-query baseline until the proof shape changes.

If the production size budget relaxes to about `150 KiB`, the existing
batch-STARK L1 proof body becomes a plausible size target because the
production-faithful run measured it at **149.1 KiB**. That does not make the
current `AiPowRecursiveCertificate` production-ready: the full checkpoint still
carries the L0 proof and program for verifier-side circuit reconstruction, and
therefore measures **1,135.5 KiB** under legacy postcard and **358.3 KiB** with
gzip-best compression. A `150 KiB` branch would need a new L1-only certificate
contract that pins the verifier key, L0 proof shape, statement digest,
preprocessed commitment, L1 circuit fingerprint, table metadata, and proof
public values without carrying the raw L0 proof. It also still has to solve the
time gate: the measured L1 outer batch-STARK step was **59.21s** after L0 proof
generation and **93.88s** end to end.

A new opt-in diagnostic,
`relaxed_l1_only_candidate_size_breakdown_for_test_pearl`, measures the same
split on the small `TEST_PEARL` profile: current full checkpoint postcard
**588,162 bytes**, embedded L0 proof **262,404 bytes**, embedded L0 program
**171,908 bytes**, full L1 outer object **153,850 bytes**, L1 proof body
**152,205 bytes**, L1 metadata **1,645 bytes**, and
`l1_public_binding_lanes=0`. The size result supports the relaxed target, but
the zero public-binding lanes and the **75.17s** release test runtime are the
next two blockers.

The follow-up opt-in diagnostic,
`relaxed_l1_only_statement_bound_candidate_size_breakdown_for_test_pearl`,
enables five public-binding lanes for the statement digest and verifies the
outer proof with the explicit public values. The latest validated run measured
the statement-bound L1 object at **153,904 bytes**, with a **152,259 byte**
proof body, **1,645 bytes** of metadata, `l1_public_binding_lanes=5`,
**54.62s** prove time, and **18ms** verify time inside the release test. An
earlier run of the same diagnostic measured **153,888 bytes** and **70.14s**,
so this path has small run-to-run proof-size variation and larger runtime
variation. Binding the five-limb digest therefore adds only about **54 bytes**
over the unbound diagnostic, so statement binding is not the relaxed-size
blocker. The measurement still does not production-qualify the path: it does
not remove the need for pinned verifier-key/L0-shape metadata, and the current
recursive prover profile still includes proof-system PoW in addition to query
soundness.

The no-PoW follow-up diagnostic,
`relaxed_l1_only_pure_query_statement_bound_candidate_size_breakdown_for_test_pearl`,
reuses the same statement-bound L1-only shape but proves it with
`commit_pow_bits=0` and `query_pow_bits=0`. The sweep measured the pure-query
60-bit profiles `lb=4,nq=15`, `lb=5,nq=12`, and `lb=6,nq=10`. The resulting
outer proof sizes were **226,542 bytes**, **196,488 bytes**, and **176,362
bytes**, with prove times **49.290s**, **98.009s**, and **195.574s**. This
closes the parameter-only question for the relaxed L1-only batch-STARK route:
once proof-system PoW is removed from the soundness accounting, none of the
measured 60-bit pure-query profiles meets the `150 KiB` relaxed size gate or
the `30s` proving gate. A production-qualified route therefore needs structural
compression or a different proof shape, not just a pure-query retune of the
current L1 batch-STARK envelope.

The cap-height follow-up,
`relaxed_l1_only_pure_query_lb6_cap_height_candidate_size_breakdown_for_test_pearl`,
varies only the MMCS cap for the smallest measured pure-query shape,
`lb=6,nq=10,pow=0`. Lowering the cap from `5` to `4` improves the outer object
from **176,362 bytes** to **173,171 bytes** and prove time from **195.574s** to
**191.448s**; raising it to `6` worsens the object to **187,961 bytes**. The
cap-4 component split is **2,278 bytes** commitments, **24,535 bytes** opened
values, **141,987 bytes** opening proof, and **3,473 bytes** global lookup data.
The opening proof is therefore the dominant section, and cap tuning is only a
few-KiB lever. It cannot make the L1-only batch-STARK envelope meet either the
`150 KiB` relaxed size gate or the `30s` proving gate.

The deeper opening-proof diagnostic,
`relaxed_l1_only_pure_query_lb6_cap4_opening_breakdown_for_test_pearl`,
keeps the best cap-4 point and splits the **141,987 byte** FRI opening proof.
The query proofs are **136,577 bytes**. Inside those query proofs, input
openings contribute **97,424 bytes**: **63,201 bytes** of opened leaf values and
**34,213 bytes** of input Merkle paths. Commit-phase openings contribute
**39,152 bytes**: **4,813 bytes** of sibling values and **34,259 bytes** of
Merkle paths. The conclusion is narrower than "Merkle paths are large": even
perfectly compressing Merkle paths would not get the current L1-only
batch-STARK envelope below the `100 KiB` production target, and would barely
approach the relaxed `150 KiB` target before accounting for any sound compacting
metadata. The next viable work must reduce the number/width of opened leaf
values, introduce a sound compact opening format, or add another recursive
compression layer.

The FRI-shape follow-up,
`relaxed_l1_only_pure_query_lb6_cap4_fri_shape_sweep_for_test_pearl`, keeps
`lb=6,nq=10,pow=0,cap=4,max_log_arity=3` and varies only
`log_final_poly_len`. Lowering the final-polynomial tail to `0` gives
**175,304 bytes** and **195.531s** prove time. Setting it to `1` gives
**173,481 bytes** and **196.417s**. The previously measured `lfp=2` point
remains the smallest measured pure-query cap-4 object at **173,171 bytes**.
This closes another soundness-neutral retune: final-polynomial/fold-shape
tweaks do not materially change the conclusion, and the L1-only batch-STARK
route still needs structural proof compression.

The second-recursion follow-up,
`pure_query_l2_over_l1_statement_bound_candidate_size_breakdown_for_test_pearl`,
wraps the best pure-query L1 proof in another pure-query batch-STARK. This
diagnostic also found and fixed a recursive-verifier soundness gap:
`verify_p3_batch_proof_circuit` reconstructed the Public AIR from
`proof.public_binding_lanes` but previously allocated zero public-input targets
for those lanes and did not set the recursive `PublicAir` binding lanes. The
verifier now allocates `proof.public_binding_lanes * TRACE_D` Public AIR inputs
and wires them into the in-circuit Public AIR constraints. Without that fix, an
L2 wrapper over a statement-bound L1 proof would not have been an explicit
cryptographic binding of the statement digest.

After the compact-body follow-up, the batch-STARK L2 sweep still misses the
end-to-end production target. With the shared L1 proof at
`lb=6,nq=10,pow=0,cap=4`, the L1 object is **173,868 bytes** and the required
L1 witness proof takes **192.807s**. The metadata-free
`GoldilocksTip5PathPrunedCompactBatchStarkProofBody` final-layer artifacts
measure **159,945 bytes / 12.571s** at `lb=4,nq=15`,
**137,816 bytes / 24.318s** at `lb=5,nq=12`, and
**126,251 bytes / 48.074s** at `lb=6,nq=10`; compact construction is about
**3-4ms** and compact-body verification is **34-41ms**. This proves the compact
preprocessed/path-pruned final-layer adapter and canonical-metadata verifier
are real, not just projections, and that `lb=5,nq=12` meets the relaxed
`150 KiB`/`30s` final-layer gate. However, dropping `BatchStarkProof` metadata
saves only about **0.9 KiB** over the wrapper; the larger **49-58 KiB** gap
between the core compact `BatchProof` and the wire body is the FRI shape plus
pruned-path restoration payload. Every actual compact body is still above the
hard `~100 KiB` target, and the pipeline remains dominated by the L1 witness
proof. A Pearl-shaped route still needs a more compact terminal/compression
proof, smaller restoration payload, or a way to prove the recursive verifier
relation without first materializing the expensive L1 batch-STARK witness
proof.

The L2 cap-height sweep explains why the cap-4 final-layer setting remains the
best measured point for `lb=5,nq=12`. `cap=2` lowers the core compact
`BatchProof` to **82,546 bytes** but grows the restoration payload to
**57,510 bytes**, for a **140,056 byte** body. `cap=6` and `cap=8` shrink the
restoration payload to **41,456 bytes** and **32,936 bytes**, respectively, but
the core compact proof grows to **105,475 bytes** and **172,198 bytes**. The
best measured body remains `cap=4` at **137,816 bytes**; cap tuning alone does
not bridge the hard `~100 KiB` gap.

The L2 FRI-shape sweep likewise rules out soundness-neutral final-polynomial
and folding-shape tuning as the missing lever. The current `lfp=2,mla=3` body
is **137,816 bytes**. The best measured alternate, `lfp=2,mla=4`, is only
**50 bytes** smaller at **137,766 bytes** with **24.345s** L2 prove time.
`lfp=0`, `lfp=1`, and `mla=2` are all larger. The hard-size gap is therefore
not in these FRI knobs.

The actual-vs-frontier Merkle diagnostic then measured whether the current
predecessor-suffix path pruning is leaving a large true-multiproof win behind.
On the same `lb=5,nq=12,lfp=2,mla=3,cap=4` row, the compact body stores
**1,056** digest siblings across **8** path dictionaries. The ideal frontier
for the actual opened leaf set contains **1,024** digest siblings. A direct
frontier multiproof would therefore save at most **32** digest nodes, or
**1,280 bytes** before adding frontier-position metadata and before replacing
the current upstream-verifier restoration path. This rules out Merkle frontier
encoding as the missing hard-size lever for the current batch-STARK L2 route.

The Pearl-rate final-shape diagnostic then tested the remaining obvious
parameter clue from Pearl: a high-blowup final proof. Pearl's checked-in
fixture is **59,724 bytes** total, with **59,538 bytes** of compact final
Plonky2 proof, and its proof preamble records `pow_bits=[18,18,22]` and
`rate_bits=[2,3,7]`. A no-PoW Plonky3 analogue cannot use those PoW bits, so
the measured final-layer row used `lb=7,nq=9` for **63** pure-query Johnson
bits. It reduced the compact body to **120,722 bytes**, with a **72,415 byte**
core `BatchProof`, but L2 proving rose to **97.358s**. This confirms that
Pearl's final `rate_bits=7` clue is not enough in the current Plonky3
batch-STARK path: the row still misses the hard `~100 KiB` proof target and is
over three times the final-layer proving-time gate, before the **196.064s** L1
witness proof is included.

The fast-L1 rerun pairs L1 `lb=3,nq=20,pow=0,cap=4` with the same actual compact
L2 adapter. It verifies after the Tip5 MMCS direction-binding fix and reduces
the L1 proving phase to **25.178s**, but the L1 proof is larger
(**279,719 bytes**) and makes the L2 verifier relation larger. The actual
metadata-free compact final-layer bodies are **174,707 bytes / 25.245s** at
`lb=4,nq=15`, **145,695 bytes / 48.530s** at `lb=5,nq=12`, and
**129,804 bytes / 98.584s** at `lb=6,nq=10`. This route therefore also misses:
the only sub-30s L2 row is too large, and the first row under the relaxed
`150 KiB` size gate takes about **74s** end-to-end when the L1 witness proof is
included.

Verifier status after the 2026-06-05 hardening pass: the batch-STARK
`AiPowRecursiveCertificate` verifier now calls
`BatchStarkProver::verify_all_tables` for the submitted L1 outer proof. That
fix is still required for cryptographic hygiene of the batch-STARK path: the
hardened verifier accepts honest generated L1 outer proofs and rejects outer
proof-body tampering, non-production envelopes, metadata tampering, and wrong
statement public inputs in the Rust test suite. This hardening does not make
the batch-STARK L1 certificate the production wire artifact. The Hoon/kernel
path remains fail-closed for `%ai-pow` until verifier wiring is explicitly
added.

## Recursive Pipeline Bottleneck Audit

There are currently two recursive proof pipelines in the tree, and they must
not be conflated:

1. `crates/ai-pow-zk/src/recursion.rs::AiPowRecursiveCertificate` is the
   hardened batch-STARK checkpoint certificate. It is the object still encoded
   by `crates/ai-pow-miner/src/certificate_noun.rs`.
2. `crates/plonky3-recursion/recursion/src/terminal.rs::TerminalCertificate`
   is the native terminal certificate target that currently meets the size and
   release-time gates for the recursion-crate Tip5 verifier fixture.
3. `crates/ai-pow-zk/src/recursion.rs::AiPowTerminalRecursiveCertificate`
   now wires that terminal backend to the actual composite L1 verifier circuit
   as an opt-in diagnostic path. It verifies after postcard decode and
   canonical verifier-key rebuild, but it has not met the production size or
   time gate.

The batch-STARK checkpoint pipeline is large because it proves the verifier
circuit execution as another full STARK:

```text
Layer-0 composite trace
  -> composite_prove_pinned_logup
  -> build_composite_l1_verifier_circuit
  -> run_composite_l1_verifier
  -> prove_all_tables over the verifier-circuit traces
  -> AiPowRecursiveCertificate { l0_proof, l0_program, l1_outer_proof }
```

The production-faithful `prod_recursion_measure 15` run measured the following
for this batch-STARK checkpoint path:

| Stage | Time | Why it costs |
|---|---:|---|
| L0 composite prove | `34.11s` | Proves a `2^15` row, `1917` column composite AIR with `log_blowup=4`; the main-trace LDE alone is about `7.5 GiB` before permutation, quotient, and Merkle-tree allocations |
| L1 verifier-circuit build | `0.51s` | Builds the recursive verifier circuit and allocates targets; not the bottleneck |
| L1 in-circuit verify | `0.06s` | Runs the verifier circuit once to produce traces; not the bottleneck |
| L1 outer certificate prove + verify | `59.21s` | Proves the verifier-circuit traces with `BatchStarkProver::prove_all_tables` using the production Tip5/recompose NPO tables and FRI profile |
| End-to-end trace-to-recursive-proof time | `93.88s` | Mostly L0 proving plus the L1 batch-STARK outer proof |
| Recursive-only time after L0 proof exists | `59.77s` | Mostly the L1 outer batch-STARK proof |

The size bottleneck in that same batch-STARK checkpoint is not top-level noun
or serde framing. It is FRI opening material in the L1 outer proof:

| Component | Measured size |
|---|---:|
| Fixed-int full batch-STARK checkpoint certificate | `5,933,764` bytes / `5,794.7 KiB` |
| Legacy postcard full checkpoint certificate | `1,162,800` bytes / `1,135.5 KiB` |
| Gzip-best full checkpoint certificate | `366,944` bytes / `358.3 KiB` |
| L1 proof body inside the checkpoint | `149.1 KiB` |
| L1 proof commitments | `4.5 KiB` |
| L1 opened values | `24.0 KiB` |
| L1 opening proof | `117.3 KiB` |
| L1 global lookup data | `3.4 KiB` |

The opening proof dominates because the outer proof is a conventional
multi-table batch-STARK over the executed verifier circuit. The active
`q=9`, `cap_height=5` shape still carries 11 independent Merkle auth-path tree
groups and 1008 raw authentication siblings. A direct path-compression model
saved only `1.4 KiB` on average and `5.4 KiB` in the best sampled case, leaving
the full fixed-int checkpoint floor around `5,789.3 KiB` even in the best
sample. Reducing queries to `q=8` with more query PoW was previously measured
as smaller, but it violates the production terminal policy of not counting
query PoW toward the 60-bit floor and does not address the full-certificate
context overhead.

The native terminal certificate is smaller and faster because it does not wrap
the verifier execution in another batch-STARK proof format. It compiles the
terminal relation directly and proves:

- a primitive sparse-R1CS row-product component, currently `22,631` bytes;
- an exhaustive supported-NPO component, currently `62,909` bytes;
- a single production prelude binding exactly the assignment root.

That is why the recursion-crate terminal fixture measured `85,948` bytes with
release prove `1.492s` and verify `1.181s`. The follow-up `ai-pow-zk`
integration pass added
`prove_terminal_certificate_from_chain_verified_composite_proof`, which proves
the actual composite L1 verifier circuit with the terminal backend and carries
the terminal public input vector alongside the terminal certificate. A
completed release/native reduced-profile run now measures:

| Full composite terminal profile | Certificate | Public inputs | Postcard wire | Compile | Prove | Verify |
|---|---:|---:|---:|---:|---:|---:|
| `lb=6,nq=10,pow=0` | `766,069` bytes / `748.1 KiB` | `5,180` bytes | `771,249` bytes / `753.2 KiB` | `7.606s` | `80.829s` | `58.825s` |

The `lb=6,nq=10,pow=0` profile is the smallest relation measured so far, but
the full composite terminal path is still more than seven times the byte target
and well over the time target. The higher-level miner noun path also still
serializes the batch-STARK checkpoint object.

This `lb=6,nq=10` measurement is a lower-bound diagnostic for recursive
terminal size, not a production profile recommendation. The current PROD
baseline remains the pure-query `lb=4,nq=15,pow=0` inflection point. The
`lb=6,nq=10` row was measured because fewer verifier queries make the
recursive terminal relation smaller; if the native terminal backend misses the
wire/time gates even there, the larger `lb=4,nq=15` and `lb=3,nq=20` recursive
relations are not likely to make the current terminal proof shape production
sized without a deeper proof-shape change. The L0 proving cost of `lb=6` is
not acceptable as an unqualified production default.

The integrated-LogUp polynomial NPO backend is not a production escape hatch
for this full composite path yet. In the recursion crate's synthetic NPO-only
Tip5 test, the bundled checkpoint measures `95,403` bytes and the primitive
plus NPO candidate body measures `96,219` bytes with `23.070s` total prove
time. The full composite diagnostic added after this audit binds the assignment
root plus the merged NPO and bundled Tip5 FRI roots, then proves the primitive
row-product component and attempts the integrated NPO proof over the actual
AI-PoW L1 verifier relation. The release/native run did not reach its final
size print after more than `7m35s` in the test binary, which is enough to
reject it for the current `<30s` production proving gate. It remains a
soundness and size-reduction research path, not the active production recursive
certificate backend.

The newer FRI-native compact residual-zero checkpoint is much more promising
on bytes, and the full-composite residual-zero layer now verifies after fixing
terminal Tip5 Merkle-direction input and hidden-lane reconstruction. The
PROD-layout diagnostic derives the NPO polynomial shape from the compiled
terminal verifying key: `14,049` rows, `16,384` padded rows, `46`
residual-value columns, `89` prover-dependent field columns, and `180` opened
basis columns for the residual-zero check. The resulting proof body is
`55,344` bytes with a `54,023` byte compact FRI payload, the residual table has
`0` nonzero values, and the verification status is `verified`. This is still
only a residual-zero layer, not a complete production terminal proof: the NPO
row-relation quotients, recompose/value-bridge checks, Tip5 lookup/AIR
bindings, primitive row-product checks, and their explicit root/transcript
bindings still have to be included or replaced. The diagnostic also shows work
still needs to be shared: L0 proving, terminal compile, NPO-column
construction, root building, prelude construction, the residual-zero proof, and
verification took `87.229s` wall time in aggregate.

The follow-up merged value-bridge checkpoint includes the residual-zero layer,
the recompose quotient, and the NPO-row value bridge in one FRI-native proof,
then serializes it with the same terminal prelude and primitive row-product
component. It verifies over the full composite relation with body `150,006`
bytes: `240` bytes of prelude, `57,501` bytes of primitive proof, and `92,265`
bytes of merged NPO proof with `90,109` bytes of compact FRI material. After
reusing primitive relation/assignment data, preparing merged NPO data once,
using prelude-checked prover calls, and evaluating the value-bridge quotient by
batched coset LDE, post-prelude proof-body construction is `16.159s` serial:
`9.232s` for the primitive row-product proof plus `6.927s` for the merged
value-bridge proof. Full diagnostic wall time is still `118.017s` because L0
prove, terminal compile, assignment commitment, prepared NPO-root construction,
prelude construction, and verification are outside that proof-body timer.
The bridge now uses the committed `mmcs_bit` to select the bus-to-trace
projection for Merkle Tip5 rows, so `mmcs_bit=1` swaps the input and hidden
lanes consistently with the lookup/AIR trace. Keeping this quotient inside the
existing degree profile means the final production stack must separately prove
that `mmcs_bit` is boolean, zero when not present, and padded consistently with
the row mode; the value bridge alone should not be read as that padding proof.

The same run breaks down the primitive bottleneck. The sparse R1CS relation has
`106,604` rows, `222,449` variables, and `489,990` entries. The row-product and
matrix-sumcheck round messages are small (`1,327` and `1,088` bytes). The
assignment evaluation proof is the primitive proof body: `55,025` of `57,501`
bytes, with `53,586` bytes of fold-round openings. Those openings are dominated
by `48,462` bytes of Merkle frontier material, `1,211` 5-round Tip5 digest
nodes across the 18 assignment fold layers. Precomputing equality-polynomial
tables for sparse R1CS evaluation left the release timing essentially
unchanged, so the next primitive reduction must change the assignment-opening
PCS shape or merge it into a shared proximity backend; scalar sumcheck
serialization is not the lever.

Parallelizing deterministic parent-level hashing in the terminal oracle Merkle
tree builder cuts the setup and primitive proving constants without changing
proof bytes or transcript semantics. In the full composite run, assignment
commitment construction dropped from about `15.854s` to `7.557s`, and primitive
prove time first dropped from about `50.169s` to `42.210s`. Reusing the
primitive relation/assignment vector and skipping redundant prover-side
prelude verification then dropped post-prelude primitive proving to `9.232s`.
This is useful engineering progress, but it does not change the structural
conclusion: the primitive body is still dominated by assignment-evaluation
frontier material, and a production theorem still has to include the missing
NPO lookup/AIR/LogUp bindings.

A direct compact-FRI lower-bound check for the assignment vector does not
change that conclusion. The diagnostic
`terminal_assignment_compact_fri_floor_for_prod_baseline_measures` commits the
`222,449`-entry assignment as one extension-field FRI column over `262,144`
padded rows and serializes to `56,307` bytes, with `56,202` bytes of compact
FRI payload and `41` bytes of opened values. It proves in `18.233s` and
verifies in `9ms` after the same terminal setup, but it is still slightly
larger than the current `55,025` byte Merkle-fold assignment evaluation proof.
More importantly, it is not a sound assignment PCS replacement: it proves a
univariate low-degree opening, not equality to the multilinear assignment
evaluation consumed at the sparse-R1CS matrix-sumcheck point. The next
primitive reduction needs a shared proximity/PCS design that supplies that
binding, or a different primitive arithmetization that avoids the current
assignment-evaluation obligation.

A phase-instrumented rerun made the failure mode more concrete:

| Full composite integrated-LogUp phase | Time |
|---|---:|
| Layer-0 proof generation for the diagnostic fixture | `32.447s` |
| L1 verifier-circuit build | `0.466s` |
| L1 verifier trace execution | `0.045s` |
| Terminal compile | `7.607s` |
| Assignment oracle commitment | `14.281s` |
| Merged NPO prelude root construction | `10.772s` |
| Bundled Tip5 prelude root construction | `13.020s` |
| Terminal prelude build | `7.551s` |
| Primitive R1CS row-product proof | `38.235s` |
| Merged value-bridge proof | `51.902s` |
| Integrated Tip5 LogUp proof | still running when stopped |

The root-construction phases duplicate work that the current subproof provers
redo internally, but removing only that duplication would not be enough: the
primitive row-product and merged value-bridge proofs alone already cost about
`90s`, before the integrated Tip5 LogUp proof completes.

The proof-body split after compact known-index proof encoding is:

| Component | Bytes | Notes |
|---|---:|---|
| Full terminal production body | `765,844` | Excludes outer certificate metadata and terminal public-input vector |
| Primitive R1CS row-product proof | `52,821` | Includes `50,348` bytes of assignment-evaluation material |
| Exhaustive NPO proof | `712,830` | Dominates the body |
| Exhaustive NPO hidden Tip5 values | `92,802` | Deterministic hidden input lanes needed for Merkle Tip5 row checks |
| Exhaustive NPO assignment-witness multiproof | `620,028` | Opens `47,814` assignment values and `5,434` Merkle frontier nodes |

The assignment-witness multiproof split is now measured directly:

| Assignment-witness multiproof component | Bytes / Count | Interpretation |
|---|---:|---|
| Nonzero value limbs | `382,515` bytes | `47,814` nonzero Goldilocks coefficients |
| Sparse nonzero masks | `20,126` bytes | Marks nonzero coefficients in the estimated dense extension-value space |
| Boolean bits | `25` bytes | `24` serialized bytes in the proof component; not a size lever |
| Merkle frontier | `217,362` bytes | `5,434` frontier nodes |
| Estimated non-boolean opened values | `80,492` values | Derived from the D=2 sparse mask length |
| Zero coefficients already elided | `113,170` coefficients | About `905 KiB` of dense coefficient payload already removed |

The sparse coefficient encoding is therefore already doing substantial work.
The remaining assignment-witness multiproof gap is mostly nonzero coefficient
payload plus Merkle authentication, not missed zero-coefficient compression.

This is after two serialization reductions in the known-index multiproof:
fixed-width little-endian field limbs and fixed-width little-endian frontier
digests. Before those reductions, the same run measured `891,780` bytes on the
postcard wire. The remaining gap is therefore not ordinary varint overhead; it
is the exhaustive requirement to reveal and authenticate tens of thousands of
assignment values.

This integration run also exposed a verifier-key reconstruction soundness bug:
two rebuilds of the same composite L1 verifier circuit produced different
terminal relation digests, so a decoded certificate could fail with
`CertificateHeaderMismatch`. The root cause was hash-ordered emission of global
lookup cumulative checks in the recursive batch-STARK verifier circuit. The
builder now emits those checks in sorted name order, and the regression
`terminal_header_rebuilds_deterministically_for_baseline_composite` confirms
that the same Layer-0 proof rebuilds the same terminal header. The terminal
wrapper also uses postcard encoding with a structural round-trip assertion.

The non-proving terminal-relation metrics explain why this full path is slow:

| Metric | TEST_PEARL baseline | PROD baseline |
|---|---:|---:|
| Terminal compile time | `21.947s` | `20.943s` |
| Terminal public input values | `459` | `459` |
| Terminal public input bytes | `5,381` | `5,319` |
| Terminal private input values | `46,002` | `43,443` |
| Terminal operations | `131,242` | `125,961` |
| Primitive operations | `111,656` | `106,349` |
| Const operations | `578` | `582` |
| Public operations | `459` | `459` |
| ALU add operations | `8,939` | `8,832` |
| ALU multiplication operations | `10,317` | `10,234` |
| ALU boolean-check operations | `240` | `255` |
| ALU fused multiply-add operations | `10,195` | `10,117` |
| ALU Horner-accumulator operations | `80,928` | `75,870` |
| Hint operations | `5,577` | `5,563` |
| Supported NPO operations / rows | `14,009` | `14,049` |
| Tip5 rows | `8,002` | `8,081` |
| Recompose rows | `238` | `225` |
| Recompose/coeff rows | `5,769` | `5,743` |
| NPO callsite input slots | `140,046` | `141,232` |
| NPO callsite output slots | `86,027` | `86,778` |
| NPO residual components | `253,882` | `242,798` |
| Circuit fingerprint | `witness=232,554 public=459 private=46,002 ops=131,242` | `witness=221,989 public=459 private=43,443 ops=125,961` |

The terminal public input vector is about `5.3 KiB`, so including it with the
terminal certificate is not the size blocker. The blockers are the generic L1
verifier relation itself: more than `100k` primitive operations, about `14k`
supported NPO rows, and a terminal compile step that already consumes most of
the `<30s` budget before terminal proving begins. Within the primitive side,
the single biggest class is Horner accumulation: `75,870` PROD operations,
about 71% of all primitive operations. Removing the last proof-system PoW
grinding from `CircuitConfig::PROD` only reduced the relation by 30 operations,
so PoW removal is a soundness-policy cleanup rather than the size/time fix.
Those Horner steps are emitted by the generic
FRI/PCS verifier arithmetic for polynomial opening, quotient, and batch
consistency checks (`crates/plonky3-recursion/recursion/src/pcs/fri/verifier.rs`
uses `horner_acc_step` in the FRI verifier). On the NPO side, the rows are
almost entirely in-circuit Tip5 permutation checks (`8,081` rows) and
coefficient recompose links (`5,743` rows). The recompose/coeff table is
currently part of the D=2 recursive-verifier construction and is enabled at
`build_composite_l1_verifier_circuit`; removing it would be a new soundness
change, not a serialization or performance tweak.

The 2026-06-06 lower-bound diagnostic quantifies that temptation and rules it
out as the main lever. Compiling the same PROD relation with
`set_recompose_coeff_ctl_for_decompose_links(false)` is unsound without a
replacement binding, and it saves only **390** terminal operations/NPO rows,
with **0** primitive-operation reduction. Most of the removed
`recompose/coeff` rows become ordinary `recompose` rows (`225` to `5,578`).
The production path must keep the coefficient binding unless a replacement is
proven, and even a perfect replacement would not close the size/time gap by
itself.

## Current Native Terminal Size And Runtime Breakdown

The current native terminal fixture certificate uses exhaustive supported-NPO
row checking, not the large two-subproof polynomial NPO payload. The
recursion-crate Tip5 verifier-circuit release measurement is:

| Component | Current bytes | Role |
|---|---:|---|
| Production certificate body | `85,726` bytes / `83.7 KiB` | Typed native terminal proof body |
| Production certificate | `85,948` bytes / `83.9 KiB` | Consensus-facing recursive certificate envelope |
| `primitive_r1cs_proof` / row-product sumcheck | `22,631` bytes | Proves primitive recursive-circuit rows through sparse-R1CS row-product sumcheck and assignment evaluation |
| `npo_exhaustive_proof` | `62,909` bytes | Opens every supported Tip5/recompose NPO callsite against the same assignment oracle and checks each row deterministically |
| Exhaustive hidden Tip5 input payload | `17,402` bytes | Revealed hidden Tip5 lanes needed to recompute hidden-input rows |
| Exhaustive assignment-witness multiproof | `45,507` bytes | Known-index Merkle multiproof binding NPO row witnesses to the assignment oracle |

The release-profile timing from the same run is `prove=1.492s` and
`verify=1.181s`. The standalone exhaustive NPO component measured
`prove=0.162s` and `verify=0.288s`; the standalone R1CS row-product component
measured `prove=0.740s` and `verify=0.515s`.

Source-backed production shape:

- `crates/plonky3-recursion/recursion/src/terminal.rs::TerminalProductionProof`
  serializes a prelude, a `TerminalR1csRowProductSumcheckProof`, and an optional
  `TerminalNpoExhaustiveProof`.
- `prove_terminal_production_goldilocks` builds one production prelude binding
  exactly the assignment root, proves the primitive R1CS component against that
  assignment oracle, and for supported NPO rows proves exhaustive row openings
  against the same assignment oracle.
- `verify_terminal_production_goldilocks` rejects extra prelude commitments,
  verifies the primitive row-product proof, and then verifies every supported
  NPO row with `verify_terminal_npo_exhaustive_goldilocks`.

The terminal profile remains the canonical pure-query 60-bit profile:
`TerminalProofParameters::production_60bit()` uses `log_blowup=4`,
`num_queries=15`, and `query_pow_bits=0`. The exhaustive NPO component itself
does not rely on sampling or terminal query PoW: it checks every supported NPO
row deterministically against verifier-derived callsites and assignment-oracle
openings.

The polynomial/proximity NPO backend remains in tree as a diagnostic and future
hardening track, but it is not the production wire artifact. The current
diagnostic measurements explain why it was removed from production:

| Diagnostic candidate | Bytes | Meaning |
|---|---:|---|
| Previous polynomial production certificate | `226,248` bytes / `220.9 KiB` | Failed the hard size gate |
| Previous `TerminalProductionNpoPolynomialProof` | `204,039` bytes | Dominated the old body |
| Previous `merged_value_bridge_proof` | `67,133` bytes | One independent FRI-backed NPO subproof |
| Previous `integrated_logup_proof` | `136,906` bytes | Second independent FRI-backed NPO subproof |
| Full NPO polynomial FRI opening candidate | `48,803` bytes / `47.7 KiB` | A single FRI opening over 668 rows and 186 field columns is far smaller than the old two-subproof NPO body |
| NPO value-column FRI candidate | `30,325` bytes / `29.6 KiB` | Value-only proximity proof is not the blocker by itself |

Engineering conclusion: exhaustive supported-NPO checking made the terminal
fixture small enough. It has not yet made the full `ai-pow-zk` composite
recursive path small or fast enough. Verifier-key reconstruction is now
canonical for the baseline diagnostic, and the `lb=6,nq=10,pow=0` measurement
has complete wire bytes and release timing. The remaining work is to reduce the
actual composite L1 terminal relation, especially the generic FRI/MMCS verifier
circuit and supported-NPO callsite count.

## Soundness Summary

| Layer | Parameters | Soundness claim | PoW counted? |
|---|---|---:|---|
| Layer-0 composite STARK | `log_blowup=4`, `num_queries=15`, `pow_bits=0` in `CircuitConfig::PROD` | 60 pure FRI-query bits under the code's Johnson accounting | No |
| Hardened L1 batch-STARK checkpoint | `log_blowup=4`, `num_queries=9`, `query_pow_bits=24`, `cap_height=5` | 60 bits under mixed query/PoW Johnson accounting | Yes; acceptable for the checkpoint, not for the terminal production target |
| Relaxed L1-only batch-STARK pure-query diagnostics | `lb=4,nq=15,pow=0`, `lb=5,nq=12,pow=0`, `lb=6,nq=10,pow=0` | 60 pure FRI-query bits under the code's Johnson accounting | No; all measured variants miss size/time |
| Production native terminal backend | `log_blowup=4`, `num_queries=15`, `query_pow_bits=0`, `max_log_arity=3`, `log_final_poly_len=0` | Intended 60 pure FRI-query bits for the terminal backend, conditionally on the selected Plonky3 FRI theorem/assumption and terminal theorem; fixture proof-size gate passes, but the full `ai-pow-zk` composite-verifier path has not met the release-time gate | No |
| End-to-end production recursive certificate target | L0 proof accepted by the native terminal recursive-verifier certificate | At most the minimum of the L0 and terminal layers: **60 bits** | No terminal query PoW counted |

The recursive certificate does not make the underlying Layer-0 statement more
sound. It replaces the large Layer-0 proof object with a smaller proof that the
recursive verifier accepted that Layer-0 proof. A successful production forgery
must either forge the Layer-0 STARK statement, forge the native terminal proof
that the verifier accepted it, or break one of the transcript/commitment
assumptions that bind the two. The hardened L1 batch-STARK path has its own
60-bit checkpoint claim but is not the production size/time target.

## Logic Flow

```text
AI-PoW attempt data
  |
  | native puzzle checks derive nonce-bound kappa, matrix commitments,
  | noised matmul values, cumsum, jackpot message, and jackpot hash
  v
Layer-0 composite STARK
  proves:
    - canonical program is pinned, not prover-selected;
    - public inputs are bound:
      cumsum, jackpot, HASH_A, HASH_B, JOB_KEY, COMMITMENT_HASH,
      HASH_JACKPOT;
    - BLAKE3 matrix/jackpot hash AIR rows match the public commitments;
    - noised matrix/matmul/range/i8-u8/cv routing lookups are globally
      consistent through LogUp;
    - FRI openings prove low-degree trace/quotient consistency.
  |
  | recursive verifier circuit runs the Layer-0 verifier
  v
Production native terminal certificate target
  proves:
    - the verifier circuit was executed with the committed Layer-0 proof,
      public inputs, relation profile, and production parameters;
    - terminal proof parameters are the canonical pure-query 60-bit tuple;
    - the terminal header, public-values digest, relation digest, proximity
      schedule, fixed terminal tables, and commitments are bound before
      terminal challenges;
    - primitive recursive-circuit rows and supported Tip5/recompose NPO rows
      are covered by the terminal primitive sparse-R1CS row-product proof and
      exhaustive supported-NPO row openings against the assignment oracle.
  |
  v
Nockchain block/wire artifact target: structured native terminal certificate
```

The hardened L1 batch-STARK checkpoint follows the same Layer-0 verifier-circuit
handoff, but proves the executed verifier circuit with `BatchStarkProver`
instead of the native terminal backend. It is useful for regression and
fallback validation, not for the production wire budget.

## Assumptions By Step

### 1. Native AI-PoW Attempt And Public Statement

Cryptographic assumptions:

- BLAKE3 behaves as a collision-resistant hash and keyed hash/MAC for matrix
  commitments and jackpot hashing.
- The nonce/ticket attempt state is unique: changing the nonce, ticket, matrix,
  noise, target, or public commitments changes the derived statement before
  proof construction.
- The verifier recomputes the public statement instead of trusting
  prover-supplied metadata.
- In the Pearl merge-mining path, cheap noun metadata precheck re-derives and
  compares the Pearl-bound slots (`HASH_A`, `HASH_B`, `JOB_KEY`,
  `COMMITMENT_HASH`, `JACKPOT`, `HASH_JACKPOT`) from the ticket and trusted
  matrices. It does not independently derive `cumsum`; `cumsum` remains bound
  by the Layer-0 proof and by full recursive verification of the exact public
  input vector carried in the certificate.

Citations and anchors:

- BLAKE3 specification: O'Connor, Aumasson, Neves, Wilcox-O'Hearn,
  "BLAKE3: one function, fast everywhere",
  <https://github.com/BLAKE3-team/BLAKE3-specs/blob/master/blake3.pdf>.
- Current statement-binding docs:
  `2026-05-31_AI_POW_ONE_MATMUL_ONE_ATTEMPT_AUDIT.md` and
  `crates/ai-pow/src/zk_bridge.rs`.

### 2. Layer-0 Composite STARK

Cryptographic assumptions:

- The Plonky3 STARK reduction is sound for the committed AIR, public inputs,
  LogUp buses, and quotient identities.
- FRI proximity/opening verification is sound for the production Goldilocks
  rate and 15 transcript-derived queries.
- Fiat-Shamir challenges are modeled as random-oracle challenges derived after
  all relevant statement data and commitments are bound.
- The Tip5 Merkle/MMCS commitment is binding/collision-resistant for the
  committed trace, quotient, and lookup columns.
- LogUp rational-sum identities are sound except for standard
  Schwartz-Zippel/denominator-pole failure probabilities over the extension
  challenge field.

Implementation anchors:

- `crates/ai-pow-zk/src/composite_proof.rs` documents the production Layer-0
  family: `composite_prove_pinned_logup` /
  `composite_verify_pow_pinned_logup`.
- `crates/ai-pow-zk/src/circuit.rs::CircuitConfig::PROD` sets
  `log_blowup=4`, `num_queries=15`, and `pow_bits=0`.
- `crates/ai-pow-zk/README.md` records the current Layer-0 soundness policy:
  60 pure query bits with no proof-system PoW grinding counted or enabled.

Citations:

- STARKs: Ben-Sasson, Bentov, Horesh, Riabzev, "Scalable, transparent, and
  post-quantum secure computational integrity", IACR ePrint 2018/046,
  <https://eprint.iacr.org/2018/046.pdf>.
- FRI: Ben-Sasson, Bentov, Horesh, Riabzev, "Fast Reed-Solomon Interactive
  Oracle Proofs of Proximity", ICALP 2018,
  <https://doi.org/10.4230/LIPIcs.ICALP.2018.14>.
- DEEP-FRI context: Ben-Sasson, Goldberg, Kopparty, Saraf, IACR ePrint
  2019/336, <https://eprint.iacr.org/2019/336>.
- Fiat-Shamir for FRI and batched FRI: Block, Garreta, Katz, Thaler, Tiwari,
  Zajac, IACR ePrint 2023/1071, <https://eprint.iacr.org/2023/1071>.
- LogUp/logarithmic-derivative lookups: Haboeck, IACR ePrint 2022/1530,
  <https://eprint.iacr.org/2022/1530>.
- Tip5: Szepieniec, Lemmens, Sauer, Threadbare, Al Kindi, "The Tip5 Hash
  Function for Recursive STARKs", IACR ePrint 2023/107,
  <https://eprint.iacr.org/2023/107.pdf>.
- Reed-Solomon proximity-gap policy anchor used by current repo docs:
  Ben-Sasson, Carmon, Haboeck, Kopparty, Saraf, "On Proximity Gaps for
  Reed-Solomon Codes", IACR ePrint 2025/2055,
  <https://eprint.iacr.org/2025/2055>.

### 3. Recursive Verifier Circuit

Cryptographic assumptions:

- The recursive verifier circuit faithfully implements the native Layer-0
  verifier transcript, commitment observations, FRI query derivation, Merkle
  path checks, LogUp checks, and public-input binding.
- The in-circuit 5-round Tip5 operations match the native 5-round Tip5
  permutation used by Layer-0 challenger/MMCS commitments.
- The recursive statement binds the same Layer-0 public-input vector and
  relation/profile metadata that the native verifier would use.

Implementation anchors:

- `crates/ai-pow-zk/src/recursion.rs::recurse_composite_to_l1` defines the
  batch-STARK checkpoint pipeline: prove Layer 0, build the L1 verifier
  circuit, verify Layer 0 in-circuit, and prove the verifier circuit.
- `crates/plonky3-recursion/recursion/src/terminal.rs` defines the native
  terminal compiler/certificate interface for the production recursive target.
- `crates/plonky3-recursion/recursion/src/pcs/fri/params.rs` requires the safe
  `with_mmcs` constructor for production FRI verification; the arithmetic-only
  path is explicitly unsafe/test-only.

Citations:

- Plonky3 recursion model:
  <https://plonky3.github.io/Plonky3-recursion/introduction.html>.
- Tip5 paper as above.
- FRI and Fiat-Shamir-for-FRI references as above.

### 4. Native Terminal Recursive Certificate Target

Cryptographic assumptions:

- The production terminal proof uses the canonical pure-query tuple
  `log_blowup=4`, `num_queries=15`, `query_pow_bits=0`; no terminal query PoW
  is counted toward the 60-bit floor.
- The terminal header, public-values digest, backend relation digest, NPO
  polynomial profile, column layout, fixed Tip5 lookup preprocessed-table
  digest, prelude parameters, relation profile, proximity profile, and backend
  commitment roots are absorbed before terminal challenges.
- Primitive circuit constraints are checked through the sparse-R1CS row-product
  sumcheck plus assignment evaluation proof.
- Supported Tip5/recompose NPO rows are checked exhaustively: verifier-derived
  callsites determine the exact assignment-witness openings, hidden Tip5 lanes,
  MMCS direction bits, row modes, and predecessor-chain semantics to verify.
- Terminal FRI/PCS openings for primitive row-product sumcheck and 5-round Tip5
  Merkle commitments are binding under the stated Plonky3 FRI and Tip5
  assumptions.

Implementation anchors:

- `docs/ai-pow-integration/2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md`
  records the production terminal interface and theorem. Its previous
  polynomial-NPO checkpoint was too large, but the current exhaustive-NPO
  recursion-crate fixture measurement is `85,948` bytes / `83.9 KiB`.
- `crates/plonky3-recursion/recursion/src/terminal.rs` implements
  `TerminalCertificate`, `TerminalProofParameters::production_60bit`,
  `prove_terminal_production_goldilocks`, and
  `verify_terminal_production_goldilocks`.
- `crates/ai-pow-zk/src/recursion.rs` now implements the opt-in
  `AiPowTerminalRecursiveCertificate` path for the actual composite L1 verifier
  circuit. Its `lb=6,nq=10,pow=0` release/native measurement verifies after
  postcard decode, but measures `771,249` wire bytes with `80.829s` prove time
  and `58.825s` verify time.
- `crates/plonky3-recursion/recursion/src/terminal.rs` stores known-index
  multiproof field limbs and frontier digests in fixed little-endian bytes,
  reducing the same full-path wire measurement from `891,780` bytes to
  `771,249` bytes without changing the checked Merkle roots or assignment
  values.
- `crates/plonky3-recursion/recursion/src/verifier/batch_stark.rs` emits global
  lookup cumulative checks in sorted order so terminal verifier-key
  reconstruction is deterministic and bound by a stable relation digest.
- Terminal production tests reject malformed proof bodies, wrong proof kind,
  noncanonical parameters, missing commitments, missing exhaustive assignment
  openings, tampered hidden Tip5 input payloads, and tampered assignment
  witness multiproofs. The recursion-crate Tip5 verifier-circuit fixture test
  passes the hard size gate at `85,948` bytes / `83.9 KiB`.

Citations:

- FRI, Fiat-Shamir-for-FRI, LogUp, and Tip5 citations are the same as the
  Layer-0 section because the terminal backend uses these same families of
  assumptions.
- The terminal-specific soundness theorem is documented in
  `2026-06-03_NATIVE_TERMINAL_COMPRESSION_SPEC.md`.

### 5. Hardened L1 Batch-STARK Checkpoint

Cryptographic assumptions:

- The L1 circuit-prover batch-STARK verifies against the submitted proof body,
  not only proof-carried metadata.
- The verifier rebuilds the canonical L1 verifier circuit from the embedded
  Layer-0 proof, pinned program, production profile, and verifier-derived
  public inputs, then rejects if submitted L1 metadata differs from the rebuilt
  canonical shape.
- The verifier registers only the production Tip5 and recompose non-primitive
  tables before calling `verify_all_tables`.
- The L1 FRI/PCS proof is sound under the active mixed query/PoW checkpoint
  profile.

Implementation anchors:

- `crates/ai-pow-zk/src/recursion.rs::verify_recursive_certificate` rebuilds
  and runs the L1 verifier circuit, compares stable metadata against a
  canonical rebuilt proof, and verifies the submitted outer proof with
  `BatchStarkProver::verify_all_tables`.
- `crates/plonky3-recursion/circuit-prover/src/batch_stark_prover.rs` defines
  `verify_all_tables` and `verify_all_tables_with_public_values`.
- `crates/ai-pow-zk/src/recursion.rs::recursive_certificate_rejects_outer_proof_body_tamper`
  is the regression test for metadata-preserving proof-body tampering.

This checkpoint is cryptographically relevant and should remain sound, but it
does not satisfy the production wire budget.

## What Is Not Being Assumed

- No trusted setup.
- No KZG, pairing-friendly curve, Groth16, or Plonkish SNARK wrapper.
- No Plonky2 proof system in production. Pearl/Plonky2 code was read only as a
  design reference for safe FRI path compression, and the native terminal
  backend is implemented in the vendored Plonky3-recursion stack.
- No claim that the multi-hundred-KiB-to-MiB L1 batch-STARK checkpoint is the production
  block/wire artifact.
- No zero-knowledge claim for the production native terminal certificate.
  Exhaustive NPO openings reveal selected recursive-verifier witness material,
  including hidden Tip5 input lanes needed to recompute hidden-input rows.

## Clear End-To-End Claim

For the intended production path, the block-facing recursive proof target is
still the native terminal certificate, not the batch-STARK checkpoint. However,
the full end-to-end production claim is not yet proven. The recursion-crate
Tip5 verifier fixture satisfies the hard constraint at **85,948 bytes /
83.9 KiB** with release prove **1.492 s** and verify **1.181 s**, but the
newly wired full `ai-pow-zk` composite-verifier terminal path measures
**771,249 bytes / 753.2 KiB** on the postcard wire with release prove
**80.829 s** and verify **58.825 s** even under the reduced
`lb=6,nq=10,pow=0` profile. The materialized Layer-0 proof is **304,048 bytes /
296.9 KiB**, but it is an intermediate diagnostic artifact rather than the
target consensus wire object.

The hardened batch-STARK L1 proof body is **149.1 KiB**, and the full checkpoint
certificate is **1,135.5 KiB** as legacy postcard, **5,794.7 KiB** as fixed-int
bincode, and **358.3 KiB** with gzip-best compression. It is retained as a
soundness-hardened checkpoint and fallback verifier target, not as the
production recursive certificate path.

The end-to-end soundness floor is **60 bits**, with the following reduction:

1. If the AI-PoW computation/public statement is false, a valid Layer-0 proof
   requires breaking the Layer-0 STARK/FRI/LogUp/Tip5/BLAKE3 assumptions or
   exploiting a bug in the AIR/public-input binding.
2. If the Layer-0 verifier would reject, a valid production terminal
   certificate requires breaking the terminal primitive sparse-R1CS,
   exhaustive supported-NPO assignment-oracle openings, FRI/PCS, Tip5
   commitment, or transcript-binding assumptions, or exploiting a bug in the
   recursive-verifier relation.
3. The certificate binds public values, relation/profile metadata, commitments,
   and production parameters before challenge derivation, so there is no
   intended grinding surface over public values, profiles, roots, or query
   indices.

The intended weakest production soundness term remains the 60-bit floor shared
by the Layer-0 proof and native terminal certificate. The open engineering
work is to make the actual composite-verifier terminal proof meet the
production size/time gates and to wire that artifact above `ai-pow-zk` without
falling back to the too-large batch-STARK checkpoint. The hardened batch-STARK
path is also required to stay cryptographically sound, but it is not the
production size/time path.
