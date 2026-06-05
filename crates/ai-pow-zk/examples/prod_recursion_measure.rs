//! Production caller — drives the ai-pow-zk → Plonky3-recursion
//! pipeline at production parameters and measures it.
//!
//! Run one trace size per process (so peak RSS is clean):
//!
//! ```text
//! RUSTFLAGS="-Ctarget-cpu=native" \
//!   cargo run --release --example prod_recursion_measure \
//!   --features recursion -- <log2_trace_height>
//! ```
//!
//! `ZkParams` is `ai_pow::params::MatmulParams::LLAMA_3_1_8B_GATE_UP`
//! — the real shipped `pearl-ai/Llama-3.1-8B-Instruct-pearl` FFN
//! gate/up GEMM, the production mining target. The Layer-0 FRI profile
//! is `CircuitConfig::PROD` (log_blowup = 4, num_queries = 15); the
//! Layer-1 outer recursive certificate uses `goldilocks_tip5_60bit()`
//! (60-bit Johnson: log_blowup = 4, num_queries = 9, query PoW = 24). At
//! the Pearl-faithful `tile = 8` (`h·w = 64`, Pearl's `default_mining_
//! config`) the production composite trace for that model is
//! **2^15 rows** (the `ai_pow::zk_bridge::expected_layer0_rows`
//! budget — strip Merkle + §6(b) sweep + noised store — rounds up to
//! 2^15). The earlier `tile = 64` over-tiling put it at 2^19.
//!
//! STARK proving is data-oblivious (same FFTs / Merkle tree / FRI
//! folding regardless of cell values), so a zero-activity
//! `CompositeTrace::baseline(n)` at height `n` gives a faithful
//! size + time + memory measurement without the 16 GB model weights:
//! the recursion cost is fixed by trace dimensions and FRI params.

use std::collections::BTreeSet;
use std::io::Write;

use ai_pow_zk::composite_layout::TOTAL_TRACE_WIDTH;
use ai_pow_zk::recursion::{
    encode_recursive_certificate, prove_canonical_ai_pow_certificate, AiPowRecursiveCertificate,
};
use ai_pow_zk::{CircuitConfig, CompositeTrace, ZkParams};
use flate2::write::{GzEncoder, ZlibEncoder};
use flate2::Compression;
use p3_field::PrimeField64;
use p3_goldilocks::Goldilocks;

/// Production composite-trace height for `LLAMA_3_1_8B_GATE_UP` at
/// the Pearl-faithful `tile = 8`.
const PROD_TRACE_LOG2: u32 = 15;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let log2_n: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(13);
    let n = 1usize << log2_n;

    // The production mining target — Llama-3.1-8B FFN gate/up GEMM.
    let zk = ZkParams {
        m: 4096,
        k: 4096,
        n: 14336,
        noise_rank: 64,
        tile: 8,
        difficulty_bits: 0,
    };
    let profile = CircuitConfig::PROD;

    // Inner-composite main-trace LDE memory projection — the
    // dominant single allocation (log_blowup = 4 ⇒ 16× blowup).
    let lde_rows = (n as u128) << 4;
    let main_lde_bytes = lde_rows * (TOTAL_TRACE_WIDTH as u128) * 8;
    let gb = |b: u128| b as f64 / (1u64 << 30) as f64;

    eprintln!("══ ai-pow-zk → Plonky3-recursion · production caller ══");
    eprintln!("target model : Llama-3.1-8B-Instruct-pearl — FFN gate/up GEMM");
    eprintln!("ZkParams     : m=4096 k=4096 n=14336 r=64 tile=8");
    eprintln!("L0 FRI profile : CircuitConfig::PROD (log_blowup=4, num_queries=15, pow_bits=0)");
    eprintln!(
        "L1 FRI profile : goldilocks_tip5_60bit (60-bit Johnson, log_blowup=4, num_queries=9, query_pow_bits=24, cap_height=5)"
    );
    eprintln!("trace width  : {TOTAL_TRACE_WIDTH} columns");
    eprintln!(
        "trace height : 2^{log2_n} = {n} rows   (production target = 2^{PROD_TRACE_LOG2} = {})",
        1usize << PROD_TRACE_LOG2
    );
    eprintln!(
        "projected inner main-trace LDE (16×): {:.2} GB  \
         — peak RSS is higher (permutation-trace LDE + quotient + Merkle trees)",
        gb(main_lde_bytes)
    );
    eprintln!();

    let trace = CompositeTrace::baseline(n);

    eprintln!("running… (L0 composite prove → L1 verifier circuit → canonical L1 cert)");
    let run = match prove_canonical_ai_pow_certificate(&zk, &profile, trace) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("FAILED at 2^{log2_n}: {e:?}");
            std::process::exit(1);
        }
    };
    let l0_proof = run.l1_cert.l0_proof();
    let l1_outer = run.l1_cert.l1_outer_proof();

    let composite_bytes = postcard::to_allocvec(l0_proof)
        .expect("serialize composite proof")
        .len();
    let composite_commitments_bytes = postcard::to_allocvec(&l0_proof.commitments)
        .expect("serialize composite commitments")
        .len();
    let composite_opened_values_bytes = postcard::to_allocvec(&l0_proof.opened_values)
        .expect("serialize composite opened values")
        .len();
    let composite_opening_proof_bytes = postcard::to_allocvec(&l0_proof.opening_proof)
        .expect("serialize composite opening proof")
        .len();
    let composite_lookup_data_bytes = postcard::to_allocvec(&l0_proof.global_lookup_data)
        .expect("serialize composite lookup data")
        .len();
    let l1_cert_serialized =
        encode_recursive_certificate(&run.l1_cert).expect("serialize canonical L1 certificate");
    let l1_cert_bytes = l1_cert_serialized.len();
    let l1_cert_postcard_bytes = postcard::to_allocvec(&run.l1_cert)
        .expect("serialize L1 certificate with postcard")
        .len();
    let l1_cert_zlib_fast_bytes =
        compressed_len_zlib(&l1_cert_serialized, Compression::fast()).expect("zlib fast");
    let l1_cert_zlib_best_bytes =
        compressed_len_zlib(&l1_cert_serialized, Compression::best()).expect("zlib best");
    let l1_cert_gzip_best_bytes =
        compressed_len_gzip(&l1_cert_serialized, Compression::best()).expect("gzip best");
    let l1_cert_bincode_varint_bytes =
        bincode_recursive_certificate_len(&run.l1_cert, bincode::config::standard())
            .expect("bincode varint");
    let l1_cert_bincode_fixed_bytes = bincode_recursive_certificate_len(
        &run.l1_cert,
        bincode::config::standard().with_fixed_int_encoding(),
    )
    .expect("bincode fixed");
    let l1_proof_bytes = postcard::to_allocvec(&l1_outer.proof)
        .expect("serialize L1 proof")
        .len();
    let l1_commitments_bytes = postcard::to_allocvec(&l1_outer.proof.commitments)
        .expect("serialize L1 commitments")
        .len();
    let l1_opened_values_bytes = postcard::to_allocvec(&l1_outer.proof.opened_values)
        .expect("serialize L1 opened values")
        .len();
    let l1_opening_proof_bytes = postcard::to_allocvec(&l1_outer.proof.opening_proof)
        .expect("serialize L1 opening proof")
        .len();
    let l1_lookup_data_bytes = postcard::to_allocvec(&l1_outer.proof.global_lookup_data)
        .expect("serialize L1 lookup data")
        .len();
    let l1_degree_bits_bytes = postcard::to_allocvec(&l1_outer.proof.degree_bits)
        .expect("serialize L1 degree bits")
        .len();
    let l1_table_packing_bytes = postcard::to_allocvec(&l1_outer.table_packing)
        .expect("serialize L1 table packing")
        .len();
    let l1_rows_bytes = postcard::to_allocvec(&l1_outer.rows)
        .expect("serialize L1 row counts")
        .len();
    let l1_non_primitives_bytes = postcard::to_allocvec(&l1_outer.non_primitives)
        .expect("serialize L1 non-primitive metadata")
        .len();
    let l1_structural_bytes = l1_cert_bytes.saturating_sub(
        l1_proof_bytes + l1_table_packing_bytes + l1_rows_bytes + l1_non_primitives_bytes,
    );
    let l1_postcard_structural_bytes = l1_cert_postcard_bytes.saturating_sub(
        l1_proof_bytes + l1_table_packing_bytes + l1_rows_bytes + l1_non_primitives_bytes,
    );
    let l1_digest_stats = merkle_digest_dictionary_stats(&run.l1_cert);
    let l1_path_compression_estimate = merkle_path_compression_estimate(&run.l1_cert);

    let kb = |b: usize| b as f64 / 1024.0;
    let s = |ms: u128| ms as f64 / 1000.0;
    let total = run.composite_prove_ms
        + run.l1_circuit_build_ms
        + run.l1_in_circuit_verify_ms
        + run.l1_outer_cert_ms;

    eprintln!();
    eprintln!(
        "── results · composite trace 2^{log2_n} ({n} rows × {} cols) ──",
        run.composite_trace_width
    );
    eprintln!(
        "L0  composite prove        : {:>9.2} s",
        s(run.composite_prove_ms)
    );
    eprintln!(
        "L1  verifier-circuit build : {:>9.2} s",
        s(run.l1_circuit_build_ms)
    );
    eprintln!(
        "L1  in-circuit verify (S3) : {:>9.2} s",
        s(run.l1_in_circuit_verify_ms)
    );
    eprintln!(
        "L1  outer cert    (S5)     : {:>9.2} s",
        s(run.l1_outer_cert_ms)
    );
    eprintln!("    TOTAL wall-clock       : {:>9.2} s", s(total));
    eprintln!();
    eprintln!(
        "composite (L0) proof       : {:>9.1} KB",
        kb(composite_bytes)
    );
    eprintln!(
        "  commitments              : {:>9.1} KB",
        kb(composite_commitments_bytes)
    );
    eprintln!(
        "  opened values            : {:>9.1} KB",
        kb(composite_opened_values_bytes)
    );
    eprintln!(
        "  opening proof            : {:>9.1} KB",
        kb(composite_opening_proof_bytes)
    );
    eprintln!(
        "  global lookup data       : {:>9.1} KB",
        kb(composite_lookup_data_bytes)
    );
    if let Some(inst) = l0_proof.opened_values.instances.first() {
        let base = &inst.base_opened_values;
        let trace_next_len = base.trace_next.as_ref().map_or(0, Vec::len);
        let prep_local_len = base.preprocessed_local.as_ref().map_or(0, Vec::len);
        let prep_next_len = base.preprocessed_next.as_ref().map_or(0, Vec::len);
        let random_len = base.random.as_ref().map_or(0, Vec::len);
        let quotient_cols: usize = base.quotient_chunks.iter().map(Vec::len).sum();
        eprintln!(
            "  opened lens              : trace_local={} trace_next={} prep_local={} prep_next={} quotient_chunks={} quotient_values={} random={} perm_local={} perm_next={}",
            base.trace_local.len(),
            trace_next_len,
            prep_local_len,
            prep_next_len,
            base.quotient_chunks.len(),
            quotient_cols,
            random_len,
            inst.permutation_local.len(),
            inst.permutation_next.len(),
        );
    }
    eprintln!("L1 recursive certificate   : {:>9.1} KB", kb(l1_cert_bytes));
    eprintln!(
        "  legacy postcard          : {:>9.1} KB",
        kb(l1_cert_postcard_bytes)
    );
    eprintln!(
        "  zlib fast envelope       : {:>9.1} KB",
        kb(l1_cert_zlib_fast_bytes)
    );
    eprintln!(
        "  zlib best envelope       : {:>9.1} KB",
        kb(l1_cert_zlib_best_bytes)
    );
    eprintln!(
        "  gzip best envelope       : {:>9.1} KB",
        kb(l1_cert_gzip_best_bytes)
    );
    eprintln!(
        "  bincode varint           : {:>9.1} KB",
        kb(l1_cert_bincode_varint_bytes)
    );
    eprintln!(
        "  bincode fixed-int        : {:>9.1} KB",
        kb(l1_cert_bincode_fixed_bytes)
    );
    eprintln!("  table packing            : {:?}", l1_outer.table_packing);
    eprintln!("  rows                     : {:?}", l1_outer.rows);
    for entry in &l1_outer.non_primitives {
        eprintln!(
            "  NPO {op_type} rows={rows} lanes={lanes} public_values={public_values}",
            op_type = entry.op_type,
            rows = entry.rows,
            lanes = entry.lanes,
            public_values = entry.public_values.len(),
        );
    }
    eprintln!(
        "  proof                    : {:>9.1} KB",
        kb(l1_proof_bytes)
    );
    eprintln!(
        "    commitments            : {:>9.1} KB",
        kb(l1_commitments_bytes)
    );
    eprintln!(
        "    opened values          : {:>9.1} KB",
        kb(l1_opened_values_bytes)
    );
    eprintln!(
        "    opening proof          : {:>9.1} KB",
        kb(l1_opening_proof_bytes)
    );
    eprintln!(
        "      auth digests         : {:>9} total / {:>9} unique",
        l1_digest_stats.total_digests, l1_digest_stats.unique_digests
    );
    eprintln!(
        "      auth digest bytes    : {:>9.1} KB raw / {:>9.1} KB dictionary est",
        kb(l1_digest_stats.raw_digest_bytes),
        kb(l1_digest_stats.dictionary_digest_bytes)
    );
    eprintln!(
        "      terminal path model  : {:>9} trees / {:>9} raw siblings",
        l1_path_compression_estimate.groups, l1_path_compression_estimate.raw_siblings
    );
    eprintln!(
        "        compressed siblings: {:>9.1} mean / {:>9} best / {:>9} worst",
        l1_path_compression_estimate.mean_compressed_siblings,
        l1_path_compression_estimate.best_compressed_siblings,
        l1_path_compression_estimate.worst_compressed_siblings,
    );
    eprintln!(
        "        digest savings     : {:>9.1} KB mean / {:>9.1} KB best / {:>9.1} KB worst",
        kb(l1_path_compression_estimate
            .mean_digest_savings_bytes
            .round() as usize),
        kb(l1_path_compression_estimate.best_digest_savings_bytes),
        kb(l1_path_compression_estimate.worst_digest_savings_bytes),
    );
    eprintln!(
        "        fixed-cert floor   : {:>9.1} KB mean / {:>9.1} KB best / {:>9.1} KB worst",
        kb(l1_cert_bytes.saturating_sub(
            l1_path_compression_estimate
                .mean_digest_savings_bytes
                .round() as usize
        )),
        kb(l1_cert_bytes.saturating_sub(l1_path_compression_estimate.best_digest_savings_bytes)),
        kb(l1_cert_bytes.saturating_sub(l1_path_compression_estimate.worst_digest_savings_bytes)),
    );
    if !l1_path_compression_estimate.top_group_summaries.is_empty() {
        eprintln!("        largest groups:");
        for group in &l1_path_compression_estimate.top_group_summaries {
            eprintln!(
                "          {label:<18} path_len={path_len:<2} raw={raw:<4} mean_compressed={mean:.1}",
                label = group.label,
                path_len = group.path_len,
                raw = group.raw_siblings,
                mean = group.mean_compressed_siblings,
            );
        }
    }
    eprintln!(
        "        note               : model assumes verifier-rederived Fiat-Shamir indices; wire-trusted indices would be unsound"
    );
    eprintln!(
        "    global lookup data     : {:>9.1} KB",
        kb(l1_lookup_data_bytes)
    );
    eprintln!(
        "    degree bits            : {:>9.1} KB",
        kb(l1_degree_bits_bytes)
    );
    eprintln!(
        "  table packing bytes      : {:>9.1} KB",
        kb(l1_table_packing_bytes)
    );
    eprintln!("  row-count bytes          : {:>9.1} KB", kb(l1_rows_bytes));
    eprintln!(
        "  non-primitive metadata   : {:>9.1} KB",
        kb(l1_non_primitives_bytes)
    );
    eprintln!(
        "  postcard structural rem. : {:>9.1} KB",
        kb(l1_postcard_structural_bytes)
    );
    eprintln!(
        "  canonical/parts delta    : {:>9.1} KB",
        kb(l1_structural_bytes)
    );
    for (i, inst) in l1_outer.proof.opened_values.instances.iter().enumerate() {
        let base = &inst.base_opened_values;
        let table = match i {
            0 => "const",
            1 => "public",
            2 => "alu",
            _ => run
                .l1_cert
                .l1_outer_proof()
                .non_primitives
                .get(i.saturating_sub(3))
                .map(|entry| entry.op_type.as_str())
                .unwrap_or("unknown"),
        };
        let trace_next_len = base.trace_next.as_ref().map_or(0, Vec::len);
        let prep_local_len = base.preprocessed_local.as_ref().map_or(0, Vec::len);
        let prep_next_len = base.preprocessed_next.as_ref().map_or(0, Vec::len);
        let random_len = base.random.as_ref().map_or(0, Vec::len);
        let quotient_cols: usize = base.quotient_chunks.iter().map(Vec::len).sum();
        eprintln!(
            "    opened[{i}] {table:<28} trace_local={} trace_next={} prep_local={} prep_next={} quotient_values={} random={} perm_local={} perm_next={}",
            base.trace_local.len(),
            trace_next_len,
            prep_local_len,
            prep_next_len,
            quotient_cols,
            random_len,
            inst.permutation_local.len(),
            inst.permutation_next.len(),
        );
    }
    // Machine-greppable row:
    // log2_n,n,width,prove_ms,build_ms,verify_ms,cert_ms,L0_bytes,L1_bytes,postcard_bytes,zlib_fast,zlib_best,gzip_best,bincode_varint,bincode_fixed,path_groups,path_raw_siblings,path_mean_compressed_siblings,path_best_compressed_siblings,path_worst_compressed_siblings,path_mean_digest_savings_bytes
    println!(
        "CSV,{log2_n},{n},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{:.0}",
        run.composite_trace_width,
        run.composite_prove_ms,
        run.l1_circuit_build_ms,
        run.l1_in_circuit_verify_ms,
        run.l1_outer_cert_ms,
        composite_bytes,
        l1_cert_bytes,
        l1_cert_postcard_bytes,
        l1_cert_zlib_fast_bytes,
        l1_cert_zlib_best_bytes,
        l1_cert_gzip_best_bytes,
        l1_cert_bincode_varint_bytes,
        l1_cert_bincode_fixed_bytes,
        l1_path_compression_estimate.groups,
        l1_path_compression_estimate.raw_siblings,
        l1_path_compression_estimate
            .mean_compressed_siblings
            .round() as usize,
        l1_path_compression_estimate.best_compressed_siblings,
        l1_path_compression_estimate.worst_compressed_siblings,
        l1_path_compression_estimate.mean_digest_savings_bytes,
    );
}

#[derive(Clone, Copy, Debug)]
struct MerkleDigestDictionaryStats {
    total_digests: usize,
    unique_digests: usize,
    raw_digest_bytes: usize,
    dictionary_digest_bytes: usize,
}

fn merkle_digest_dictionary_stats(cert: &AiPowRecursiveCertificate) -> MerkleDigestDictionaryStats {
    let mut unique = BTreeSet::<[u64; 5]>::new();
    let mut total = 0usize;
    let proof = &cert.l1_outer_proof().proof;

    for query in &proof.opening_proof.query_proofs {
        for batch in &query.input_proof {
            record_digests(&batch.opening_proof, &mut unique, &mut total);
        }
        for step in &query.commit_phase_openings {
            record_digests(&step.opening_proof, &mut unique, &mut total);
        }
    }

    let index_bytes = if unique.len() <= u16::MAX as usize {
        2
    } else {
        4
    };
    let digest_bytes = core::mem::size_of::<[u64; 5]>();
    MerkleDigestDictionaryStats {
        total_digests: total,
        unique_digests: unique.len(),
        raw_digest_bytes: total * digest_bytes,
        dictionary_digest_bytes: unique.len() * digest_bytes + total * index_bytes,
    }
}

fn record_digests(proof: &[[Goldilocks; 5]], unique: &mut BTreeSet<[u64; 5]>, total: &mut usize) {
    for digest in proof {
        *total += 1;
        unique.insert(digest.map(|word| word.as_canonical_u64()));
    }
}

#[derive(Clone, Debug)]
struct AuthPathGroup {
    label: String,
    path_len: usize,
    index_shift: usize,
    index_bits: usize,
}

#[derive(Clone, Debug)]
struct AuthPathGroupSummary {
    label: String,
    path_len: usize,
    raw_siblings: usize,
    mean_compressed_siblings: f64,
}

#[derive(Clone, Debug)]
struct MerklePathCompressionEstimate {
    groups: usize,
    raw_siblings: usize,
    mean_compressed_siblings: f64,
    best_compressed_siblings: usize,
    worst_compressed_siblings: usize,
    mean_digest_savings_bytes: f64,
    best_digest_savings_bytes: usize,
    worst_digest_savings_bytes: usize,
    top_group_summaries: Vec<AuthPathGroupSummary>,
}

/// Estimate Plonky2-style Merkle path compression for the recursive
/// certificate's FRI opening proof shape.
///
/// This intentionally does not mutate the canonical proof. It models the
/// terminal-compressor opportunity by grouping openings that hit the same
/// Merkle tree and removing sibling digests that can be reconstructed from
/// already-known leaves/siblings in the group. A production terminal
/// compressor must rederive the query indices from the Fiat-Shamir transcript;
/// this estimator samples transcript-shaped random indices because the current
/// benchmark only has the finalized proof object, not the verifier's rebuilt
/// table AIRs needed to replay the whole batch-STARK transcript in place.
fn merkle_path_compression_estimate(
    cert: &AiPowRecursiveCertificate,
) -> MerklePathCompressionEstimate {
    const TRIALS: usize = 256;
    const DIGEST_BYTES: usize = core::mem::size_of::<[u64; 5]>();
    const CAP_HEIGHT: usize = p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_CAP_HEIGHT;
    let proof = &cert.l1_outer_proof().proof;

    let groups = auth_path_groups(cert, CAP_HEIGHT);
    let raw_siblings: usize =
        groups.iter().map(|g| g.path_len).sum::<usize>() * proof.opening_proof.query_proofs.len();

    if groups.is_empty() || proof.opening_proof.query_proofs.is_empty() {
        return MerklePathCompressionEstimate {
            groups: groups.len(),
            raw_siblings,
            mean_compressed_siblings: raw_siblings as f64,
            best_compressed_siblings: raw_siblings,
            worst_compressed_siblings: raw_siblings,
            mean_digest_savings_bytes: 0.0,
            best_digest_savings_bytes: 0,
            worst_digest_savings_bytes: 0,
            top_group_summaries: Vec::new(),
        };
    }

    let num_queries = proof.opening_proof.query_proofs.len();
    let log_global_max_height = groups
        .iter()
        .map(|g| g.index_shift + g.index_bits)
        .max()
        .unwrap_or(0);

    let mut totals = Vec::with_capacity(TRIALS);
    let mut group_totals = vec![0usize; groups.len()];

    for trial in 0..TRIALS {
        let global_indices: Vec<usize> = (0..num_queries)
            .map(|query| sample_index(trial, query, log_global_max_height))
            .collect();
        let mut total = 0usize;
        for (group_idx, group) in groups.iter().enumerate() {
            let indices: Vec<usize> = global_indices
                .iter()
                .map(|&index| reduced_index(index, group.index_shift, group.index_bits))
                .collect();
            let compressed = compressed_sibling_count(CAP_HEIGHT, group.path_len, &indices);
            total += compressed;
            group_totals[group_idx] += compressed;
        }
        totals.push(total);
    }

    let best_compressed_siblings = totals.iter().copied().min().unwrap_or(raw_siblings);
    let worst_compressed_siblings = totals.iter().copied().max().unwrap_or(raw_siblings);
    let mean_compressed_siblings = totals.iter().sum::<usize>() as f64 / TRIALS as f64;
    let mean_digest_savings_bytes =
        (raw_siblings as f64 - mean_compressed_siblings) * DIGEST_BYTES as f64;
    let best_digest_savings_bytes =
        raw_siblings.saturating_sub(best_compressed_siblings) * DIGEST_BYTES;
    let worst_digest_savings_bytes =
        raw_siblings.saturating_sub(worst_compressed_siblings) * DIGEST_BYTES;

    let mut top_group_summaries: Vec<AuthPathGroupSummary> = groups
        .iter()
        .zip(group_totals)
        .map(|(group, total)| {
            let raw = group.path_len * num_queries;
            AuthPathGroupSummary {
                label: group.label.clone(),
                path_len: group.path_len,
                raw_siblings: raw,
                mean_compressed_siblings: total as f64 / TRIALS as f64,
            }
        })
        .collect();
    top_group_summaries.sort_by(|a, b| {
        let a_savings = a.raw_siblings as f64 - a.mean_compressed_siblings;
        let b_savings = b.raw_siblings as f64 - b.mean_compressed_siblings;
        b_savings
            .partial_cmp(&a_savings)
            .unwrap_or(core::cmp::Ordering::Equal)
    });
    top_group_summaries.truncate(5);

    MerklePathCompressionEstimate {
        groups: groups.len(),
        raw_siblings,
        mean_compressed_siblings,
        best_compressed_siblings,
        worst_compressed_siblings,
        mean_digest_savings_bytes,
        best_digest_savings_bytes,
        worst_digest_savings_bytes,
        top_group_summaries,
    }
}

fn auth_path_groups(cert: &AiPowRecursiveCertificate, cap_height: usize) -> Vec<AuthPathGroup> {
    let proof = &cert.l1_outer_proof().proof;
    let Some(first_query) = proof.opening_proof.query_proofs.first() else {
        return Vec::new();
    };

    let mut groups = Vec::new();
    let log_arities: Vec<usize> = first_query
        .commit_phase_openings
        .iter()
        .map(|step| step.log_arity as usize)
        .collect();
    let log_global_max_height = log_arities.iter().sum::<usize>()
        + p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_LOG_BLOWUP
        + p3_circuit_prover::config::GOLDILOCKS_TIP5_RECURSIVE_LOG_FINAL_POLY_LEN;

    for (batch_idx, batch) in first_query.input_proof.iter().enumerate() {
        let path_len = batch.opening_proof.len();
        let index_bits = cap_height + path_len;
        groups.push(AuthPathGroup {
            label: format!("input[{batch_idx}]"),
            path_len,
            index_shift: log_global_max_height.saturating_sub(index_bits),
            index_bits,
        });
    }

    let mut folded_shift = 0usize;
    for (step_idx, step) in first_query.commit_phase_openings.iter().enumerate() {
        folded_shift += step.log_arity as usize;
        let path_len = step.opening_proof.len();
        let index_bits = cap_height + path_len;
        groups.push(AuthPathGroup {
            label: format!("fri-step[{step_idx}]"),
            path_len,
            index_shift: folded_shift,
            index_bits,
        });
    }

    groups
}

fn compressed_sibling_count(cap_height: usize, path_len: usize, indices: &[usize]) -> usize {
    if path_len == 0 || indices.is_empty() {
        return 0;
    }
    let height = cap_height + path_len;
    if height >= usize::BITS as usize {
        return path_len * indices.len();
    }

    let num_leaves = 1usize << height;
    let mut known = BTreeSet::new();
    for &leaf in indices {
        let mut node = (leaf % num_leaves) + num_leaves;
        for _ in 0..path_len {
            known.insert(node);
            node >>= 1;
        }
    }

    let mut compressed = 0usize;
    for &leaf in indices {
        let mut node = (leaf % num_leaves) + num_leaves;
        for _ in 0..path_len {
            let sibling = node ^ 1;
            if known.insert(sibling) {
                compressed += 1;
            }
            node >>= 1;
            known.insert(node);
        }
    }
    compressed
}

fn reduced_index(index: usize, shift: usize, bits: usize) -> usize {
    if bits == 0 {
        return 0;
    }
    let shifted = index.checked_shr(shift as u32).unwrap_or(0);
    shifted & low_bits_mask(bits)
}

fn sample_index(trial: usize, query: usize, bits: usize) -> usize {
    if bits == 0 {
        return 0;
    }
    let seed = 0x9e37_79b9_7f4a_7c15u64
        ^ ((trial as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9))
        ^ ((query as u64).wrapping_mul(0x94d0_49bb_1331_11eb));
    (splitmix64(seed) as usize) & low_bits_mask(bits)
}

fn low_bits_mask(bits: usize) -> usize {
    if bits >= usize::BITS as usize {
        usize::MAX
    } else {
        (1usize << bits) - 1
    }
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn compressed_len_zlib(bytes: &[u8], level: Compression) -> Result<usize, std::io::Error> {
    let mut encoder = ZlibEncoder::new(Vec::new(), level);
    encoder.write_all(bytes)?;
    encoder.finish().map(|bytes| bytes.len())
}

fn compressed_len_gzip(bytes: &[u8], level: Compression) -> Result<usize, std::io::Error> {
    let mut encoder = GzEncoder::new(Vec::new(), level);
    encoder.write_all(bytes)?;
    encoder.finish().map(|bytes| bytes.len())
}

fn bincode_recursive_certificate_len<C>(
    cert: &AiPowRecursiveCertificate,
    config: C,
) -> Result<usize, bincode::error::EncodeError>
where
    C: bincode::config::Config,
{
    bincode::serde::encode_to_vec(cert, config).map(|bytes| bytes.len())
}
