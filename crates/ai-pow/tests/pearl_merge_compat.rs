use ai_pow::commit::matrix_commitment;
use ai_pow::fiat_shamir::{noise_seed_a, noise_seed_b};
use ai_pow::params::MatmulParams;
use ai_pow::pearl_compat::{
    compute_pearl_pattern_ticket, pearl_adjust_target_for_config, pearl_kappa,
    pearl_nbits_to_target_le, verify_pearl_compatible_work, verify_pearl_pattern_ticket,
    PearlAttempt, PearlCompatError, PearlIncompleteBlockHeader, PearlMiningConfig,
    PearlPeriodicPattern, PearlPublicProofParams, PEARL_INCOMPLETE_BLOCK_HEADER_SIZE,
    PEARL_MINING_CONFIG_RESERVED_SIZE, PEARL_MINING_CONFIG_SIZE, PEARL_MMA_INT7XINT7_TO_INT32,
    PEARL_PUBLIC_PROOF_PARAMS_SIZE,
};
use ai_pow::prover::{params_tag, BlockContext};
use ai_pow::synth::synth_matrices;

fn simple_pattern(length: u32) -> PearlPeriodicPattern {
    PearlPeriodicPattern {
        shape: [(1, length), (length, 1), (length, 1)],
    }
}

fn header() -> PearlIncompleteBlockHeader {
    PearlIncompleteBlockHeader {
        version: 0x0102_0304,
        prev_block: [0x11; 32],
        merkle_root: [0x22; 32],
        timestamp: 0x6677_8899,
        nbits: 0x1d00_ffff,
    }
}

fn mining_config() -> PearlMiningConfig {
    PearlMiningConfig {
        common_dim: 64,
        rank: 4,
        mma_type: PEARL_MMA_INT7XINT7_TO_INT32,
        rows_pattern: simple_pattern(8),
        cols_pattern: simple_pattern(8),
        reserved: [0u8; PEARL_MINING_CONFIG_RESERVED_SIZE],
    }
}

fn pearl_default_cols_pattern() -> Vec<u32> {
    (0..32).flat_map(|i| [8 * i, 8 * i + 1]).collect()
}

fn pearl_compat_test_pattern() -> PearlPeriodicPattern {
    PearlPeriodicPattern::from_list(&[0, 1, 8, 9, 64, 65, 72, 73]).unwrap()
}

fn pearl_square_params() -> MatmulParams {
    MatmulParams {
        m: 128,
        k: 1024,
        n: 128,
        noise_rank: 64,
        tile: 8,
        spot_checks: 1,
        difficulty_bits: 0,
    }
}

fn pearl_square_config() -> PearlMiningConfig {
    PearlMiningConfig {
        common_dim: 1024,
        rank: 64,
        mma_type: PEARL_MMA_INT7XINT7_TO_INT32,
        rows_pattern: simple_pattern(8),
        cols_pattern: simple_pattern(8),
        reserved: [0u8; PEARL_MINING_CONFIG_RESERVED_SIZE],
    }
}

#[test]
fn pearl_header_and_mining_config_serialization_are_exact() {
    let header = header();
    let encoded_header = header.to_bytes();
    assert_eq!(encoded_header.len(), PEARL_INCOMPLETE_BLOCK_HEADER_SIZE);
    assert_eq!(&encoded_header[0..4], &0x0102_0304u32.to_le_bytes());
    assert_eq!(&encoded_header[4..36], &[0x11; 32]);
    assert_eq!(&encoded_header[36..68], &[0x22; 32]);
    assert_eq!(&encoded_header[68..72], &0x6677_8899u32.to_le_bytes());
    assert_eq!(&encoded_header[72..76], &0x1d00_ffffu32.to_le_bytes());
    assert_eq!(
        PearlIncompleteBlockHeader::from_bytes(&encoded_header).unwrap(),
        header
    );

    let config = mining_config();
    let encoded_config = config.to_bytes().unwrap();
    assert_eq!(encoded_config.len(), PEARL_MINING_CONFIG_SIZE);
    assert_eq!(&encoded_config[0..4], &64u32.to_le_bytes());
    assert_eq!(&encoded_config[4..6], &4u16.to_le_bytes());
    assert_eq!(
        &encoded_config[6..8],
        &PEARL_MMA_INT7XINT7_TO_INT32.to_le_bytes()
    );
    assert_eq!(&encoded_config[8..14], &[0, 7, 0, 0, 0, 0]);
    assert_eq!(&encoded_config[14..20], &[0, 7, 0, 0, 0, 0]);
    assert_eq!(&encoded_config[20..52], &[0u8; 32]);
    assert_eq!(
        PearlMiningConfig::from_bytes(&encoded_config).unwrap(),
        config
    );
}

#[test]
fn pearl_periodic_pattern_matches_reference_semantics() {
    let rows = PearlPeriodicPattern::from_list(&[0, 8]).unwrap();
    assert_eq!(rows.shape, [(8, 2), (16, 1), (16, 1)]);
    assert_eq!(rows.to_bytes().unwrap(), [7, 1, 0, 0, 0, 0]);
    assert_eq!(
        PearlPeriodicPattern::from_bytes(&[7, 1, 0, 0, 0, 0]).unwrap(),
        rows
    );
    assert_eq!(rows.to_list().unwrap(), vec![0, 8]);
    assert_eq!(rows.indices_with_offset_bounded(5, 8).unwrap(), vec![5, 13]);
    assert_eq!(rows.size().unwrap(), 2);
    assert_eq!(rows.period().unwrap(), 16);
    assert_eq!(rows.max().unwrap(), 8);
    assert!(rows.offset_is_valid(0));
    assert!(rows.offset_is_valid(7));
    assert!(!rows.offset_is_valid(8));

    let cols_list = pearl_default_cols_pattern();
    let cols = PearlPeriodicPattern::from_list(&cols_list).unwrap();
    assert_eq!(cols.shape, [(1, 2), (8, 32), (256, 1)]);
    assert_eq!(cols.to_bytes().unwrap(), [0, 1, 3, 31, 0, 0]);
    assert_eq!(cols.to_list().unwrap(), cols_list);
    assert_eq!(cols.size().unwrap(), 64);
    assert_eq!(cols.period().unwrap(), 256);
    assert_eq!(cols.max().unwrap(), 249);
    assert!(cols.offset_is_valid(0));
    assert!(cols.offset_is_valid(2));
    assert!(!cols.offset_is_valid(1));
    assert!(!cols.offset_is_valid(8));
}

#[test]
fn pearl_periodic_pattern_rejects_noncanonical_or_unbounded_shapes() {
    assert_eq!(
        PearlPeriodicPattern::from_list(&[]),
        Err(PearlCompatError::PatternEmpty)
    );
    assert_eq!(
        PearlPeriodicPattern::from_list(&[1, 2]),
        Err(PearlCompatError::PatternMustStartAtZero)
    );
    assert_eq!(
        PearlPeriodicPattern::from_list(&[0, 2, 2]),
        Err(PearlCompatError::PatternNotStrictlyIncreasing)
    );
    assert_eq!(
        PearlPeriodicPattern::from_list(&[0, 1, 3]),
        Err(PearlCompatError::PatternNotRepresentable)
    );
    assert_eq!(
        PearlPeriodicPattern::from_bytes(&[0, 1, 0, 1, 0, 0]),
        Err(PearlCompatError::BrokenSingleStride)
    );
    assert_eq!(
        PearlPeriodicPattern {
            shape: [(0, 1), (1, 1), (1, 1)]
        }
        .to_bytes(),
        Err(PearlCompatError::BadPatternStride)
    );
    assert_eq!(
        PearlPeriodicPattern {
            shape: [(1, 257), (257, 1), (257, 1)]
        }
        .to_bytes(),
        Err(PearlCompatError::PatternByteOverflow)
    );
    assert_eq!(
        PearlPeriodicPattern {
            shape: [(1, 2), (2, 2), (4, 2)]
        }
        .to_list_bounded(7),
        Err(PearlCompatError::PatternListTooLarge)
    );
}

#[test]
fn pearl_public_proof_params_round_trip_and_sanity_match_reference_shape() {
    let pattern = pearl_compat_test_pattern();
    let config = PearlMiningConfig {
        common_dim: 1216,
        rank: 64,
        mma_type: PEARL_MMA_INT7XINT7_TO_INT32,
        rows_pattern: pattern,
        cols_pattern: pattern,
        reserved: [0u8; PEARL_MINING_CONFIG_RESERVED_SIZE],
    };
    let params = PearlPublicProofParams {
        block_header: header(),
        mining_config: config,
        hash_a: [0xA1; 32],
        hash_b: [0xB2; 32],
        hash_jackpot: [0xC3; 32],
        m: 6144,
        n: 4096,
        t_rows: 0,
        t_cols: 0,
    };

    params.sanity_check().unwrap();
    let public_data = params.to_public_data().unwrap();
    assert_eq!(public_data.len(), PEARL_PUBLIC_PROOF_PARAMS_SIZE);
    let restored = PearlPublicProofParams::from_public_data(header(), &public_data).unwrap();
    assert_eq!(restored, params);
    assert_eq!(
        restored.a_rows_indices_bounded(16).unwrap(),
        vec![0, 1, 8, 9, 64, 65, 72, 73]
    );
    assert_eq!(
        restored.b_cols_indices_bounded(16).unwrap(),
        vec![0, 1, 8, 9, 64, 65, 72, 73]
    );

    let row_partitions = restored.row_thread_partitions_bounded(8, 1024).unwrap();
    assert_eq!(row_partitions.len(), 6144 / 8);
    assert_eq!(row_partitions[0], vec![0, 1, 8, 9, 64, 65, 72, 73]);
}

#[test]
fn pearl_public_proof_params_reject_bad_offsets_and_envelope_violations() {
    let pattern = pearl_compat_test_pattern();
    let config = PearlMiningConfig {
        common_dim: 1216,
        rank: 64,
        mma_type: PEARL_MMA_INT7XINT7_TO_INT32,
        rows_pattern: pattern,
        cols_pattern: pattern,
        reserved: [0u8; PEARL_MINING_CONFIG_RESERVED_SIZE],
    };
    let mut params = PearlPublicProofParams {
        block_header: header(),
        mining_config: config,
        hash_a: [0xA1; 32],
        hash_b: [0xB2; 32],
        hash_jackpot: [0xC3; 32],
        m: 6144,
        n: 4096,
        t_rows: 0,
        t_cols: 0,
    };

    let mut public_data = params.to_public_data().unwrap();
    public_data[156..160].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(
        PearlPublicProofParams::from_public_data(header(), &public_data),
        Err(PearlCompatError::InvalidPatternOffset)
    );

    params.m = 72;
    assert_eq!(
        params.sanity_check(),
        Err(PearlCompatError::PatternOutOfMatrix)
    );

    params.m = 6144;
    params.mining_config.rank = 4;
    assert_eq!(
        params.sanity_check(),
        Err(PearlCompatError::PublicParamEnvelope)
    );
}

#[test]
fn pearl_nbits_and_adjusted_target_match_reference_edges() {
    let small = pearl_nbits_to_target_le(0x0312_3456);
    assert_eq!(&small[..4], &[0x56, 0x34, 0x12, 0x00]);
    assert!(small[4..].iter().all(|&b| b == 0));

    let genesis = pearl_nbits_to_target_le(0x1d00_ffff);
    assert_eq!(genesis[26], 0xff);
    assert_eq!(genesis[27], 0xff);
    assert!(genesis[..26].iter().all(|&b| b == 0));
    assert!(genesis[28..].iter().all(|&b| b == 0));

    assert_eq!(pearl_nbits_to_target_le(0x1d00_0000), [0u8; 32]);
    assert_eq!(pearl_nbits_to_target_le(0x1d80_0000), [0u8; 32]);

    let adjusted = pearl_adjust_target_for_config(0x0312_3456, &mining_config()).unwrap();
    let expected = 0x123456u128 * 4096u128;
    assert_eq!(&adjusted[..16], &expected.to_le_bytes());
    assert!(adjusted[16..].iter().all(|&b| b == 0));

    let saturated = pearl_adjust_target_for_config(0x207f_ffff, &mining_config()).unwrap();
    assert_eq!(saturated, [0xffu8; 32]);
}

#[test]
fn pearl_and_nockchain_target_checks_are_independent() {
    let mut public = PearlPublicProofParams {
        block_header: PearlIncompleteBlockHeader {
            nbits: 0x0100_0001,
            ..header()
        },
        mining_config: mining_config(),
        hash_a: [0xA1; 32],
        hash_b: [0xB2; 32],
        hash_jackpot: [1u8; 32],
        m: 64,
        n: 64,
        t_rows: 0,
        t_cols: 0,
    };

    assert_eq!(
        public.check_pearl_jackpot_difficulty(),
        Err(PearlCompatError::PearlTargetNotMet)
    );
    public.hash_jackpot = [0u8; 32];
    assert_eq!(public.check_pearl_jackpot_difficulty(), Ok(()));

    public.hash_jackpot = [7u8; 32];
    let mut nock_target = [7u8; 32];
    assert_eq!(public.check_nockchain_jackpot_target(&nock_target), Ok(()));
    nock_target[0] = 6;
    assert_eq!(
        public.check_nockchain_jackpot_target(&nock_target),
        Err(PearlCompatError::NockchainTargetNotMet)
    );
}

#[test]
fn pearl_pattern_ticket_matches_square_tile_when_patterns_are_contiguous() {
    let params = pearl_square_params();
    params.validate().unwrap();
    let config = pearl_square_config();
    let (a, b) = synth_matrices(b"pearl-pattern-square-ticket", &params);
    let attempt = PearlAttempt::build_with_config(&header(), &config, &a, &b, &params).unwrap();
    let first = &attempt.tile_digests[0];
    assert_eq!((first.tile_i, first.tile_j), (0, 0));

    let public = PearlPublicProofParams {
        block_header: header(),
        mining_config: config,
        hash_a: attempt.commitments.h_a,
        hash_b: attempt.commitments.h_b,
        hash_jackpot: first.jackpot_hash,
        m: params.m,
        n: params.n,
        t_rows: 0,
        t_cols: 0,
    };

    let ticket = verify_pearl_pattern_ticket(&public, &a, &b, &attempt.commitments, 16).unwrap();
    assert_eq!(ticket.a_rows, (0..8).collect::<Vec<u32>>());
    assert_eq!(ticket.b_cols, (0..8).collect::<Vec<u32>>());
    assert_eq!(ticket.tile_state, first.tile_state);
    assert_eq!(ticket.jackpot_hash, first.jackpot_hash);
}

#[test]
fn pearl_pattern_ticket_rejects_commitment_and_jackpot_tampering() {
    let params = pearl_square_params();
    let config = pearl_square_config();
    let (a, b) = synth_matrices(b"pearl-pattern-ticket-tamper", &params);
    let attempt = PearlAttempt::build_with_config(&header(), &config, &a, &b, &params).unwrap();
    let mut public = PearlPublicProofParams {
        block_header: header(),
        mining_config: config,
        hash_a: attempt.commitments.h_a,
        hash_b: attempt.commitments.h_b,
        hash_jackpot: attempt.tile_digests[0].jackpot_hash,
        m: params.m,
        n: params.n,
        t_rows: 0,
        t_cols: 0,
    };

    public.hash_a[0] ^= 0x01;
    assert_eq!(
        compute_pearl_pattern_ticket(&public, &a, &b, &attempt.commitments, 16),
        Err(PearlCompatError::PublicCommitmentMismatch)
    );

    public.hash_a = attempt.commitments.h_a;
    public.hash_jackpot[0] ^= 0x01;
    assert_eq!(
        verify_pearl_pattern_ticket(&public, &a, &b, &attempt.commitments, 16),
        Err(PearlCompatError::JackpotHashMismatch)
    );
}

#[test]
fn pearl_pattern_ticket_computes_noncontiguous_pattern_indices() {
    let params = pearl_square_params();
    let (a, b) = synth_matrices(b"pearl-pattern-noncontiguous-ticket", &params);
    let config = PearlMiningConfig {
        common_dim: params.k,
        rank: params.noise_rank as u16,
        mma_type: PEARL_MMA_INT7XINT7_TO_INT32,
        rows_pattern: pearl_compat_test_pattern(),
        cols_pattern: pearl_compat_test_pattern(),
        reserved: [0u8; PEARL_MINING_CONFIG_RESERVED_SIZE],
    };
    let commitments = ai_pow::pearl_compat::derive_pearl_work_commitments(
        &header().to_bytes(),
        &config.to_bytes().unwrap(),
        &a,
        &b,
    );
    let mut public = PearlPublicProofParams {
        block_header: header(),
        mining_config: config,
        hash_a: commitments.h_a,
        hash_b: commitments.h_b,
        hash_jackpot: [0u8; 32],
        m: params.m,
        n: params.n,
        t_rows: 0,
        t_cols: 0,
    };

    let ticket = compute_pearl_pattern_ticket(&public, &a, &b, &commitments, 16).unwrap();
    assert_eq!(ticket.a_rows, vec![0, 1, 8, 9, 64, 65, 72, 73]);
    assert_eq!(ticket.b_cols, vec![0, 1, 8, 9, 64, 65, 72, 73]);
    public.hash_jackpot = ticket.jackpot_hash;
    assert_eq!(
        verify_pearl_pattern_ticket(&public, &a, &b, &commitments, 16).unwrap(),
        ticket
    );
}

#[test]
fn pearl_attempt_transcript_matches_reference_formulas() {
    let params = MatmulParams::TEST_SMALL;
    let (a, b) = synth_matrices(b"pearl-merge-transcript", &params);
    let header = header();
    let config = mining_config();
    let sigma = header.to_bytes();
    let mu = config.to_bytes().unwrap();
    let attempt = PearlAttempt::build_with_config(&header, &config, &a, &b, &params).unwrap();

    let mut raw = Vec::new();
    raw.extend_from_slice(&sigma);
    raw.extend_from_slice(&mu);
    let expected_kappa = *blake3::hash(&raw).as_bytes();
    assert_eq!(attempt.commitments.kappa, expected_kappa);
    assert_eq!(attempt.commitments.kappa, pearl_kappa(&sigma, &mu));

    let a_bytes: Vec<u8> = a.iter().map(|&v| v as u8).collect();
    let b_bytes: Vec<u8> = b.iter().map(|&v| v as u8).collect();
    let expected_h_a = matrix_commitment(&a_bytes, &expected_kappa);
    let expected_h_b = matrix_commitment(&b_bytes, &expected_kappa);
    assert_eq!(attempt.commitments.h_a, expected_h_a);
    assert_eq!(attempt.commitments.h_b, expected_h_b);

    let expected_s_b = noise_seed_b(&expected_kappa, &expected_h_b);
    let expected_s_a = noise_seed_a(&expected_s_b, &expected_h_a);
    assert_eq!(attempt.commitments.s_a, expected_s_a);
    assert_eq!(attempt.commitments.s_b, expected_s_b);

    assert_eq!(attempt.tile_digests.len(), params.num_tiles() as usize);
    let first = &attempt.tile_digests[0];
    assert_eq!(first.tile_i, 0);
    assert_eq!(first.tile_j, 0);
    assert_eq!(
        first.jackpot_hash,
        first.tile_state.keyed_hash(&attempt.commitments.s_a),
        "Pearl-compatible jackpot hashing must key directly with s_A"
    );
}

#[test]
fn pearl_attempt_is_not_the_native_nockchain_nonce_path() {
    let params = MatmulParams::TEST_SMALL;
    let (a, b) = synth_matrices(b"pearl-merge-native-separation", &params);
    let header = header();
    let config = mining_config();
    let sigma = header.to_bytes();
    let pearl = PearlAttempt::build_with_config(&header, &config, &a, &b, &params).unwrap();

    let native_nonce = b"native-ncmn-nonce-bytes";
    let native = BlockContext::build(&sigma, native_nonce, &a, &b, &params).unwrap();
    assert_ne!(
        pearl.commitments.kappa,
        *native.kappa(),
        "native mode length-prefixes block/nonce and hashes params_tag; Pearl mode is sigma || mu"
    );
    assert_ne!(
        pearl.commitments.s_a,
        native.pow_key(),
        "Pearl-compatible mode must not use pow_key_for_nonce(s_A, nonce)"
    );

    let native_selected = ai_pow::fiat_shamir::attempt_tile_index(
        native.attempt_state(),
        &params_tag(&params),
        native.s_a(),
        params.num_tiles(),
    ) as usize;
    assert!(native_selected < pearl.tile_digests.len());
    assert_eq!(
        pearl.tile_digests.len(),
        params.num_tiles() as usize,
        "Pearl-compatible mining exposes every legal tile as a ticket"
    );
}

#[test]
fn changing_sigma_changes_pearl_work_before_jackpot_hashing() {
    let params = MatmulParams::TEST_SMALL;
    let (a, b) = synth_matrices(b"pearl-merge-sigma-binding", &params);
    let sigma_a = header().to_bytes();
    let mut sigma_b = sigma_a;
    sigma_b[10] ^= 0x80;
    let mu = mining_config().to_bytes().unwrap();
    let attempt_a = PearlAttempt::build_from_serialized(&sigma_a, &mu, &a, &b, &params).unwrap();
    let attempt_b = PearlAttempt::build_from_serialized(&sigma_b, &mu, &a, &b, &params).unwrap();

    assert_ne!(attempt_a.commitments.kappa, attempt_b.commitments.kappa);
    assert_ne!(attempt_a.commitments.h_a, attempt_b.commitments.h_a);
    assert_ne!(attempt_a.commitments.h_b, attempt_b.commitments.h_b);
    assert_ne!(attempt_a.commitments.s_a, attempt_b.commitments.s_a);
    assert_ne!(
        attempt_a.tile_digests[0].jackpot_hash,
        attempt_b.tile_digests[0].jackpot_hash
    );

    assert_ne!(sigma_a, sigma_b);
}

#[test]
fn pearl_attempt_rejects_config_that_does_not_match_params() {
    let params = MatmulParams::TEST_SMALL;
    let (a, b) = synth_matrices(b"pearl-merge-config-binding", &params);
    let header = header();
    let mut config = mining_config();

    config.common_dim = params.k + 1;
    assert_eq!(
        PearlAttempt::build_with_config(&header, &config, &a, &b, &params),
        Err(PearlCompatError::CommonDimMismatch)
    );

    config = mining_config();
    config.rank = (params.noise_rank + 1) as u16;
    assert_eq!(
        PearlAttempt::build_with_config(&header, &config, &a, &b, &params),
        Err(PearlCompatError::RankMismatch)
    );
}

#[test]
fn pearl_compatible_work_precheck_accepts_shared_attempt_for_both_targets() {
    let params = pearl_square_params();
    let config = pearl_square_config();
    let easy_header = PearlIncompleteBlockHeader {
        nbits: 0x207f_ffff,
        ..header()
    };
    let (a, b) = synth_matrices(b"pearl-compatible-work-precheck", &params);
    let attempt = PearlAttempt::build_with_config(&easy_header, &config, &a, &b, &params).unwrap();
    let public = PearlPublicProofParams {
        block_header: easy_header,
        mining_config: config,
        hash_a: attempt.commitments.h_a,
        hash_b: attempt.commitments.h_b,
        hash_jackpot: attempt.tile_digests[0].jackpot_hash,
        m: params.m,
        n: params.n,
        t_rows: 0,
        t_cols: 0,
    };
    let nockchain_target = [0xffu8; 32];

    let precheck = verify_pearl_compatible_work(&public, &a, &b, &nockchain_target, 16).unwrap();
    assert_eq!(precheck.commitments, attempt.commitments);
    assert_eq!(precheck.ticket.jackpot_hash, public.hash_jackpot);
    assert_eq!(
        precheck.ticket.tile_state,
        attempt.tile_digests[0].tile_state
    );
    assert_eq!(precheck.pearl_target, [0xffu8; 32]);
    assert_eq!(precheck.nockchain_target, nockchain_target);
}

#[test]
fn pearl_compatible_work_precheck_rejects_tampered_statement_fields() {
    let params = pearl_square_params();
    let config = pearl_square_config();
    let easy_header = PearlIncompleteBlockHeader {
        nbits: 0x207f_ffff,
        ..header()
    };
    let (a, b) = synth_matrices(b"pearl-compatible-work-tamper", &params);
    let attempt = PearlAttempt::build_with_config(&easy_header, &config, &a, &b, &params).unwrap();
    let public = PearlPublicProofParams {
        block_header: easy_header,
        mining_config: config,
        hash_a: attempt.commitments.h_a,
        hash_b: attempt.commitments.h_b,
        hash_jackpot: attempt.tile_digests[0].jackpot_hash,
        m: params.m,
        n: params.n,
        t_rows: 0,
        t_cols: 0,
    };
    let nockchain_target = [0xffu8; 32];

    let mut bad_header = public.clone();
    bad_header.block_header.timestamp ^= 1;
    assert_eq!(
        verify_pearl_compatible_work(&bad_header, &a, &b, &nockchain_target, 16),
        Err(PearlCompatError::PublicCommitmentMismatch)
    );

    let mut bad_config = public.clone();
    bad_config.mining_config.rows_pattern = pearl_compat_test_pattern();
    assert_eq!(
        verify_pearl_compatible_work(&bad_config, &a, &b, &nockchain_target, 16),
        Err(PearlCompatError::PublicCommitmentMismatch)
    );

    let mut bad_jackpot = public.clone();
    bad_jackpot.hash_jackpot[0] ^= 1;
    assert_eq!(
        verify_pearl_compatible_work(&bad_jackpot, &a, &b, &nockchain_target, 16),
        Err(PearlCompatError::JackpotHashMismatch)
    );
}

#[test]
fn pearl_compatible_work_precheck_rejects_target_and_input_failures() {
    let params = pearl_square_params();
    let config = pearl_square_config();
    let easy_header = PearlIncompleteBlockHeader {
        nbits: 0x207f_ffff,
        ..header()
    };
    let (a, b) = synth_matrices(b"pearl-compatible-work-targets", &params);
    let attempt = PearlAttempt::build_with_config(&easy_header, &config, &a, &b, &params).unwrap();
    let mut public = PearlPublicProofParams {
        block_header: easy_header,
        mining_config: config,
        hash_a: attempt.commitments.h_a,
        hash_b: attempt.commitments.h_b,
        hash_jackpot: attempt.tile_digests[0].jackpot_hash,
        m: params.m,
        n: params.n,
        t_rows: 0,
        t_cols: 0,
    };
    assert_ne!(public.hash_jackpot, [0u8; 32]);

    let nockchain_target = [0u8; 32];
    assert_eq!(
        verify_pearl_compatible_work(&public, &a, &b, &nockchain_target, 16),
        Err(PearlCompatError::NockchainTargetNotMet)
    );

    public.block_header.nbits = 0x0100_0001;
    let nockchain_target = [0xffu8; 32];
    assert_eq!(
        verify_pearl_compatible_work(&public, &a, &b, &nockchain_target, 16),
        Err(PearlCompatError::PearlTargetNotMet)
    );

    public.block_header = easy_header;
    assert_eq!(
        verify_pearl_compatible_work(&public, &a[..a.len() - 1], &b, &nockchain_target, 16),
        Err(PearlCompatError::InputAShape {
            expected: params.m as usize * params.k as usize,
            actual: params.m as usize * params.k as usize - 1,
        })
    );
}
