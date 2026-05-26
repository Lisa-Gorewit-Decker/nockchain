use std::time::Duration;

use ibig::UBig;
use nockapp::noun::slab::NounSlab;
use nockapp::noun::IntoSlab;
use nockvm::noun::{Atom, Noun, NounAllocator, T};
use noun_serde::NounEncode;
use tracing::info;

pub const DEFAULT_FAKENET_POW_LEN: u64 = 2;
pub const DEFAULT_FAKENET_LOG_DIFFICULTY: u64 = 1;
pub const FAKENET_V1_PHASE: u64 = 1;
pub const FAKENET_BYTHOS_PHASE: u64 = 1;
pub const FAKENET_BASE_FEE: u64 = 128;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, NounEncode)]
pub struct Seconds(pub u64);

impl Seconds {
    pub fn new(seconds: u64) -> Self {
        Self(seconds)
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }

    pub fn to_duration(&self) -> Duration {
        Duration::from_secs(self.0)
    }
}

impl From<u64> for Seconds {
    fn from(seconds: u64) -> Self {
        Self(seconds)
    }
}

impl TryFrom<Duration> for Seconds {
    type Error = &'static str;

    fn try_from(duration: Duration) -> Result<Self, Self::Error> {
        if duration.subsec_nanos() != 0 {
            return Err("Duration must be whole seconds only");
        }
        Ok(Self(duration.as_secs()))
    }
}

impl From<Seconds> for Duration {
    fn from(seconds: Seconds) -> Self {
        Duration::from_secs(seconds.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, NounEncode)]
pub struct NoteDataConstraints {
    pub max_size: u64,
    pub min_fee: u64,
}

/// Shared blockchain constants model used for explicit kernel constants pokes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockchainConstants {
    pub max_block_size: u64,
    pub blocks_per_epoch: u64,
    pub target_epoch_duration: Seconds,
    pub update_candidate_timestamp_interval: Seconds,
    pub max_future_timestamp: Seconds,
    pub min_past_blocks: u64,
    pub genesis_target_atom: UBig,
    pub max_target_atom: UBig,
    pub check_pow_flag: bool,
    pub coinbase_timelock_min: u64,
    pub pow_len: u64,
    pub max_coinbase_split: u64,
    pub first_month_coinbase_min: u64,
    pub v1_phase: u64,
    pub bythos_phase: u64,
    pub note_data: NoteDataConstraints,
    pub base_fee: u64,
    pub input_fee_divisor: u64,
    pub asert_phase: u64,
    pub asert_anchor_height: u64,
    pub asert_anchor_target_atom: UBig,
    pub asert_ideal_block_time: u64,
    pub asert_half_life: u64,
}

impl BlockchainConstants {
    pub const DEFAULT_MAX_BLOCK_SIZE: u64 = 8_000_000;
    pub const DEFAULT_BLOCKS_PER_EPOCH: u64 = 2_016;
    pub const DEFAULT_TARGET_EPOCH_DURATION: u64 = 1_209_600;
    pub const DEFAULT_UPDATE_CANDIDATE_TIMESTAMP_INTERVAL_SECS: u64 = 300;
    pub const DEFAULT_MAX_FUTURE_TIMESTAMP: u64 = 7_200;
    pub const DEFAULT_GENESIS_TARGET_ATOM: &str =
        "0x3ffffffec0000003bffffff88000000b3ffffff34000000b3ffffff880000003bffffffec0000";
    pub const DEFAULT_MAX_TIP5_ATOM: &str =
        "0xfffffffb0000000effffffe20000002cffffffcd0000002cffffffe20000000efffffffb00000000";
    pub const DEFAULT_MIN_PAST_BLOCKS: u64 = 11;
    pub const DEFAULT_CHECK_POW_FLAG: bool = true;
    pub const DEFAULT_COINBASE_TIMELOCK_MIN: u64 = 100;
    pub const DEFAULT_POW_LEN: u64 = 64;
    pub const DEFAULT_MAX_COINBASE_SPLIT: u64 = 2;
    pub const DEFAULT_FIRST_MONTH_COINBASE_MIN: u64 = 4_383;
    pub const DEFAULT_V1_PHASE: u64 = 39_000;
    pub const DEFAULT_BYTHOS_PHASE: u64 = 54_000;
    pub const DEFAULT_NOTE_DATA_MAX_SIZE: u64 = 2_048;
    pub const DEFAULT_NOTE_DATA_MIN_FEE: u64 = 256;
    pub const DEFAULT_BASE_FEE: u64 = 16_384;
    pub const DEFAULT_INPUT_FEE_DIVISOR: u64 = 4;
    pub const DEFAULT_ASERT_PHASE: u64 = 65_500;
    pub const DEFAULT_ASERT_ANCHOR_HEIGHT: u64 = 65_499;
    pub const DEFAULT_ASERT_ANCHOR_TARGET_BEX: u64 = 291;
    pub const DEFAULT_ASERT_IDEAL_BLOCK_TIME: u64 = 150;
    pub const DEFAULT_ASERT_HALF_LIFE: u64 = 43_200;

    pub fn new() -> Self {
        let max_target_atom = UBig::from_str_with_radix_prefix(Self::DEFAULT_MAX_TIP5_ATOM)
            .expect("Failed to parse max tip5 atom");
        let genesis_target_atom =
            UBig::from_str_with_radix_prefix(Self::DEFAULT_GENESIS_TARGET_ATOM)
                .expect("Failed to parse genesis target atom");
        let asert_anchor_target_atom =
            UBig::from(1u64) << (Self::DEFAULT_ASERT_ANCHOR_TARGET_BEX as usize);

        Self {
            max_block_size: Self::DEFAULT_MAX_BLOCK_SIZE,
            blocks_per_epoch: Self::DEFAULT_BLOCKS_PER_EPOCH,
            target_epoch_duration: Self::DEFAULT_TARGET_EPOCH_DURATION.into(),
            update_candidate_timestamp_interval:
                Self::DEFAULT_UPDATE_CANDIDATE_TIMESTAMP_INTERVAL_SECS.into(),
            max_future_timestamp: Self::DEFAULT_MAX_FUTURE_TIMESTAMP.into(),
            min_past_blocks: Self::DEFAULT_MIN_PAST_BLOCKS,
            genesis_target_atom,
            max_target_atom,
            check_pow_flag: Self::DEFAULT_CHECK_POW_FLAG,
            coinbase_timelock_min: Self::DEFAULT_COINBASE_TIMELOCK_MIN,
            pow_len: Self::DEFAULT_POW_LEN,
            max_coinbase_split: Self::DEFAULT_MAX_COINBASE_SPLIT,
            first_month_coinbase_min: Self::DEFAULT_FIRST_MONTH_COINBASE_MIN,
            v1_phase: Self::DEFAULT_V1_PHASE,
            bythos_phase: Self::DEFAULT_BYTHOS_PHASE,
            note_data: NoteDataConstraints {
                max_size: Self::DEFAULT_NOTE_DATA_MAX_SIZE,
                min_fee: Self::DEFAULT_NOTE_DATA_MIN_FEE,
            },
            base_fee: Self::DEFAULT_BASE_FEE,
            input_fee_divisor: Self::DEFAULT_INPUT_FEE_DIVISOR,
            asert_phase: Self::DEFAULT_ASERT_PHASE,
            asert_anchor_height: Self::DEFAULT_ASERT_ANCHOR_HEIGHT,
            asert_anchor_target_atom,
            asert_ideal_block_time: Self::DEFAULT_ASERT_IDEAL_BLOCK_TIME,
            asert_half_life: Self::DEFAULT_ASERT_HALF_LIFE,
        }
    }

    pub fn with_genesis_target_atom_bex(mut self, bex: u128) -> Self {
        let difficulty = UBig::from((1 << bex) as u128);
        self.genesis_target_atom = self.max_target_atom.clone() / difficulty;
        info!("Genesis target atom set to {}", self.genesis_target_atom);
        self
    }

    pub fn with_update_candidate_timestamp_interval(mut self, interval_secs: Seconds) -> Self {
        self.update_candidate_timestamp_interval = interval_secs;
        self
    }

    pub fn with_pow_len(mut self, pow_len: u64) -> Self {
        self.pow_len = pow_len;
        self
    }

    pub fn with_v1_phase(mut self, v1_phase: u64) -> Self {
        self.v1_phase = v1_phase;
        self
    }

    pub fn with_bythos_phase(mut self, bythos_phase: u64) -> Self {
        self.bythos_phase = bythos_phase;
        self
    }

    pub fn with_first_month_coinbase_min(mut self, coinbase_min: u64) -> Self {
        self.first_month_coinbase_min = coinbase_min;
        self
    }

    pub fn with_coinbase_timelock_min(mut self, coinbase_min: u64) -> Self {
        self.coinbase_timelock_min = coinbase_min;
        self
    }

    pub fn with_base_fee(mut self, base_fee: u64) -> Self {
        self.base_fee = base_fee;
        self
    }

    pub fn with_asert_phase(mut self, asert_phase: u64) -> Self {
        self.asert_phase = asert_phase;
        self
    }

    pub fn with_asert_anchor_height(mut self, asert_anchor_height: u64) -> Self {
        self.asert_anchor_height = asert_anchor_height;
        self
    }

    pub fn with_asert_anchor_target_atom(mut self, asert_anchor_target_atom: UBig) -> Self {
        self.asert_anchor_target_atom = asert_anchor_target_atom;
        self
    }

    pub fn with_asert_anchor_target_bex(mut self, bex: u64) -> Self {
        self.asert_anchor_target_atom = UBig::from(1u64) << (bex as usize);
        self
    }

    fn to_blockchain_constants_v0_fields<A: NounAllocator>(&self, allocator: &mut A) -> Vec<Noun> {
        let max_block_size = Atom::new(allocator, self.max_block_size).as_noun();
        let blocks_per_epoch = Atom::new(allocator, self.blocks_per_epoch).as_noun();
        let target_epoch_duration = self.target_epoch_duration.to_noun(allocator);
        let update_candidate_timestamp_interval_atoms =
            UBig::from(self.update_candidate_timestamp_interval.0) << 64;
        let update_candidate_timestamp_interval =
            Atom::from_ubig(allocator, &update_candidate_timestamp_interval_atoms).as_noun();
        let max_future_timestamp = self.max_future_timestamp.to_noun(allocator);
        let min_past_blocks = Atom::new(allocator, self.min_past_blocks).as_noun();
        let genesis_target_atom = Atom::from_ubig(allocator, &self.genesis_target_atom).as_noun();
        let max_target_atom = Atom::from_ubig(allocator, &self.max_target_atom).as_noun();
        let check_pow_flag = self.check_pow_flag.to_noun(allocator);
        let coinbase_timelock_min = Atom::new(allocator, self.coinbase_timelock_min).as_noun();
        let pow_len = Atom::new(allocator, self.pow_len).as_noun();
        let max_coinbase_split = Atom::new(allocator, self.max_coinbase_split).as_noun();
        let first_month_coinbase_min =
            Atom::new(allocator, self.first_month_coinbase_min).as_noun();

        vec![
            max_block_size, blocks_per_epoch, target_epoch_duration,
            update_candidate_timestamp_interval, max_future_timestamp, min_past_blocks,
            genesis_target_atom, max_target_atom, check_pow_flag, coinbase_timelock_min, pow_len,
            max_coinbase_split, first_month_coinbase_min,
        ]
    }
}

impl Default for BlockchainConstants {
    fn default() -> Self {
        Self::new()
    }
}

impl NounEncode for BlockchainConstants {
    fn to_noun<A: NounAllocator>(&self, allocator: &mut A) -> Noun {
        // blockchain-constants:v1 from `hoon/common/tx-engine-1.hoon:208–263`.
        // The Hoon $: is an 11-slot record:
        //
        //     $:  v1-phase=@                       ::  slot 1
        //         bythos-phase=@                   ::  slot 2
        //         data=[max-size=@ min-fee=@]      ::  slot 3 (2-cell)
        //         base-fee=@                       ::  slot 4
        //         input-fee-divisor=@              ::  slot 5
        //         blockchain-constants:v0          ::  slot 6 (13-tuple sub-cell)
        //         asert-phase=@                    ::  slot 7
        //         asert-anchor-height=@            ::  slot 8
        //         asert-anchor-target-atom=@       ::  slot 9
        //         asert-ideal-block-time=@         ::  slot 10
        //         asert-half-life=@                ::  slot 11
        //     ==
        //
        // Right-folded into a single noun:
        //   [v1-phase [bythos-phase [data [base-fee [input-fee-divisor
        //    [v0-13-tuple [asert-phase [asert-anchor-height
        //    [asert-anchor-target [asert-ideal asert-half]]]]]]]]]]
        //
        // Confirmed empirically by the wallet's `PlannerBlockchainConstantsNoun`
        // (crates/nockchain-wallet/src/create_tx.rs), which decodes 5 v1-prefix
        // slots followed by `_legacy_constants = [v0_constants asert_block]` —
        // i.e. the right-fold of slots 6..11 where slot 6 is the v0 sub-cell.
        //
        // History: the pre-rebuild legacy `assets/dumb.jam` (commit fccde2a,
        // 2025-11-20, PRE-Bythos and PRE-ASERT) expected a 4-tuple
        // [v1_phase note_data base_fee v0_constants]. Commit 794dfa9 patched
        // this encoder to that shape to unblock fakenet on the legacy jam.
        // After rebuilding the jam from current Hoon source (commit f495f43
        // %pow tagged-union work), the kernel now expects the 11-slot shape
        // again — this revert restores it.
        let v1_phase = Atom::new(allocator, self.v1_phase).as_noun();
        let bythos_phase = Atom::new(allocator, self.bythos_phase).as_noun();
        let note_data = self.note_data.to_noun(allocator);
        let base_fee = Atom::new(allocator, self.base_fee).as_noun();
        let input_fee_divisor = Atom::new(allocator, self.input_fee_divisor).as_noun();
        let v0_fields = self.to_blockchain_constants_v0_fields(allocator);
        let v0_constants = T(allocator, &v0_fields);
        let asert_phase = Atom::new(allocator, self.asert_phase).as_noun();
        let asert_anchor_height = Atom::new(allocator, self.asert_anchor_height).as_noun();
        let asert_anchor_target_atom =
            Atom::from_ubig(allocator, &self.asert_anchor_target_atom).as_noun();
        let asert_ideal_block_time = Atom::new(allocator, self.asert_ideal_block_time).as_noun();
        let asert_half_life = Atom::new(allocator, self.asert_half_life).as_noun();

        T(
            allocator,
            &[
                v1_phase,
                bythos_phase,
                note_data,
                base_fee,
                input_fee_divisor,
                v0_constants,
                asert_phase,
                asert_anchor_height,
                asert_anchor_target_atom,
                asert_ideal_block_time,
                asert_half_life,
            ],
        )
    }
}

impl IntoSlab for BlockchainConstants {
    fn into_slab(self) -> NounSlab {
        let mut slab = NounSlab::new();
        let noun = self.to_noun(&mut slab);
        slab.set_root(noun);
        slab
    }
}

pub fn fakenet_blockchain_constants(pow_len: u64, target_bex: u64) -> BlockchainConstants {
    // Fakenet starts from the mainnet defaults and overrides only the fields
    // that are intentionally relaxed for local testing.
    //
    // `update_candidate_timestamp_interval = 5s` (not 5 minutes) so the
    // kernel's post-poke `update-candidate-block` quickly emits a `%mine`
    // effect after the miner pokes `enable-mining`. With the mainnet
    // default of 5 minutes, the first candidate emission lags ~5min,
    // which is impractical for a fakenet smoke test.
    BlockchainConstants::new()
        .with_update_candidate_timestamp_interval(Seconds(5))
        .with_pow_len(pow_len)
        .with_genesis_target_atom_bex(target_bex as u128)
        .with_first_month_coinbase_min(0)
        .with_coinbase_timelock_min(1)
        .with_base_fee(FAKENET_BASE_FEE)
        .with_v1_phase(FAKENET_V1_PHASE)
        .with_bythos_phase(FAKENET_BYTHOS_PHASE)
}

pub fn default_fakenet_blockchain_constants() -> BlockchainConstants {
    fakenet_blockchain_constants(DEFAULT_FAKENET_POW_LEN, DEFAULT_FAKENET_LOG_DIFFICULTY)
}

#[cfg(test)]
mod tests {
    use ibig::UBig;

    use super::*;

    fn tuple_len(noun: Noun) -> usize {
        let mut len = 0;
        let mut cur = noun;
        loop {
            if let Ok(cell) = cur.as_cell() {
                len += 1;
                cur = cell.tail();
            } else {
                len += 1;
                break;
            }
        }
        len
    }

    /// Asserts that `BlockchainConstants::to_noun()` produces the 11-slot
    /// right-fold matching `blockchain-constants:v1` in current Hoon source
    /// (`hoon/common/tx-engine-1.hoon:208–263`):
    ///
    ///     [v1-phase bythos-phase data base-fee input-fee-divisor
    ///      v0-sub-cell asert-phase asert-anchor-height
    ///      asert-anchor-target-atom asert-ideal-block-time asert-half-life]
    ///
    /// Slot 3 (`data`) is a `[max-size min-fee]` sub-cell. Slot 6
    /// (`blockchain-constants:v0`) is a 13-atom right-fold sub-cell.
    /// Regression test for the jam rebuild after commit f495f43 (%pow
    /// tagged-union work) which exposed the schema drift between current
    /// Hoon source and the previous legacy-jam-targeting 4-tuple encoder.
    #[test]
    fn to_noun_matches_current_hoon_source_shape() {
        let constants = default_fakenet_blockchain_constants();
        let mut slab: NounSlab = NounSlab::new();
        let noun = constants.to_noun(&mut slab);
        slab.set_root(noun);
        let root = unsafe { *slab.root() };

        // slot 1: v1_phase (fakenet = 1)
        let c1 = root.as_cell().expect("root is a cell");
        assert_eq!(
            c1.head().as_atom().unwrap().as_u64().unwrap(),
            FAKENET_V1_PHASE,
            "slot 1 = v1_phase"
        );

        // slot 2: bythos_phase (fakenet = 1)
        let c2 = c1.tail().as_cell().expect("c2 cell");
        assert_eq!(
            c2.head().as_atom().unwrap().as_u64().unwrap(),
            FAKENET_BYTHOS_PHASE,
            "slot 2 = bythos_phase"
        );

        // slot 3: data sub-cell [max-size min-fee]
        let c3 = c2.tail().as_cell().expect("c3 cell");
        let nd = c3.head().as_cell().expect("data sub-cell");
        assert_eq!(
            nd.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_NOTE_DATA_MAX_SIZE,
            "data.max_size"
        );
        assert_eq!(
            nd.tail().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_NOTE_DATA_MIN_FEE,
            "data.min_fee"
        );

        // slot 4: base_fee
        let c4 = c3.tail().as_cell().expect("c4 cell");
        assert_eq!(
            c4.head().as_atom().unwrap().as_u64().unwrap(),
            FAKENET_BASE_FEE,
            "slot 4 = base_fee"
        );

        // slot 5: input_fee_divisor
        let c5 = c4.tail().as_cell().expect("c5 cell");
        assert_eq!(
            c5.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_INPUT_FEE_DIVISOR,
            "slot 5 = input_fee_divisor"
        );

        // slot 6: v0 sub-cell (13-atom right-fold)
        let c6 = c5.tail().as_cell().expect("c6 cell");
        let v0 = c6.head();
        assert_eq!(
            tuple_len(v0),
            13,
            "slot 6 = v0 must be a 13-atom right-fold"
        );
        let v0c = v0.as_cell().expect("v0 is a cell");
        assert_eq!(
            v0c.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_MAX_BLOCK_SIZE,
            "v0 head = max-block-size"
        );

        // slot 7: asert_phase
        let c7 = c6.tail().as_cell().expect("c7 cell");
        assert_eq!(
            c7.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_ASERT_PHASE,
            "slot 7 = asert_phase"
        );

        // slot 8: asert_anchor_height
        let c8 = c7.tail().as_cell().expect("c8 cell");
        assert_eq!(
            c8.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_ASERT_ANCHOR_HEIGHT,
            "slot 8 = asert_anchor_height"
        );

        // slot 9: asert_anchor_target_atom (big atom, just check it's an atom)
        let c9 = c8.tail().as_cell().expect("c9 cell");
        assert!(c9.head().is_atom(), "slot 9 = asert_anchor_target_atom");

        // slot 10: asert_ideal_block_time
        let c10 = c9.tail().as_cell().expect("c10 cell");
        assert_eq!(
            c10.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_ASERT_IDEAL_BLOCK_TIME,
            "slot 10 = asert_ideal_block_time"
        );

        // slot 11: asert_half_life (last; sits in c10.tail as an atom)
        assert_eq!(
            c10.tail().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_ASERT_HALF_LIFE,
            "slot 11 = asert_half_life (right-fold terminal)"
        );
    }

    #[test]
    fn blockchain_constants_new_defaults_are_valid() {
        let constants = BlockchainConstants::new();

        assert_eq!(
            constants.max_block_size, 8_000_000,
            "max-block-size mismatch"
        );
        assert_eq!(
            constants.blocks_per_epoch, 2_016,
            "blocks-per-epoch mismatch"
        );
        assert_eq!(
            constants.target_epoch_duration,
            Seconds::new(14 * 24 * 60 * 60),
            "target-epoch-duration mismatch",
        );
        assert_eq!(
            constants.update_candidate_timestamp_interval,
            Seconds::new(5 * 60),
            "update-candidate-interval mismatch",
        );
        assert_eq!(
            constants.max_future_timestamp,
            Seconds::new(60 * 120),
            "max-future-timestamp mismatch",
        );
        assert_eq!(constants.min_past_blocks, 11, "min-past-blocks mismatch");

        let max_tip5_atom =
            UBig::from_str_with_radix_prefix(BlockchainConstants::DEFAULT_MAX_TIP5_ATOM)
                .expect("parse max tip5 atom");
        assert_eq!(
            constants.max_target_atom, max_tip5_atom,
            "max-target-atom mismatch",
        );

        let expected_genesis_target = &max_tip5_atom / (UBig::from(1u64) << 14);
        assert_eq!(
            constants.genesis_target_atom, expected_genesis_target,
            "genesis-target-atom mismatch",
        );

        assert!(constants.check_pow_flag, "check-pow-flag mismatch");
        assert_eq!(
            constants.coinbase_timelock_min, 100,
            "coinbase-timelock-min mismatch"
        );
        assert_eq!(constants.pow_len, 64, "pow-len mismatch");
        assert_eq!(
            constants.max_coinbase_split, 2,
            "max-coinbase-split mismatch"
        );
        assert_eq!(
            constants.first_month_coinbase_min, 4_383,
            "first-month-coinbase-min mismatch",
        );
        assert_eq!(constants.v1_phase, 39_000, "v1-phase mismatch");
        assert_eq!(constants.bythos_phase, 54_000, "bythos-phase mismatch");
        assert_eq!(
            constants.note_data,
            NoteDataConstraints {
                max_size: 2_048,
                min_fee: 256,
            },
            "note-data mismatch",
        );
        assert_eq!(constants.base_fee, 16_384, "base-fee mismatch");
        assert_eq!(constants.input_fee_divisor, 4, "input-fee-divisor mismatch");
        assert_eq!(constants.asert_phase, 65_500, "asert-phase mismatch");
        assert_eq!(
            constants.asert_anchor_height, 65_499,
            "asert-anchor-height mismatch"
        );
        assert_eq!(
            constants.asert_anchor_target_atom,
            UBig::from(1u64) << 291,
            "asert-anchor-target-atom mismatch"
        );
        assert_eq!(
            constants.asert_ideal_block_time, 150,
            "asert-ideal-block-time mismatch"
        );
        assert_eq!(
            constants.asert_half_life, 43_200,
            "asert-half-life mismatch"
        );
    }

    #[test]
    fn fakenet_blockchain_constants_activate_early_phases() {
        let constants = default_fakenet_blockchain_constants();

        assert_eq!(
            constants.pow_len, DEFAULT_FAKENET_POW_LEN,
            "pow-len mismatch"
        );
        assert_eq!(
            constants.coinbase_timelock_min, 1,
            "coinbase-timelock-min mismatch"
        );
        assert_eq!(
            constants.first_month_coinbase_min, 0,
            "first-month-coinbase-min mismatch",
        );
        assert_eq!(constants.base_fee, FAKENET_BASE_FEE, "base-fee mismatch");
        assert_eq!(constants.v1_phase, FAKENET_V1_PHASE, "v1-phase mismatch");
        assert_eq!(
            constants.bythos_phase, FAKENET_BYTHOS_PHASE,
            "bythos-phase mismatch"
        );
    }

    #[test]
    fn with_v1_phase_overrides_default() {
        let constants = BlockchainConstants::new().with_v1_phase(54_321);

        assert_eq!(constants.v1_phase, 54_321);
    }

    #[test]
    fn with_asert_overrides_default() {
        let constants = BlockchainConstants::new()
            .with_asert_phase(10)
            .with_asert_anchor_height(9)
            .with_asert_anchor_target_bex(2);

        assert_eq!(constants.asert_phase, 10);
        assert_eq!(constants.asert_anchor_height, 9);
        assert_eq!(constants.asert_anchor_target_atom, UBig::from(1u64) << 2);
    }

    /// Asserts mainnet defaults (`BlockchainConstants::new()`) round-trip
    /// through the 11-slot current-Hoon-source shape (see
    /// [`to_noun_matches_current_hoon_source_shape`] for the slot map).
    /// This is the mainnet-side companion to the fakenet test.
    #[test]
    fn mainnet_defaults_encode_in_current_hoon_source_shape() {
        let slab = BlockchainConstants::new().into_slab();
        let root = unsafe { *slab.root() };

        // slot 1: v1_phase = 39000
        let c1 = root.as_cell().expect("outer cell");
        assert_eq!(
            c1.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_V1_PHASE,
            "slot 1 = v1_phase"
        );

        // slot 2: bythos_phase = 54000
        let c2 = c1.tail().as_cell().expect("c2 cell");
        assert_eq!(
            c2.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_BYTHOS_PHASE,
            "slot 2 = bythos_phase"
        );

        // slot 3: data sub-cell [max-size min-fee]
        let c3 = c2.tail().as_cell().expect("c3 cell");
        let nd = c3.head().as_cell().expect("data sub-cell");
        assert_eq!(
            nd.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_NOTE_DATA_MAX_SIZE,
        );
        assert_eq!(
            nd.tail().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_NOTE_DATA_MIN_FEE,
        );

        // slot 4: base_fee
        let c4 = c3.tail().as_cell().expect("c4 cell");
        assert_eq!(
            c4.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_BASE_FEE,
            "slot 4 = base_fee"
        );

        // slot 5: input_fee_divisor
        let c5 = c4.tail().as_cell().expect("c5 cell");
        assert_eq!(
            c5.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_INPUT_FEE_DIVISOR,
            "slot 5 = input_fee_divisor"
        );

        // slot 6: v0 sub-cell (13 atoms)
        let c6 = c5.tail().as_cell().expect("c6 cell");
        let v0 = c6.head();
        assert_eq!(
            tuple_len(v0),
            13,
            "slot 6 = v0 must be a 13-atom right-fold"
        );
        let v0c = v0.as_cell().expect("v0 cell");
        assert_eq!(
            v0c.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_MAX_BLOCK_SIZE,
            "v0 head = max-block-size"
        );

        // slots 7..11: asert fields
        let c7 = c6.tail().as_cell().expect("c7 cell");
        assert_eq!(
            c7.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_ASERT_PHASE,
            "slot 7 = asert_phase"
        );
        let c8 = c7.tail().as_cell().expect("c8 cell");
        assert_eq!(
            c8.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_ASERT_ANCHOR_HEIGHT,
            "slot 8 = asert_anchor_height"
        );
        let c9 = c8.tail().as_cell().expect("c9 cell");
        assert!(c9.head().is_atom(), "slot 9 = asert_anchor_target_atom");
        let c10 = c9.tail().as_cell().expect("c10 cell");
        assert_eq!(
            c10.head().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_ASERT_IDEAL_BLOCK_TIME,
            "slot 10 = asert_ideal_block_time"
        );
        assert_eq!(
            c10.tail().as_atom().unwrap().as_u64().unwrap(),
            BlockchainConstants::DEFAULT_ASERT_HALF_LIFE,
            "slot 11 = asert_half_life"
        );
    }
}
