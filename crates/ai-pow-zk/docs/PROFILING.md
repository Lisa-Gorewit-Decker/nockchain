> _Created **2026-05-15** · last updated **2026-05-15** · organized into `crates/ai-pow-zk/docs/` on 2026-05-19._

# Profiling & benchmarking the ai-pow → ai-pow-zk pipeline

The F1 harness (`crates/ai-pow/examples/f1_harness.rs`) is the
single instrumented cross-crate fixture: a real `ai-pow` solve at
`TEST_SMALL` → `ai-pow-zk` `composite_prove` /
`composite_verify_pow`, with a hard byte-equivalence assertion
tying the SNARK's `HASH_A` PI to `ai-pow`'s `BlockContext.h_a_chunk`.
It is the substrate every profiling / benchmarking recipe targets
(per `GAP_AUDIT.md` — instrumentation is non-negotiable F1 scope,
not a follow-on).

## TL;DR

```sh
scripts/profile_f1.sh run            # one-shot metrics line
scripts/profile_f1.sh rss   3        # peak RSS over 3 iters
scripts/profile_f1.sh samply 5       # CPU profile (5 iters, prover-dominated)
scripts/profile_f1.sh all   3        # metrics + rss + samply
```

`ITERS` (2nd arg → `F1_ITERS`) repeats prove+verify so the
sampler/RSS sees a prover-dominated run. Use 1 for a quick read,
3–5 for `samply`.

## The metrics line

```
f1_harness shape=TEST_SMALL seed=f1-harness-v1 iters=1 \
  mine_ms=3 trace_gen_ms=6 prove_ms=33346 verify_ms=38 \
  proof_bytes=306815 num_pis=60 plain_h_a_chunk_ok=true
```

| field | meaning |
|---|---|
| `mine_ms` | real `ai-pow` solve wall time |
| `trace_gen_ms` | `CompositeTrace` build + `place_matrix_hash_a/b` |
| `prove_ms` | `composite_prove` (mean over `iters`) |
| `verify_ms` | `composite_verify_pow` incl. C2 difficulty check (mean) |
| `proof_bytes` | bincode-encoded proof size |
| `num_pis` | public-input vector length (60 after C1–C4) |
| `plain_h_a_chunk_ok` | cross-crate byte-equivalence assertion result |

Parse it with `awk` for CI bench tracking (see below).

## samply (CPU flame profile)

`samply` records a sampling profile viewable in the Firefox
Profiler (no nightly, no perf-event privileges needed on macOS).

```sh
cargo install samply                       # once
scripts/profile_f1.sh samply 5             # → f1_profile_<ts>.json.gz
samply load f1_profile_<ts>.json.gz        # opens the UI
```

Reading the trace for bottlenecks:

- The hot stack is almost entirely under `composite_prove`. Drill
  into it; expect FRI commit (LDE + Merkle) and quotient
  evaluation to dominate — that is the `rows × log_blowup`
  cost the FRI sweep (`ENGINEERING_REPORT.md §11`) quantifies.
- `place_matrix_hash_*` shows up under `trace_gen`; if it's
  material at `TEST_SMALL` it will be catastrophic at PROD
  (M52 step 7) — a useful early-warning signal.
- Inverted ("bottom-up") view groups by leaf function: look for
  Tip5 permutation, Goldilocks mul, and Merkle hashing — those
  are the levers recursion (M12) or a field/hash swap would move.

For an interactive (auto-open) profile, drop `--save-only` from
the script's `samply record` line.

## Peak RSS (memory ceiling — GAP_AUDIT P2)

No code dependency; uses the platform `time`:

- **macOS**: `/usr/bin/time -l` → "maximum resident set size"
  (bytes).
- **Linux**: `/usr/bin/time -v` → "Maximum resident set size
  (kbytes)".

```sh
scripts/profile_f1.sh rss 3
# peak_rss_bytes=… (… MiB)
```

This is the GAP_AUDIT P2 lever: we still have no hard memory
bound, and `num_pis` just went 36 → 60 with three new constraint
families. Track `peak_rss` alongside `prove_ms` so the M12 / PROD
scaling decision has data.

## CI bench tracking (GAP_AUDIT P4) — recommended wiring

The metrics line is intentionally machine-readable. A minimal
tracking job:

```sh
scripts/profile_f1.sh run \
  | tee -a bench_history.tsv \
  | awk '{for(i=1;i<=NF;i++)print $i}' \
  | grep -E 'prove_ms|verify_ms|proof_bytes'
```

Pin three shapes once the harness grows a `--profile` knob
(TEST_SMALL today; TEST_PEARL / PROD when F1 deepens) and fail CI
on a >X% `prove_ms` / `proof_bytes` regression. This is the
cheapest durable lever — every M52/C1–C4-class change has so far
shifted these numbers silently.

## What this does NOT measure yet

The faithful jackpot→blake3 instruction chain (deep F1) that
makes `HASH_JACKPOT` / `JOB_KEY` / `COMMITMENT_HASH` non-zero
PIs. Those are zero here, so the C1/C4 bindings are vacuous and
`composite_verify_pow` clears any target. The harness measures
the matrix-binding + prove/verify pipeline, which is what's
wired; extend it when deep F1 lands.
