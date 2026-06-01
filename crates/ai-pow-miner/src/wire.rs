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
//! 1. The run loop obtains the canonical recursive AI-PoW certificate noun.
//! 2. It pokes the node over gRPC via
//!    [`nockchain_mining_common::NodeClient::poke_wire`] with
//!    `AiPowMinerWire::Mined.to_wire()`.
//! 3. The payload is the consensus command `[%command %pow %ai-pow cert]`;
//!    the kernel persists `cert` in the candidate block's `pow` slot.

use nockapp::nockapp::wire::{Wire, WireRepr};

pub enum AiPowMinerWire {
    /// Driver → node: enable / disable mining.
    Enable,
    /// Kernel-internal: a new candidate puzzle.
    Candidate,
    /// Driver → node: canonical recursive certificate. Payload (v1):
    /// `[%command %pow %ai-pow cert=ai-pow-certificate]`.
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
