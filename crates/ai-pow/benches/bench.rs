//! Criterion benches for the AI-PoW prover and verifier.
//!
//! These run at `MatmulParams::TEST_SMALL` by default for fast feedback in
//! CI; pass `BENCH_PROFILE=mid` or `BENCH_PROFILE=prod` in the environment
//! to bench larger profiles.

use std::env;
use std::time::Duration;

use ai_pow::params::MatmulParams;
use ai_pow::prover::{mine, mine_block, ProverOptions};
use ai_pow::verifier::verify;
use criterion::{criterion_group, criterion_main, Criterion};

fn pick_params() -> MatmulParams {
    match env::var("BENCH_PROFILE").as_deref() {
        Ok("prod") => MatmulParams::PROD,
        Ok("gemma4_31b_ffn") => MatmulParams::GEMMA_4_31B_FFN,
        Ok("qwen3_6_27b_ffn") => MatmulParams::QWEN_3_6_27B_FFN,
        Ok("mid") => MatmulParams {
            m: 256,
            k: 256,
            n: 256,
            noise_rank: 16,
            tile: 32,
            spot_checks: 16,
            lambda: 16,
        },
        _ => MatmulParams::TEST_SMALL,
    }
}

fn bench_prover(c: &mut Criterion) {
    let params = pick_params();
    let target = [0xff_u8; 32];
    c.bench_function("prover.mine.one_attempt", |b| {
        b.iter(|| mine(b"hdr", b"nce", &params, &target, ProverOptions::default()).unwrap())
    });
}

fn bench_verifier(c: &mut Criterion) {
    let params = pick_params();
    let target = [0xff_u8; 32];
    let proof = mine(b"hdr", b"nce", &params, &target, ProverOptions::default())
        .unwrap()
        .unwrap();
    c.bench_function("verifier.verify", |b| {
        b.iter(|| verify(b"hdr", b"nce", &params, &target, &proof).unwrap())
    });
}

fn bench_mine_block_amortized(c: &mut Criterion) {
    // Sweep 4 nonces with a target so tight nothing satisfies, so the prover
    // does all 4 full matmuls and we can compare against 4× `mine.one_attempt`.
    // The difference is the block-level noise-cache savings.
    let params = pick_params();
    let target = [0u8; 32];
    c.bench_function("prover.mine_block.4_nonces_no_match", |b| {
        b.iter(|| {
            mine_block(
                b"hdr",
                [b"n1" as &[u8], b"n2", b"n3", b"n4"],
                &params,
                &target,
                ProverOptions::default(),
            )
            .unwrap()
        })
    });
    c.bench_function("prover.mine.one_attempt_no_match", |b| {
        b.iter(|| mine(b"hdr", b"n1", &params, &target, ProverOptions::default()).unwrap())
    });
}

criterion_group! {
    name = benches;
    config = Criterion::default()
        .sample_size(10)
        .measurement_time(Duration::from_secs(5));
    targets = bench_prover, bench_verifier, bench_mine_block_amortized
}
criterion_main!(benches);
