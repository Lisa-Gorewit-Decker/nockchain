//! Fiat-Shamir-derived synthetic prompt generation.
//!
//! Given a per-block commitment and a model identifier, produce a
//! deterministic stream of token IDs to use as the puzzle's prompt.
//! No prompt content is mined into the chain; the puzzle is "answer this
//! prompt with this model and find a tile of the answer that hashes
//! below the target."
//!
//! Algorithm (pinned by `CTX_PROMPT`; consensus-stable):
//! 1. Seed a domain-separated BLAKE3 keyed-hash with
//!    `block_commitment || model_id`.
//! 2. Read 4 bytes at a time from its XOF; interpret as little-endian
//!    `u32`.
//! 3. If `tok = u32 % vocab_size` is in `reserved_tokens`, reject and
//!    redraw. Otherwise emit `tok`.
//! 4. Stop when `seq_len` tokens have been emitted.
//!
//! The redraw step keeps the output uniformly distributed over the
//! non-reserved vocabulary even when the reserved set is large.

use blake3::Hasher;
use thiserror::Error;

use crate::model::Token;

const CTX_PROMPT: &str = "ai-pow-vi v1 prompt";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum PromptError {
    #[error("seq_len must be > 0")]
    ZeroSeqLen,
    #[error("vocab_size must be > 0")]
    ZeroVocab,
    #[error("reserved tokens cover the entire vocabulary; no token is admissible")]
    NoAdmissibleToken,
}

/// Generate a deterministic prompt of length `seq_len` for the puzzle.
///
/// `block_commitment` is the per-block challenge seed (e.g.
/// `prev_block_commitment || nonce`). `model_id` is the 32-byte
/// content-address of the model's weight file. `reserved_tokens` is the
/// set of token IDs that must NOT appear (e.g. BOS, EOS, special tags);
/// pass an empty slice if the model has no reserved tokens.
pub fn synth_prompt(
    block_commitment: &[u8],
    model_id: &[u8; 32],
    seq_len: u32,
    vocab_size: u32,
    reserved_tokens: &[Token],
) -> Result<Vec<Token>, PromptError> {
    if seq_len == 0 {
        return Err(PromptError::ZeroSeqLen);
    }
    if vocab_size == 0 {
        return Err(PromptError::ZeroVocab);
    }
    if reserved_tokens.len() as u64 >= vocab_size as u64 {
        return Err(PromptError::NoAdmissibleToken);
    }

    let mut hasher = Hasher::new_derive_key(CTX_PROMPT);
    hasher.update(block_commitment);
    hasher.update(model_id);
    let mut xof = hasher.finalize_xof();

    let mut out = Vec::with_capacity(seq_len as usize);
    while out.len() < seq_len as usize {
        let mut buf = [0u8; 4];
        xof.fill(&mut buf);
        let raw = u32::from_le_bytes(buf);
        let tok = raw % vocab_size;
        if reserved_tokens.contains(&tok) {
            continue;
        }
        out.push(tok);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determinism_same_inputs_same_output() {
        let block = b"block-commitment-bytes";
        let model_id = [42u8; 32];
        let a = synth_prompt(block, &model_id, 16, 1000, &[]).unwrap();
        let b = synth_prompt(block, &model_id, 16, 1000, &[]).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn different_block_commitment_yields_different_prompt() {
        let model_id = [7u8; 32];
        let a = synth_prompt(b"alpha", &model_id, 32, 5000, &[]).unwrap();
        let b = synth_prompt(b"beta", &model_id, 32, 5000, &[]).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn different_model_id_yields_different_prompt() {
        let block = b"same-block";
        let a = synth_prompt(block, &[1u8; 32], 32, 5000, &[]).unwrap();
        let b = synth_prompt(block, &[2u8; 32], 32, 5000, &[]).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn reserved_tokens_never_appear() {
        let block = b"reserved-test";
        let model_id = [0u8; 32];
        let reserved: Vec<Token> = (0..50).collect(); // tokens 0..49 reserved.
        let prompt = synth_prompt(block, &model_id, 64, 100, &reserved).unwrap();
        for &t in &prompt {
            assert!(t >= 50, "got reserved token {t}");
            assert!(t < 100, "got out-of-vocab token {t}");
        }
    }

    #[test]
    fn rejects_zero_dims() {
        assert_eq!(
            synth_prompt(b"x", &[0u8; 32], 0, 100, &[]).err(),
            Some(PromptError::ZeroSeqLen),
        );
        assert_eq!(
            synth_prompt(b"x", &[0u8; 32], 1, 0, &[]).err(),
            Some(PromptError::ZeroVocab),
        );
    }

    #[test]
    fn rejects_when_reserved_covers_entire_vocab() {
        let reserved: Vec<Token> = (0..10).collect();
        assert_eq!(
            synth_prompt(b"x", &[0u8; 32], 1, 10, &reserved).err(),
            Some(PromptError::NoAdmissibleToken),
        );
    }

    #[test]
    fn tokens_are_within_vocab() {
        let prompt = synth_prompt(b"vocab-bound", &[0u8; 32], 256, 1024, &[]).unwrap();
        for &t in &prompt {
            assert!(t < 1024);
        }
    }

    #[test]
    fn small_vocab_with_no_reserved_terminates() {
        // vocab=2, no reserved → all draws are admissible. Should produce
        // any seq_len without spinning.
        let p = synth_prompt(b"small", &[0u8; 32], 32, 2, &[]).unwrap();
        assert_eq!(p.len(), 32);
        for &t in &p {
            assert!(t < 2);
        }
    }

    #[test]
    fn high_reserved_density_still_terminates() {
        // vocab=10, reserved=9 of them → only token 9 is admissible. Must
        // produce all-9 prompt without infinite-looping.
        let reserved: Vec<Token> = (0..9).collect();
        let p = synth_prompt(b"dense", &[0u8; 32], 8, 10, &reserved).unwrap();
        for &t in &p {
            assert_eq!(t, 9);
        }
    }
}
