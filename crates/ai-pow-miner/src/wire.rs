//! Wire vocabulary for the AI-PoW miner.
//!
//! [`AiPowMinerWire`] mirrors `zk_pow_miner::wire::ZkPowMinerWire`
//! (`SOURCE = "zk-pow-miner"`) on its own source `"ai-pow-miner"` so the
//! kernel-side handler for the AI puzzle can be registered independently
//! of the dumb-puzzle ZK PoW path. Both wires share the same tag
//! vocabulary (`enable`, `candidate`, `mined`, `setpubkey`,
//! `mining-error`).
//!
//! Submission flow:
//! 1. The run loop builds an `[%mined nonce-atom found-idx]` noun.
//! 2. It pokes the node over gRPC via
//!    [`nockchain_mining_common::NodeClient::poke_wire`] with
//!    `AiPowMinerWire::Mined.to_wire()`.
//! 3. The kernel-side AI-puzzle handler (future work) decodes the
//!    payload on its `source = "ai-pow-miner"` branch.

use nockapp::nockapp::wire::{Wire, WireRepr};

pub enum AiPowMinerWire {
    /// Driver → node: enable / disable mining.
    Enable,
    /// Kernel-internal: a new candidate puzzle.
    Candidate,
    /// Driver → node: solved tile. Payload (v1):
    /// `[%mined nonce-as-atom found-idx-as-atom]`.
    Mined,
    /// Driver → node: set mining-payout pubkey(s).
    SetPubKey,
    /// Driver → node: mining terminated without a solution.
    /// Payload: `[%mining-error message-as-atom]`.
    MiningError,
}

impl AiPowMinerWire {
    pub fn label(&self) -> &'static str {
        match self {
            AiPowMinerWire::Enable => "enable",
            AiPowMinerWire::Candidate => "candidate",
            AiPowMinerWire::Mined => "mined",
            AiPowMinerWire::SetPubKey => "setpubkey",
            AiPowMinerWire::MiningError => "mining-error",
        }
    }
}

impl Wire for AiPowMinerWire {
    const VERSION: u64 = 1;
    const SOURCE: &'static str = "ai-pow-miner";

    fn to_wire(&self) -> WireRepr {
        let tags = vec![self.label().into()];
        WireRepr::new(AiPowMinerWire::SOURCE, AiPowMinerWire::VERSION, tags)
    }
}
