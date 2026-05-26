//! Kernel-side wire vocabulary the miner pokes the node with.
//!
//! Lifted verbatim from `crates/nockchain/src/mining.rs`. The
//! kernel-side Hoon in `hoon/apps/dumbnet/lib/miner.hoon` (compiled
//! to `assets/miner.jam`) speaks exactly these labels with
//! `SOURCE = "miner"` and `VERSION = 1`. **Renaming any variant or
//! changing the source/version silently breaks block production.**

use nockapp::nockapp::wire::Wire;
use nockapp::wire::WireRepr;

pub enum MiningWire {
    Mined,
    Candidate,
    SetPubKey,
    Enable,
}

impl MiningWire {
    pub fn verb(&self) -> &'static str {
        match self {
            MiningWire::Mined => "mined",
            MiningWire::SetPubKey => "setpubkey",
            MiningWire::Candidate => "candidate",
            MiningWire::Enable => "enable",
        }
    }
}

impl Wire for MiningWire {
    const VERSION: u64 = 1;
    const SOURCE: &'static str = "miner";

    fn to_wire(&self) -> WireRepr {
        let tags = vec![self.verb().into()];
        WireRepr::new(MiningWire::SOURCE, MiningWire::VERSION, tags)
    }
}
