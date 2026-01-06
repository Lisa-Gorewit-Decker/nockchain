use libp2p::PeerId;
use nockapp::noun::slab::NounSlab;
use nockapp::NockAppError;
use nockvm::ext::NounExt;
use nockvm::noun::{Noun, NounAllocator, NounSpace, D};
use nockvm_macros::tas;
use serde_bytes::ByteBuf;

use crate::p2p_util::PeerIdExt;
use crate::tip5_util::{tip5_hash_to_base58, tip5_hash_to_base58_stack};

const POKE_VERSION: u64 = 0;

#[derive(Debug, Clone)]
pub enum NockchainFact {
    // [%heard-block p=page:dt] with poke slab
    HeardBlock(String, NounSlab),
    // [%heard-elders p=[oldest=page-number:dt ids=(list block-id:dt)]] with poke slab
    HeardElders(u64, Vec<String>, NounSlab),
    // [%heard-tx p=raw-tx:dt] with poke slab
    HeardTx(String, NounSlab),
}

impl NockchainFact {
    pub fn from_noun_slab(slab: &mut NounSlab) -> Result<Self, NockAppError> {
        let mut poke_slab = NounSlab::new();

        poke_slab.copy_from_slab(slab);
        poke_slab.modify(|response_noun| vec![D(tas!(b"fact")), D(POKE_VERSION), response_noun]);

        let noun = unsafe { slab.root() };
        let space = slab.noun_space();
        let head = noun.as_cell()?.head(&space);

        if head.eq_bytes(b"heard-block", &space) {
            let page = noun.as_cell()?.tail(&space);
            let block_id = block_id_from_page(page, &space)?;
            let block_id_str = tip5_hash_to_base58_stack(slab, block_id, &space)?;
            Ok(NockchainFact::HeardBlock(block_id_str, poke_slab))
        } else if head.eq_bytes(b"heard-tx", &space) {
            let raw_tx = noun.as_cell()?.tail(&space);
            let tx_id = tx_id_from_raw_tx(raw_tx, &space)?;
            let tx_id_str = tip5_hash_to_base58_stack(slab, tx_id, &space)?;
            Ok(NockchainFact::HeardTx(tx_id_str, poke_slab))
        } else if head.eq_bytes(b"heard-elders", &space) {
            let elders_dat = noun.as_cell()?.tail(&space);
            let oldest = elders_dat
                .as_cell()?
                .head(&space)
                .as_atom()?
                .as_u64(&space)?;
            let elder_ids = elders_dat.as_cell()?.tail(&space);
            // Need to handle the closure capturing mutable reference
            let mut elder_id_strings = Vec::new();
            for id_noun in elder_ids.list_iter(&space) {
                elder_id_strings.push(tip5_hash_to_base58_stack(slab, id_noun, &space)?);
            }
            Ok(NockchainFact::HeardElders(
                oldest, elder_id_strings, poke_slab,
            ))
        } else {
            Err(NockAppError::OtherError(String::from(
                "Invalid fact head tag",
            )))
        }
    }
    pub fn fact_poke(&self) -> &NounSlab {
        match self {
            Self::HeardBlock(_, slab) => &slab,
            Self::HeardTx(_, slab) => &slab,
            Self::HeardElders(_, _, slab) => &slab,
        }
    }
}

fn block_id_from_page(page: Noun, space: &NounSpace) -> Result<Noun, NockAppError> {
    let page_cell = page.as_cell()?;
    // page v0: [block-id ...]
    // page v1: [%1 block-id ...]
    match page_cell.head(space).as_atom() {
        Ok(version_atom) => {
            let version = version_atom.as_u64(space)?;
            if version == 1 {
                return Ok(page_cell.tail(space).as_cell()?.head(space));
            }
            return Err(NockAppError::OtherError(format!(
                "Unsupported page version {}",
                version
            )));
        }
        Err(_) => Ok(page_cell.head(space)),
    }
}

fn tx_id_from_raw_tx(raw_tx: Noun, space: &NounSpace) -> Result<Noun, NockAppError> {
    let raw_tx_cell = raw_tx.as_cell()?;
    // raw-tx v0: [tx-id ...]
    // raw-tx v1: [%1 tx-id ...]
    match raw_tx_cell.head(space).as_atom() {
        Ok(version_atom) => {
            let version = version_atom.as_u64(space)?;
            if version == 1 {
                return Ok(raw_tx_cell.tail(space).as_cell()?.head(space));
            }
            return Err(NockAppError::OtherError(format!(
                "Unsupported raw-tx version {}",
                version
            )));
        }
        Err(_) => Ok(raw_tx_cell.head(space)),
    }
}

#[derive(Debug, Clone)]
pub enum NockchainDataRequest {
    BlockByHeight(u64), // Height requested
    #[allow(dead_code)]
    EldersById(String, PeerId, NounSlab), // Block ID as string, peer id, block id as noun,
    #[allow(dead_code)]
    RawTransactionById(String, NounSlab), // transaction id as string, transaction id as noun,
}

impl NockchainDataRequest {
    /// Takes noun of type [%request p=request]
    pub fn from_noun(noun: Noun, space: &NounSpace) -> Result<Self, NockAppError> {
        let res = (|| {
            let request_cell = noun.as_cell()?;
            if !request_cell.head(space).eq_bytes(b"request", space) {
                return Err(NockAppError::OtherError(String::from(
                    "Missing %request tag",
                )));
            }
            // kind cell type $%([%block request-block] [%raw-tx request-tx])
            let kind_cell = request_cell.tail(space).as_cell()?;
            if kind_cell.head(space).eq_bytes(b"block", space) {
                // block_cell type
                // $%  [%by-height p=page-number:dt]
                //     [%elders p=block-id:dt q=peer-id]
                // ==
                let block_cell = kind_cell.tail(space).as_cell()?;
                if block_cell.head(space).eq_bytes(b"by-height", space) {
                    let height = block_cell.tail(space).as_atom()?.as_u64(space)?;
                    Ok(Self::BlockByHeight(height))
                } else if block_cell.head(space).eq_bytes(b"elders", space) {
                    let elders_cell = block_cell.tail(space).as_cell()?;
                    let block_id = tip5_hash_to_base58(elders_cell.head(space), space)?;
                    let peer_id = PeerId::from_noun(elders_cell.tail(space), space)?;
                    let slab = {
                        let mut slab = NounSlab::new();
                        slab.copy_into(elders_cell.head(space), space);
                        slab
                    };
                    Ok(Self::EldersById(block_id, peer_id, slab))
                } else {
                    Err(NockAppError::OtherError(String::from(
                        "Failed to parse EldersById message",
                    )))
                }
            } else if kind_cell.head(space).eq_bytes(b"raw-tx", space) {
                // has type [%by-id p=tx-id:dt]
                let raw_tx_cell = kind_cell.tail(space).as_cell()?;
                let raw_tx_id = tip5_hash_to_base58(raw_tx_cell.tail(space), space)?;
                let slab = {
                    let mut slab = NounSlab::new();
                    slab.copy_into(raw_tx_cell.tail(space), space);
                    slab
                };
                Ok(Self::RawTransactionById(raw_tx_id, slab))
            } else {
                Err(NockAppError::OtherError(String::from(
                    "Failed to parse RawTransaction message",
                )))
            }
        })();
        res.map_err(|_| {
            NockAppError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                "bad request",
            ))
        })
    }
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// Network struct (in serde/CBOR) for requests
pub enum NockchainRequest {
    /// Request a block or TX from another node, carry PoW
    Request {
        pow: equix::SolutionByteArray,
        nonce: u64,
        message: ByteBuf,
    },
    /// Gossip a block or TX to another node
    Gossip { message: ByteBuf },
}

impl NockchainRequest {
    /// Make a new "request" which gossips a block or a TX
    pub(crate) fn new_gossip(message: &NounSlab) -> NockchainRequest {
        let message_bytes = ByteBuf::from(message.jam().as_ref());
        NockchainRequest::Gossip {
            message: message_bytes,
        }
    }

    /// Make a new request for a block or a TX
    pub(crate) fn new_request(
        builder: &mut equix::EquiXBuilder,
        local_peer_id: &libp2p::PeerId,
        remote_peer_id: &libp2p::PeerId,
        message: &NounSlab,
    ) -> NockchainRequest {
        let message_bytes = ByteBuf::from(message.jam().as_ref());
        let local_peer_bytes = (*local_peer_id).to_bytes();
        let remote_peer_bytes = (*remote_peer_id).to_bytes();
        let mut pow_buf = Vec::with_capacity(
            size_of::<u64>()
                + local_peer_bytes.len()
                + remote_peer_bytes.len()
                + message_bytes.len(),
        );
        pow_buf.extend_from_slice(&[0; size_of::<u64>()][..]);
        pow_buf.extend_from_slice(&local_peer_bytes[..]);
        pow_buf.extend_from_slice(&remote_peer_bytes[..]);
        pow_buf.extend_from_slice(&message_bytes[..]);

        let mut nonce = 0u64;
        let sol_bytes = loop {
            {
                let nonce_buf = &mut pow_buf[0..size_of::<u64>()];
                nonce_buf.copy_from_slice(&nonce.to_le_bytes()[..]);
            }
            if let Ok(sols) = builder.solve(&pow_buf[..]) {
                if !sols.is_empty() {
                    break sols[0].to_bytes();
                }
            }
            nonce += 1;
        };

        NockchainRequest::Request {
            pow: sol_bytes,
            nonce,
            message: message_bytes,
        }
    }

    /// Verify the EquiX PoW attached to a request
    pub(crate) fn verify_pow(
        &self,
        builder: &mut equix::EquiXBuilder,
        local_peer_id: &libp2p::PeerId,
        remote_peer_id: &libp2p::PeerId,
    ) -> Result<(), equix::Error> {
        match self {
            NockchainRequest::Request {
                pow,
                nonce,
                message,
            } => {
                //  This looks backwards, but it's because which node is local and which is remote
                //  is swapped between generation at the sender and verification at the receiver.
                let local_peer_bytes = (*remote_peer_id).to_bytes();
                let remote_peer_bytes = (*local_peer_id).to_bytes();
                let nonce_bytes = nonce.to_le_bytes();
                let mut pow_buf = Vec::with_capacity(
                    size_of::<u64>()
                        + local_peer_bytes.len()
                        + remote_peer_bytes.len()
                        + message.len(),
                );
                pow_buf.extend_from_slice(&nonce_bytes[..]);
                pow_buf.extend_from_slice(&local_peer_bytes[..]);
                pow_buf.extend_from_slice(&remote_peer_bytes[..]);
                pow_buf.extend_from_slice(&message[..]);
                builder.verify_bytes(&pow_buf[..], pow)
            }
            NockchainRequest::Gossip { message: _ } => Ok(()),
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
/// Responses to Nockchain requests
pub enum NockchainResponse {
    /// The requested block or raw-tx
    Result { message: ByteBuf },
    /// If the request was a gossip, no actual response is needed
    Ack { acked: bool },
}

impl NockchainResponse {
    pub(crate) fn new_response_result(message: impl AsRef<[u8]>) -> NockchainResponse {
        let message_bytes: &[u8] = message.as_ref();
        let message_bytebuf = ByteBuf::from(message_bytes.to_vec());
        NockchainResponse::Result {
            message: message_bytebuf,
        }
    }
}
