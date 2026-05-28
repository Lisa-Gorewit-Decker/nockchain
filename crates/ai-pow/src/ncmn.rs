//! NCMN v1 nonce format shared by the miner and verifier.
//!
//! The nonce is part of the PoW transcript, but it also carries a chain anchor
//! that the production verifier must check before accepting work for a
//! candidate Nockchain block.
//!
//! Layout:
//!
//! ```text
//! 0    magic                 4 bytes  b"NCMN"
//! 4    version               1 byte   1
//! 5    reserved              3 bytes  all zero
//! 8    nck_commitment        32 bytes required Nockchain block anchor
//! 40   external_commitment   32 bytes zero means absent
//! 72   extranonce            8 bytes  big-endian u64
//! ```

/// NCMN nonce magic.
pub const NCMN_MAGIC: [u8; 4] = *b"NCMN";
pub const NCMN_VERSION: u8 = 1;
pub const NCMN_NONCE_LEN: usize = 80;
pub type NcmnNonce = [u8; NCMN_NONCE_LEN];

/// All-zero external slot means no external-chain commitment is supplied.
pub const NCMN_EXTERNAL_ABSENT: [u8; 32] = [0u8; 32];

/// Anchors carried by an NCMN nonce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NonceAnchors {
    pub nck_commitment: [u8; 32],
    pub external_commitment: Option<[u8; 32]>,
}

impl NonceAnchors {
    pub fn nck_only(nck_commitment: [u8; 32]) -> Self {
        Self {
            nck_commitment,
            external_commitment: None,
        }
    }
}

#[derive(thiserror::Error, Debug, PartialEq, Eq)]
pub enum NonceFormatError {
    #[error("nonce length {0} != NCMN expected {NCMN_NONCE_LEN}")]
    BadLength(usize),
    #[error("nonce magic {0:?} != NCMN")]
    BadMagic([u8; 4]),
    #[error("nonce version {got} != NCMN version {expected}")]
    BadVersion { got: u8, expected: u8 },
    #[error("nonce reserved bytes must be zero, got {0:?}")]
    BadReserved([u8; 3]),
}

/// Compose an NCMN v1 nonce from anchors and an extranonce.
pub fn build_ncmn_nonce(anchors: &NonceAnchors, extranonce: u64) -> NcmnNonce {
    let mut out = [0u8; NCMN_NONCE_LEN];
    out[0..4].copy_from_slice(&NCMN_MAGIC);
    out[4] = NCMN_VERSION;
    out[8..40].copy_from_slice(&anchors.nck_commitment);
    out[40..72].copy_from_slice(
        &anchors.external_commitment.unwrap_or(NCMN_EXTERNAL_ABSENT),
    );
    out[72..80].copy_from_slice(&extranonce.to_be_bytes());
    out
}

/// Parse an NCMN v1 nonce. The external commitment is `None` iff its 32-byte
/// slot is all zero.
pub fn parse_ncmn_nonce(
    nonce: &[u8],
) -> Result<(NonceAnchors, u64), NonceFormatError> {
    if nonce.len() != NCMN_NONCE_LEN {
        return Err(NonceFormatError::BadLength(nonce.len()));
    }
    let mut magic = [0u8; 4];
    magic.copy_from_slice(&nonce[0..4]);
    if magic != NCMN_MAGIC {
        return Err(NonceFormatError::BadMagic(magic));
    }
    if nonce[4] != NCMN_VERSION {
        return Err(NonceFormatError::BadVersion {
            got: nonce[4],
            expected: NCMN_VERSION,
        });
    }
    let mut reserved = [0u8; 3];
    reserved.copy_from_slice(&nonce[5..8]);
    if reserved != [0u8; 3] {
        return Err(NonceFormatError::BadReserved(reserved));
    }
    let mut nck = [0u8; 32];
    nck.copy_from_slice(&nonce[8..40]);
    let mut ext = [0u8; 32];
    ext.copy_from_slice(&nonce[40..72]);
    let mut xn = [0u8; 8];
    xn.copy_from_slice(&nonce[72..80]);
    Ok((
        NonceAnchors {
            nck_commitment: nck,
            external_commitment: if ext == NCMN_EXTERNAL_ABSENT {
                None
            } else {
                Some(ext)
            },
        },
        u64::from_be_bytes(xn),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ncmn_round_trip_with_external_anchor() {
        let anchors = NonceAnchors {
            nck_commitment: [0x11; 32],
            external_commitment: Some([0x22; 32]),
        };
        for xn in [0u64, 1, 42, u64::MAX] {
            let bytes = build_ncmn_nonce(&anchors, xn);
            let (a2, xn2) = parse_ncmn_nonce(&bytes).expect("parse");
            assert_eq!(a2, anchors);
            assert_eq!(xn2, xn);
        }
    }

    #[test]
    fn ncmn_external_absent_round_trips_as_none() {
        let anchors = NonceAnchors::nck_only([0x33; 32]);
        let bytes = build_ncmn_nonce(&anchors, 7);
        assert_eq!(&bytes[40..72], &[0u8; 32]);
        let (a2, _) = parse_ncmn_nonce(&bytes).unwrap();
        assert_eq!(a2, anchors);
    }

    #[test]
    fn ncmn_rejects_malformed_headers() {
        assert_eq!(
            parse_ncmn_nonce(&[0u8; 32]).unwrap_err(),
            NonceFormatError::BadLength(32),
        );

        let mut bytes = build_ncmn_nonce(&NonceAnchors::nck_only([0x44; 32]), 0);
        bytes[0] = b'X';
        assert_eq!(
            parse_ncmn_nonce(&bytes).unwrap_err(),
            NonceFormatError::BadMagic(*b"XCMN"),
        );

        let mut bytes = build_ncmn_nonce(&NonceAnchors::nck_only([0x55; 32]), 0);
        bytes[4] = 99;
        assert_eq!(
            parse_ncmn_nonce(&bytes).unwrap_err(),
            NonceFormatError::BadVersion {
                got: 99,
                expected: 1,
            },
        );

        let mut bytes = build_ncmn_nonce(&NonceAnchors::nck_only([0x66; 32]), 0);
        bytes[6] = 1;
        assert_eq!(
            parse_ncmn_nonce(&bytes).unwrap_err(),
            NonceFormatError::BadReserved([0, 1, 0]),
        );
    }
}
