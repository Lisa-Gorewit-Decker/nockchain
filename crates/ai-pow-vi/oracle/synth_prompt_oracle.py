"""Numpy/blake3 reference of crate::prompt::synth_prompt.

Matches the Rust impl byte-for-byte: same domain context, same XOF
read order, same modulo + reserved-rejection loop.
"""

from __future__ import annotations

from typing import Sequence

import blake3

CTX_PROMPT = "ai-pow-vi v1 prompt"


def synth_prompt(
    block_commitment: bytes,
    model_id: bytes,
    seq_len: int,
    vocab_size: int,
    reserved_tokens: Sequence[int] = (),
) -> list[int]:
    if seq_len <= 0:
        raise ValueError("seq_len must be > 0")
    if vocab_size <= 0:
        raise ValueError("vocab_size must be > 0")
    if len(model_id) != 32:
        raise ValueError("model_id must be 32 bytes")
    if len(reserved_tokens) >= vocab_size:
        raise ValueError("reserved tokens cover the entire vocabulary")

    h = blake3.blake3(derive_key_context=CTX_PROMPT)
    h.update(block_commitment)
    h.update(model_id)

    out: list[int] = []
    seek = 0
    reserved_set = set(int(t) for t in reserved_tokens)
    while len(out) < seq_len:
        chunk = h.digest(length=4, seek=seek)
        seek += 4
        raw = int.from_bytes(chunk, "little")
        tok = raw % vocab_size
        if tok in reserved_set:
            continue
        out.append(tok)
    return out


if __name__ == "__main__":
    # Smoke test against the Rust pin fixture in tests/determinism_pins.rs.
    block = b"ai-pow-vi pin block-commitment v1"
    model_id = bytes([0xA5] * 32)
    reserved = [0, 1, 2]
    p = synth_prompt(block, model_id, 32, 256, reserved)
    print("first 8 tokens:", p[:8])
    print("len:", len(p))
    print("any reserved?", any(t in reserved for t in p))
