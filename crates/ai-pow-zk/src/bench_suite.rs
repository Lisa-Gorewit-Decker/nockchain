//! Multi-shape, multi-activity benches for the M10.1c stack.
//!
//! Every bench in this module is `#[ignore]`'d so the default
//! `cargo test` doesn't pay the prove cost. Run a specific bench
//! with:
//!
//! ```sh
//! cargo test -p ai-pow-zk --release --lib bench_suite::bench_8k_baseline -- \
//!     --ignored --nocapture
//! ```
//!
//! ## What we measure
//!
//! Each bench reports four numbers + proof size:
//!
//! | Metric | What it measures |
//! |---|---|
//! | `trace_gen_ms` | Time to build the `CompositeTrace` (baseline + activity placement). Excludes `populate_lookup_freq`. |
//! | `populate_ms` | Time spent in `populate_lookup_freq` only. Often non-trivial — scans every row × every queried column and allocates two hashmaps. |
//! | `prove_ms` | Time inside `prove_batch`. Dominated by LDE + FRI commits at large shapes. |
//! | `verify_ms` | Time inside `verify_batch`. Should stay roughly constant per FRI query count regardless of trace size. |
//! | `proof_bytes` | Size of the bincode-encoded proof. Scales with trace size (more LDE openings to encode). |
//!
//! ## Shapes
//!
//! Trace lengths in rows: 8192 (= `MIN_STARK_LEN`), 16384, 32768. Power-of-2.
//!
//! ## Activity levels
//!
//! * **Baseline:** no chip activity. Range tables filled, I8U8 filled,
//!   STARK_ROW_IDX monotonic; every other cell is zero. All 7
//!   LogUp buses balance trivially.
//! * **Light:** one BLAKE3 hash (rows 0..8), one matmul reset+update
//!   chain (rows 8..10), one jackpot step (row 10). Same shape as
//!   the `three_chip_integration_verifies` regression test but at
//!   varying trace sizes.
//! * **Heavy:** 100 BLAKE3 hashes back-to-back (rows 0..800), 100
//!   matmul update steps (rows 800..900), 100 jackpot rotations
//!   (rows 900..1000). Trace size must be ≥ 1024 for this to fit;
//!   we only run heavy at 8K and 16K.

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use bincode::config::standard as bincode_standard;
    use p3_batch_stark::{prove_batch, verify_batch, ProverData, StarkInstance};

    use crate::chips::blake3::compress::{Blake3Tweak, BLAKE3_IV};
    use crate::chips::jackpot::compute::apply_jackpot_step;
    use crate::chips::matmul::compute::CUMSUM_LEN;
    use crate::circuit::{build_stark_config, AiPowStarkConfig, CircuitConfig};
    use crate::composite_full_air_with_lookups::CompositeFullAirWithLookups;
    use crate::composite_layout::{JACKPOT_SIZE, MIN_STARK_LEN, TILE_D, TILE_H, TOTAL_TRACE_WIDTH};
    use crate::composite_trace::CompositeTrace;
    use crate::params::ZkParams;

    fn test_zk_params() -> ZkParams {
        ZkParams {
            m: 8,
            k: 16,
            n: 8,
            noise_rank: 2,
            tile: 2,
            difficulty_bits: 0,
        }
    }

    /// Outcomes for one bench run.
    #[derive(Debug)]
    struct BenchResult {
        rows: usize,
        cols: usize,
        activity: &'static str,
        trace_gen_ms: u128,
        populate_ms: u128,
        prove_ms: u128,
        verify_ms: u128,
        proof_bytes: usize,
    }

    impl BenchResult {
        fn print(&self) {
            println!(
                "ai-pow-zk bench [{} @ {}×{}]: trace_gen={} ms, populate={} ms, prove={} ms, verify={} ms, proof={} B",
                self.activity,
                self.rows,
                self.cols,
                self.trace_gen_ms,
                self.populate_ms,
                self.prove_ms,
                self.verify_ms,
                self.proof_bytes,
            );
        }
    }

    /// Build a baseline-only trace at the given row count and run
    /// prove_batch + verify_batch through `CompositeFullAirWithLookups`.
    fn run_baseline_at(n: usize, profile: &CircuitConfig) -> BenchResult {
        let cfg = build_stark_config(&test_zk_params(), profile);

        let t = Instant::now();
        let mut trace = CompositeTrace::baseline(n);
        let trace_gen_ms = t.elapsed().as_millis();

        let t = Instant::now();
        trace.populate_lookup_freq();
        let populate_ms = t.elapsed().as_millis();

        run_logup(trace, &cfg, n, "baseline", trace_gen_ms, populate_ms)
    }

    /// Build a "light activity" trace — one BLAKE3 hash + one
    /// matmul chain + one jackpot rotation — at the given row count.
    fn run_light_at(n: usize, profile: &CircuitConfig) -> BenchResult {
        let cfg = build_stark_config(&test_zk_params(), profile);

        let t = Instant::now();
        let mut trace = CompositeTrace::baseline(n);

        // (a) BLAKE3 hash @ rows 0..7.
        let cv: [u32; 8] = core::array::from_fn(|i| BLAKE3_IV[i]);
        let msg: [u32; 16] = core::array::from_fn(|i| (i as u32 + 1) * 0xABCDEF);
        let tweak = Blake3Tweak {
            counter_low: 42,
            counter_high: 0,
            block_len: 64,
            flags: 0x1B,
        };
        let _cv_out = trace.place_blake3_hash(0, &msg, &cv, &tweak);

        // (b) Matmul reset + update @ rows 8, 9.
        let a = [[0i8; TILE_D]; TILE_H];
        let b = [[0i8; TILE_D]; TILE_H];
        let zero_cumsum = [0i32; CUMSUM_LEN];
        let after_reset = trace.place_matmul_step(8, &a, &b, true, false, &zero_cumsum);
        let after_update = trace.place_matmul_step(9, &a, &b, false, true, &after_reset);
        trace.fill_cumsum_passthrough(10, &after_update);

        // (c) Jackpot @ row 10.
        let initial_jackpot = [0u32; JACKPOT_SIZE];
        let _after_step = trace.place_jackpot_step(10, &initial_jackpot, 0, 0xDEAD_BEEF, true);
        let jackpot_final = apply_jackpot_step(&initial_jackpot, 0, 0xDEAD_BEEF, true);
        trace.fill_jackpot_passthrough(11, &jackpot_final);

        let trace_gen_ms = t.elapsed().as_millis();

        let t = Instant::now();
        trace.populate_lookup_freq();
        let populate_ms = t.elapsed().as_millis();

        run_logup(trace, &cfg, n, "light", trace_gen_ms, populate_ms)
    }

    /// Build a "heavy activity" trace — N_BLAKE3 BLAKE3 hashes,
    /// N_MATMUL matmul steps, N_JACKPOT jackpot rotations, all
    /// back-to-back. Requires the trace size to fit all activity.
    fn run_heavy_at(
        n: usize,
        n_blake3: usize,
        n_matmul: usize,
        n_jackpot: usize,
        profile: &CircuitConfig,
    ) -> BenchResult {
        let blake3_rows = n_blake3 * 8;
        let matmul_rows = n_matmul;
        let jackpot_rows = n_jackpot;
        let total_active = blake3_rows + matmul_rows + jackpot_rows;
        assert!(
            total_active < n,
            "heavy activity ({}) exceeds trace size ({})",
            total_active,
            n
        );

        let cfg = build_stark_config(&test_zk_params(), profile);

        let t = Instant::now();
        let mut trace = CompositeTrace::baseline(n);

        // (a) BLAKE3 hashes back-to-back at rows 0..(8 * n_blake3).
        //     Each hash uses the previous hash's CV_OUT as CV_IN.
        let mut cv: [u32; 8] = core::array::from_fn(|i| BLAKE3_IV[i]);
        let tweak = Blake3Tweak {
            counter_low: 0,
            counter_high: 0,
            block_len: 64,
            flags: 0x1B,
        };
        for h in 0..n_blake3 {
            let msg: [u32; 16] = core::array::from_fn(|i| (h as u32 + 1) * 0x01010101 ^ (i as u32));
            let cv_out = trace.place_blake3_hash(h * 8, &msg, &cv, &tweak);
            cv = cv_out;
        }

        // (b) Matmul steps at rows [blake3_rows..blake3_rows+n_matmul).
        let a = [[0i8; TILE_D]; TILE_H];
        let b = [[0i8; TILE_D]; TILE_H];
        let mut cumsum = [0i32; CUMSUM_LEN];
        for s in 0..n_matmul {
            let row = blake3_rows + s;
            let is_reset = s == 0;
            let is_update = !is_reset;
            cumsum = trace.place_matmul_step(row, &a, &b, is_reset, is_update, &cumsum);
        }
        let cumsum_passthrough_start = blake3_rows + n_matmul;
        trace.fill_cumsum_passthrough(cumsum_passthrough_start, &cumsum);

        // (c) Jackpot steps at rows [cumsum_passthrough_start..).
        let mut state = [0u32; JACKPOT_SIZE];
        for s in 0..n_jackpot {
            let row = cumsum_passthrough_start + s;
            let slot = s % JACKPOT_SIZE;
            let x = (s as u32 + 1) * 0xCAFE;
            state = trace.place_jackpot_step(row, &state, slot, x, true);
        }
        let jackpot_passthrough_start = cumsum_passthrough_start + n_jackpot;
        trace.fill_jackpot_passthrough(jackpot_passthrough_start, &state);

        let trace_gen_ms = t.elapsed().as_millis();

        let t = Instant::now();
        trace.populate_lookup_freq();
        let populate_ms = t.elapsed().as_millis();

        run_logup(trace, &cfg, n, "heavy", trace_gen_ms, populate_ms)
    }

    /// Shared tail: derive PIs, prove + verify + measure proof
    /// size.
    fn run_logup(
        trace: CompositeTrace,
        cfg: &AiPowStarkConfig,
        n: usize,
        activity: &'static str,
        trace_gen_ms: u128,
        populate_ms: u128,
    ) -> BenchResult {
        let pis =
            crate::composite_public::CompositePublicInputs::derive_from_trace(&trace).to_vec();

        let air = CompositeFullAirWithLookups;
        let instances = vec![StarkInstance {
            air: &air,
            trace: &trace.matrix,
            public_values: pis.clone(),
        }];

        let prover_data = ProverData::from_instances(cfg, &instances);

        let t = Instant::now();
        let proof = prove_batch(cfg, &instances, &prover_data);
        let prove_ms = t.elapsed().as_millis();

        let t = Instant::now();
        verify_batch(cfg, &[air], &proof, &[pis], &prover_data.common).expect("bench verify");
        let verify_ms = t.elapsed().as_millis();

        let encoded =
            bincode::serde::encode_to_vec(&proof, bincode_standard()).expect("bincode encode");

        BenchResult {
            rows: n,
            cols: TOTAL_TRACE_WIDTH,
            activity,
            trace_gen_ms,
            populate_ms,
            prove_ms,
            verify_ms,
            proof_bytes: encoded.len(),
        }
    }

    // =================================================================
    //  TEST_PEARL-profile benches (cheaper, useful for relative
    //  scaling). Use for the 16K/32K shapes where PROD would be too
    //  slow for an interactive run.
    // =================================================================

    #[test]
    #[ignore = "bench — TEST_PEARL profile @ 8K baseline"]
    fn bench_test_pearl_8k_baseline() {
        let r = run_baseline_at(MIN_STARK_LEN, &CircuitConfig::TEST_PEARL);
        r.print();
    }

    #[test]
    #[ignore = "bench — TEST_PEARL profile @ 16K baseline"]
    fn bench_test_pearl_16k_baseline() {
        let r = run_baseline_at(MIN_STARK_LEN * 2, &CircuitConfig::TEST_PEARL);
        r.print();
    }

    #[test]
    #[ignore = "bench — TEST_PEARL profile @ 32K baseline"]
    fn bench_test_pearl_32k_baseline() {
        let r = run_baseline_at(MIN_STARK_LEN * 4, &CircuitConfig::TEST_PEARL);
        r.print();
    }

    #[test]
    #[ignore = "bench — TEST_PEARL profile @ 8K light"]
    fn bench_test_pearl_8k_light() {
        let r = run_light_at(MIN_STARK_LEN, &CircuitConfig::TEST_PEARL);
        r.print();
    }

    #[test]
    #[ignore = "bench — TEST_PEARL profile @ 16K light"]
    fn bench_test_pearl_16k_light() {
        let r = run_light_at(MIN_STARK_LEN * 2, &CircuitConfig::TEST_PEARL);
        r.print();
    }

    #[test]
    #[ignore = "bench — TEST_PEARL profile @ 8K heavy (100 hashes + 100 matmul + 100 jackpot)"]
    fn bench_test_pearl_8k_heavy() {
        // 100 hashes × 8 + 100 + 100 = 1000 active rows out of 8192.
        let r = run_heavy_at(MIN_STARK_LEN, 100, 100, 100, &CircuitConfig::TEST_PEARL);
        r.print();
    }

    #[test]
    #[ignore = "bench — TEST_PEARL profile @ 16K heavy (250 hashes + 250 matmul + 250 jackpot)"]
    fn bench_test_pearl_16k_heavy() {
        let r = run_heavy_at(MIN_STARK_LEN * 2, 250, 250, 250, &CircuitConfig::TEST_PEARL);
        r.print();
    }

    // =================================================================
    //  PROD-profile benches (60-bit pure-query FRI soundness at
    //  the Johnson radius — paper IACR ePrint 2025/2055 Theorem 1.5;
    //  2026-05-21 anchored-between policy).
    // =================================================================

    #[test]
    #[ignore = "bench — PROD profile @ 8K baseline"]
    fn bench_prod_8k_baseline() {
        let r = run_baseline_at(MIN_STARK_LEN, &CircuitConfig::PROD);
        r.print();
    }

    #[test]
    #[ignore = "bench — PROD profile @ 16K baseline"]
    fn bench_prod_16k_baseline() {
        let r = run_baseline_at(MIN_STARK_LEN * 2, &CircuitConfig::PROD);
        r.print();
    }

    #[test]
    #[ignore = "bench — PROD profile @ 8K light"]
    fn bench_prod_8k_light() {
        let r = run_light_at(MIN_STARK_LEN, &CircuitConfig::PROD);
        r.print();
    }

    #[test]
    #[ignore = "bench — PROD profile @ 8K heavy"]
    fn bench_prod_8k_heavy() {
        let r = run_heavy_at(MIN_STARK_LEN, 100, 100, 100, &CircuitConfig::PROD);
        r.print();
    }

    // =================================================================
    //  FRI parameter sweep. The historical PROD_LBn profiles remain
    //  at ≥80-bit unconditional Johnson-radius soundness; current PROD
    //  is the 60-bit pure-query anchored-between profile.
    //
    //  The high-margin profiles pair `log_blowup` with `num_queries` so
    //  that `log_blowup · num_queries ≥ 80` (~90 bits each for margin).
    //  The LDE size scales as `2^log_blowup × trace`; the proof
    //  size scales (roughly) as `num_queries × log2(LDE)`. So as
    //  log_blowup goes up:
    //   - prove cost goes UP (bigger LDE to commit)
    //   - proof size goes DOWN (fewer FRI openings)
    //
    //  Goal: identify the operating point that minimizes proof
    //  size at acceptable prove-time cost.
    // =================================================================

    #[test]
    #[ignore = "FRI sweep — log_blowup=2, num_queries=45 @ 8K heavy (≥80-bit Johnson)"]
    fn bench_fri_sweep_lb2_8k_heavy() {
        let r = run_heavy_at(MIN_STARK_LEN, 100, 100, 100, &CircuitConfig::PROD_LB2);
        r.print();
    }

    #[test]
    #[ignore = "FRI sweep — current PROD log_blowup=4, num_queries=15, pow_bits=0 @ 8K heavy (60-bit Johnson)"]
    fn bench_fri_sweep_current_prod_8k_heavy() {
        let r = run_heavy_at(MIN_STARK_LEN, 100, 100, 100, &CircuitConfig::PROD);
        r.print();
    }

    #[test]
    #[ignore = "FRI sweep — log_blowup=4, num_queries=23 @ 8K heavy (≥80-bit Johnson)"]
    fn bench_fri_sweep_lb4_8k_heavy() {
        let r = run_heavy_at(MIN_STARK_LEN, 100, 100, 100, &CircuitConfig::PROD_LB4);
        r.print();
    }

    #[test]
    #[ignore = "FRI sweep — log_blowup=5, num_queries=18 @ 8K heavy (≥80-bit Johnson)"]
    fn bench_fri_sweep_lb5_8k_heavy() {
        let r = run_heavy_at(MIN_STARK_LEN, 100, 100, 100, &CircuitConfig::PROD_LB5);
        r.print();
    }

    #[test]
    #[ignore = "FRI sweep — log_blowup=6, num_queries=15 @ 8K heavy (≥80-bit Johnson)"]
    fn bench_fri_sweep_lb6_8k_heavy() {
        let r = run_heavy_at(MIN_STARK_LEN, 100, 100, 100, &CircuitConfig::PROD_LB6);
        r.print();
    }

    // Baseline variants too — proof-size scaling without activity.

    #[test]
    #[ignore = "FRI sweep — log_blowup=2, num_queries=45 @ 8K baseline (≥80-bit Johnson)"]
    fn bench_fri_sweep_lb2_8k_baseline() {
        let r = run_baseline_at(MIN_STARK_LEN, &CircuitConfig::PROD_LB2);
        r.print();
    }

    #[test]
    #[ignore = "FRI sweep — log_blowup=4, num_queries=23 @ 8K baseline (≥80-bit Johnson)"]
    fn bench_fri_sweep_lb4_8k_baseline() {
        let r = run_baseline_at(MIN_STARK_LEN, &CircuitConfig::PROD_LB4);
        r.print();
    }

    #[test]
    #[ignore = "FRI sweep — log_blowup=5, num_queries=18 @ 8K baseline (≥80-bit Johnson)"]
    fn bench_fri_sweep_lb5_8k_baseline() {
        let r = run_baseline_at(MIN_STARK_LEN, &CircuitConfig::PROD_LB5);
        r.print();
    }

    #[test]
    #[ignore = "FRI sweep — log_blowup=6, num_queries=15 @ 8K baseline (≥80-bit Johnson)"]
    fn bench_fri_sweep_lb6_8k_baseline() {
        let r = run_baseline_at(MIN_STARK_LEN, &CircuitConfig::PROD_LB6);
        r.print();
    }

    // =================================================================
    //  Prover profiling — span-timing attribution of the inner ai-pow
    //  STARK prove. Installs a `tracing-subscriber` fmt layer with
    //  span-CLOSE events at INFO level, so every Plonky3
    //  `info_span!`-instrumented prove phase (commit-to-trace,
    //  compute-quotient, commit-to-quotient, open) prints with its
    //  wall-clock duration. Used to attribute the ~17.7 s PROD @ 8K
    //  prove time (see 2026-05-21_E2E_LATENCY_AND_SWEEP_MEASUREMENTS.md)
    //  to its dominant sub-phase ⇒ informs the inner-PoUW optimisation
    //  path selection.
    // =================================================================

    #[test]
    #[ignore = "profiling — span-timing breakdown of the PROD @ 8K inner prove"]
    fn profile_prod_8k_baseline_prove_spans() {
        use tracing_subscriber::fmt::format::FmtSpan;

        // INFO-level fmt subscriber, emitting a line on each span CLOSE
        // with the span's busy/idle time. `try_init` so a second
        // profiling test in the same process doesn't panic.
        let _ = tracing_subscriber::fmt()
            .with_span_events(FmtSpan::CLOSE)
            .with_max_level(tracing::Level::INFO)
            .with_target(false)
            .try_init();

        eprintln!("\n=== PROFILE: PROD @ 8K baseline inner prove — span timings ===");
        let r = run_baseline_at(MIN_STARK_LEN, &CircuitConfig::PROD);
        r.print();
        eprintln!("=== END PROFILE ===\n");
    }
}
