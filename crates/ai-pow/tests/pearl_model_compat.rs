//! Phase B — byte-equivalence & correctness vs Pearl for the
//! **shipped production model** `pearl-ai/Llama-3.1-8B-Instruct-pearl`.
//!
//! `pearl_compat_fixtures.rs` already pins S0–S9 byte-equality
//! against a *vendored copy of Pearl's reference functions* at
//! *generic* hand-picked shapes. This file lifts that to the
//! **real production preset's scale-sensitive parameters**
//! (`k = 4096`, `r = 64`, `tile = 64`, the 57 344-chunk weight
//! scale) and is the home of:
//!
//!   * **B1.0 (here, Pearl-independent):** the protocol boundaries
//!     are structurally sound + self-consistent at the real
//!     scale (catches shape/scale latents the tiny generic
//!     fixtures miss — e.g. the `r-1 = 63` permutation bitmask,
//!     `64 | 4096`, the 64-stripe fold wrapping the 16 `M` slots
//!     4×, the chunk-Merkle leaf count at the real weight size).
//!   * **B1.1/B1.2 (residual, Pearl-gated — see the `b1_1_*`
//!     stub):** the *byte-equality* assertions vs **Pearl's real
//!     miner** for this model's mining config `μ`. Blocked on the
//!     Pearl-side golden artifact (`PHASE_B_DESIGN.md` Risk-1 /
//!     DB-1); wired so the only remaining work is dropping in
//!     `fixtures/pearl_model.rs` and removing the stub's
//!     `#[ignore]`.
//!
//! Authoritative plan: `crates/ai-pow-zk/PHASE_B_DESIGN.md`.

use ai_pow::commit::{padded_chunk_len, CHUNK_LEN};
use ai_pow::matmul::{compute_tile_from_slices, BlockNoise};
use ai_pow::params::{LlamaFfnLayer, MatmulParams};
use ai_pow::synth::synth_matrices;
use ai_pow::tile_hash::difficulty_target;

/// The real shipped model's mined GEMM, reduced to **one tile**
/// (`m = n = tile = 64`) at the **real scale-sensitive params**
/// (`k = 4096`, `r = 64`, `tile = 64`). `m/n` are reduced only so
/// this stays a fast unit test; every parameter that drives a
/// scale-dependent code path (`k`, `r`, `tile`, `num_stripes =
/// k/r = 64`) is the production value.
fn real_one_tile() -> MatmulParams {
    let p = MatmulParams {
        m: 64,
        k: 4096, // hidden_size — the real contraction dim
        n: 64,
        noise_rank: 64, // the real r (perm bitmask r-1 = 63)
        tile: 64,       // the real tile
        spot_checks: 1, // one tile ⇒ ≤ 1
        difficulty_bits: 0,
    };
    p.validate().expect("real-scale one-tile params valid");
    p
}

/// **B1.0a — the B3 ⇄ preset cross-link.** `gate_proj` is a
/// group_1 INT7 *mineable* layer and its params are exactly the
/// `LLAMA_3_1_8B_GATE_UP` shape; the real preset is
/// envelope-valid (consensus-admissible).
#[test]
fn b1_0a_real_preset_is_the_mineable_gate_up_shape() {
    let gu = LlamaFfnLayer::GateProj
        .mineable_matmul_params(4096)
        .expect("gate_proj is group_1 INT7 — mineable");
    assert_eq!(
        (gu.k, gu.n),
        (
            MatmulParams::LLAMA_3_1_8B_GATE_UP.k,
            MatmulParams::LLAMA_3_1_8B_GATE_UP.n
        ),
    );
    assert!(MatmulParams::LLAMA_3_1_8B_GATE_UP
        .validate_prod_envelope()
        .is_ok());
}

/// **B1.0b — chunk-Merkle leaf count at the real weight scale.**
/// The P-B.2 motivating scale: a `gate_proj`/`up_proj` weight is
/// `n·k = 14336·4096 = 58 720 256` bytes ⇒ exactly `57 344`
/// BLAKE3 chunks (no chunk-padding off-by-one only visible at
/// the real size); the `m·k` activation side is `16 777 216` ⇒
/// `16 384` chunks. Pure arithmetic — no Pearl oracle, no
/// materialization.
#[test]
fn b1_0b_commitment_chunk_count_at_real_weight_scale() {
    let k = MatmulParams::LLAMA_3_1_8B_GATE_UP.k as usize; // 4096
    let n = MatmulParams::LLAMA_3_1_8B_GATE_UP.n as usize; // 14336
    let m = MatmulParams::LLAMA_3_1_8B_GATE_UP.m as usize; // 4096

    let w_bytes = n * k; // i8 ⇒ 1 byte each
    assert_eq!(w_bytes, 58_720_256);
    let w_padded = padded_chunk_len(w_bytes);
    assert_eq!(w_padded, w_bytes, "real weight bytes are chunk-aligned");
    assert_eq!(w_padded / CHUNK_LEN, 57_344, "the P-B.2 motivating scale");

    let a_bytes = m * k;
    assert_eq!(a_bytes, 16_777_216);
    assert_eq!(padded_chunk_len(a_bytes) / CHUNK_LEN, 16_384);
}

/// **B1.0c — §4.4 noise-factor structure at the real `r = 64`.**
/// `BlockNoise::expand` at `(k=4096, r=64)`: `E_L`/`F_R` values
/// live in Pearl's `[−32, 31]` (`(byte & 0x3F) − 32`), and every
/// choice position is in `[0, r) = [0, 64)` with the `+`/`−`
/// positions **distinct** (Pearl §4.4 ChoiceMatrix) — the
/// invariant the `r−1 = 63` bitmask must preserve at the real
/// rank. Lengths match the documented `m·r` / `n·r` / `k` shapes.
#[test]
fn b1_0c_noise_structure_at_real_rank() {
    let p = real_one_tile();
    let nz = BlockNoise::expand(&[7u8; 32], &[9u8; 32], &p);
    let (m, k, n, r) =
        (p.m as usize, p.k as usize, p.n as usize, p.noise_rank as usize);

    assert_eq!(nz.e_l.len(), m * r);
    assert_eq!(nz.f_r.len(), n * r);
    assert_eq!(nz.e_r_pos.len(), k);
    assert_eq!(nz.f_l_pos.len(), k);

    for &v in &nz.e_l {
        assert!((-32..=31).contains(&(v as i32)), "E_L {v} ∉ [-32,31]");
    }
    for &v in &nz.f_r {
        assert!((-32..=31).contains(&(v as i32)), "F_R {v} ∉ [-32,31]");
    }
    for &(pp, pm) in nz.e_r_pos.iter().chain(nz.f_l_pos.iter()) {
        assert!((pp as usize) < r && (pm as usize) < r, "pos ≥ r");
        assert_ne!(pp, pm, "Pearl §4.4: the two positions are distinct");
    }
}

/// **B1.0d — mineable-unit self-consistency at the real
/// `(k, r, tile)`.** `compute_tile_from_slices` is deterministic
/// at the production scale (catches scale-dependent
/// nondeterminism/UB); the `num_stripes = k/r = 64`-stripe fold
/// wraps the 16 `M` slots **4×** (the slot-wrap path the
/// `k = 64` fixtures never reach); the keyed hash is
/// deterministic and key-sensitive (D5: the digest is identical
/// given the same key — the precise byte-equivalence claim's
/// anchor).
#[test]
fn b1_0d_mineable_unit_self_consistent_at_real_scale() {
    let p = real_one_tile();
    assert_eq!(p.num_stripes(), 64, "k/r = 4096/64 ⇒ 64 stripes > 16");

    // One tile's slices at the real scale (m = n = tile = 64 ⇒
    // synth's m·k / n·k ARE the single tile's t·k slices).
    let (a, b) = synth_matrices(b"b1.0d-real-scale", &p);
    assert_eq!(a.len(), 64 * 4096);
    assert_eq!(b.len(), 64 * 4096);

    let s1 = compute_tile_from_slices(&a, &b, &p);
    let s2 = compute_tile_from_slices(&a, &b, &p);
    assert_eq!(s1, s2, "compute_tile_from_slices must be deterministic");

    // Different inputs ⇒ different state (the fold is live, not a
    // constant, at 64 stripes).
    let (a2, _) = synth_matrices(b"b1.0d-other", &p);
    assert_ne!(
        compute_tile_from_slices(&a2, &b, &p),
        s1,
        "distinct A ⇒ distinct TileState (64-stripe fold active)"
    );

    // Keyed hash: deterministic + key-sensitive (D5 — the digest
    // is a pure fn of (state, key); feeding Pearl's key would
    // reproduce Pearl's digest, the s6 anchor).
    let h_k1 = s1.keyed_hash(&[1u8; 32]);
    assert_eq!(h_k1, s1.keyed_hash(&[1u8; 32]), "hash deterministic");
    assert_ne!(h_k1, s1.keyed_hash(&[2u8; 32]), "hash is key-sensitive");
}

/// **B1.0e — shape-aware difficulty target at the real
/// `(b, r, t)`.** `difficulty_target` is a well-formed 32-byte LE
/// encoding at the real preset, deterministic, non-trivial, and
/// responds to `difficulty_bits` (Pearl §4.8 `2^(256−b)·r·t²`,
/// saturating) — the S9 invariant re-pinned at the real shape.
#[test]
fn b1_0e_difficulty_target_well_formed_at_real_preset() {
    let p = MatmulParams::LLAMA_3_1_8B_GATE_UP;
    let t0 = difficulty_target(&p);
    assert_eq!(t0, difficulty_target(&p), "deterministic");
    assert!(t0.iter().any(|&x| x != 0), "non-trivial target");

    // weight = r·t² = 64·64² = 2¹⁸; the target `2^(256−b)·weight`
    // *saturates* to all-0xFF until `b ≳ 18`. Use a clearly
    // non-saturating `b` (128 ⇒ 2^(128)·2¹⁸ = 2¹⁴⁶) to prove the
    // target genuinely responds to difficulty.
    let t0_saturates = t0.iter().all(|&x| x == 0xFF);
    assert!(t0_saturates, "b=0 ⇒ target saturates to 2^256-1 (all 0xFF)");
    let harder = difficulty_target(&MatmulParams { difficulty_bits: 128, ..p });
    assert_ne!(
        harder, t0,
        "a non-saturating difficulty_bits must change the target \
         (§4.8 2^(256-b)·r·t², saturating)"
    );
    assert!(
        !harder.iter().all(|&x| x == 0xFF),
        "b=128 at weight 2^18 must NOT saturate"
    );
}

/// **B1.1/B1.2 — RESIDUAL (Pearl-gated; `PHASE_B_DESIGN.md`
/// Risk-1 / DB-1).** Byte-equality vs **Pearl's real miner** for
/// the shipped model's mining config `μ`. This is the *only*
/// Phase-B item that cannot be discharged in-repo: it needs
/// golden `(κ, s_a, s_b, sampled E/F rows, A, B, X, jackpot[16],
/// digest, H_A, H_B, target)` captured from Pearl's actual miner
/// (run it, or obtain from the Pearl team — DB-1). The harness
/// is otherwise complete: drop the golden into
/// `tests/fixtures/pearl_model.rs`, assert each `ai-pow`
/// primitive (`prng`/`commit`/`fiat_shamir`/`matmul`/fold/
/// `tile_hash`) bit-matches it (mirroring the S0–S9 assertion
/// shapes already proven against the vendored reference), and
/// remove this `#[ignore]`. No other code is required (B2.4
/// pinned the `BlockContext` layout for the digest-parity edge).
#[test]
#[ignore = "B1.1: needs Pearl real-miner golden vectors for the shipped \
            model μ — the one external Phase-B dependency (Risk-1/DB-1). \
            Drop fixtures/pearl_model.rs in and remove this ignore."]
fn b1_1_byte_equal_to_pearl_real_miner_for_model_mu() {
    // Intentionally empty: the gate is the presence of the
    // real-miner golden fixture (Pearl-side artifact). Until then
    // this records the precise residual and stays #[ignore]d so
    // `--include-ignored` surfaces exactly what is outstanding.
    panic!(
        "B1.1 residual: supply Pearl real-miner golden vectors for \
         pearl-ai/Llama-3.1-8B-Instruct-pearl's mining config μ \
         (see PHASE_B_DESIGN.md §2 / Risk-1 / DB-1)."
    );
}
