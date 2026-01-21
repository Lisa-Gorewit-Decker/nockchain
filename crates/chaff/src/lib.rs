use bytes::Bytes;
use either::Either;
use habit::{BitReader, BitWriter};
use intmap::IntMap;
use nockapp::noun::slab::{CueError, Jammer, NounMap, NounSlab};
use nockvm::ext::AtomExt;
use nockvm::noun::{Atom, Cell, CellMemory, DirectAtom, Noun, NounSpace, D};
use nockvm::serialization::{met0_u64_to_usize, met0_usize};

pub struct Chaff;

const MAX_USIZE_BITS: usize = usize::BITS as usize;

impl Jammer for Chaff {
    fn cue(slab: &mut NounSlab<Self>, bytes: Bytes) -> Result<Noun, CueError> {
        fn rub_backref(reader: &mut BitReader) -> Result<usize, CueError> {
            let zeros = reader.read_unary().ok_or(CueError::TruncatedBuffer)?;
            if zeros == 0 {
                return Ok(0);
            }
            if zeros > MAX_USIZE_BITS {
                return Err(CueError::BackrefTooBig);
            }
            let size_low = if zeros > 1 {
                reader
                    .read_bits_to_usize(zeros - 1)
                    .ok_or(CueError::TruncatedBuffer)?
            } else {
                0
            };
            let bit_count = (1usize << (zeros - 1)) | size_low;
            if bit_count > MAX_USIZE_BITS {
                return Err(CueError::BackrefTooBig);
            }
            reader
                .read_bits_to_usize(bit_count)
                .ok_or(CueError::TruncatedBuffer)
        }

        fn rub_atom(slab: &mut NounSlab<Chaff>, reader: &mut BitReader) -> Result<Atom, CueError> {
            let zeros = reader.read_unary().ok_or(CueError::TruncatedBuffer)?;
            if zeros == 0 {
                return unsafe { Ok(DirectAtom::new_unchecked(0).as_atom()) };
            }
            if zeros > MAX_USIZE_BITS {
                return Err(CueError::TruncatedBuffer);
            }
            let size_low = if zeros > 1 {
                reader
                    .read_bits_to_usize(zeros - 1)
                    .ok_or(CueError::TruncatedBuffer)?
            } else {
                0
            };
            let bit_count = (1usize << (zeros - 1)) | size_low;
            if bit_count < 64 {
                let value = reader
                    .read_bits_to_usize(bit_count)
                    .ok_or(CueError::TruncatedBuffer)? as u64;
                unsafe { Ok(DirectAtom::new_unchecked(value).as_atom()) }
            } else {
                let byte_len = (bit_count + 7) >> 3;
                let mut buffer = vec![0u8; byte_len];
                reader
                    .read_bits_to_bytes(&mut buffer, bit_count)
                    .ok_or(CueError::TruncatedBuffer)?;
                Ok(<Atom as AtomExt>::from_bytes(slab, &buffer))
            }
        }

        enum CueStackEntry {
            DestinationPointer(*mut Noun),
            BackRef(u64, *const Noun),
        }

        let mut reader = BitReader::new(bytes);
        let mut result = D(0);
        let mut stack = vec![CueStackEntry::DestinationPointer(&mut result)];
        let mut backrefs: IntMap<u64, Noun> = IntMap::new();

        while let Some(entry) = stack.pop() {
            match entry {
                CueStackEntry::DestinationPointer(dest) => {
                    let backref_pos = reader.position() as u64;
                    let tag = reader.read_bit().ok_or(CueError::TruncatedBuffer)?;
                    if !tag {
                        let atom = rub_atom(slab, &mut reader)?;
                        unsafe {
                            *dest = atom.as_noun();
                        }
                        backrefs.insert(backref_pos, unsafe { *dest });
                    } else {
                        let second = reader.read_bit().ok_or(CueError::TruncatedBuffer)?;
                        if second {
                            let backref = rub_backref(&mut reader)? as u64;
                            let noun =
                                backrefs.get(backref).copied().ok_or(CueError::BadBackref)?;
                            unsafe {
                                *dest = noun;
                            }
                        } else {
                            let (cell, cell_mem): (Cell, *mut CellMemory) =
                                unsafe { Cell::new_raw_mut(slab) };
                            unsafe {
                                *dest = cell.as_noun();
                            }
                            stack.push(CueStackEntry::BackRef(backref_pos, dest as *const Noun));
                            unsafe {
                                stack
                                    .push(CueStackEntry::DestinationPointer(&mut (*cell_mem).tail));
                                stack
                                    .push(CueStackEntry::DestinationPointer(&mut (*cell_mem).head));
                            }
                        }
                    }
                }
                CueStackEntry::BackRef(pos, noun_ptr) => {
                    backrefs.insert(pos, unsafe { *noun_ptr });
                }
            }
        }

        slab.set_root(result);
        Ok(result)
    }

    fn jam(noun: Noun, space: &NounSpace) -> Bytes {
        fn mat_backref_fast(writer: &mut BitWriter, backref: usize) {
            if backref == 0 {
                writer.write_bits_from_value(0b111, 3); // 1 1 1
                return;
            }
            let backref_sz = met0_u64_to_usize(backref as u64);
            let backref_sz_sz = met0_u64_to_usize(backref_sz as u64);
            // backref tag 1 1
            writer.write_bit(true);
            writer.write_bit(true);
            // write backref_sz_sz zeros
            writer.write_zeros(backref_sz_sz);
            // delimiter 1
            writer.write_bit(true);
            // write backref_sz_sz-1 bits of backref_sz (LSB first)
            writer.write_bits_from_value(backref_sz, backref_sz_sz - 1);
            // write backref bits (backref_sz bits)
            writer.write_bits_from_value(backref, backref_sz);
        }

        fn mat_atom_fast(writer: &mut BitWriter, atom: Atom, space: &NounSpace) {
            unsafe {
                if atom.as_noun().raw_equals(&D(0)) {
                    writer.write_bits_from_value(0b10, 2); // 0 1
                    return;
                }
            }
            let atom_sz = met0_usize(atom, space);
            let atom_sz_sz = met0_u64_to_usize(atom_sz as u64);
            writer.write_bit(false); // atom tag 0
            writer.write_zeros(atom_sz_sz); // size zeros
            writer.write_bit(true); // delimiter
                                    // write size bits (atom_sz_sz - 1)
            writer.write_bits_from_value(atom_sz, atom_sz_sz - 1);
            // write atom bits (little-endian order)
            writer.write_bits_from_le_bytes(atom.in_space(space).as_ne_bytes(), atom_sz);
        }

        // Main jam implementation ----------------------------------------
        let mut writer = BitWriter::new();
        let mut backref_map = NounMap::<usize>::new();
        let mut stack = vec![noun];
        while let Some(noun) = stack.pop() {
            if let Some(backref) = backref_map.get(noun, space) {
                // already seen this noun
                if let Ok(atom) = noun.in_space(space).as_atom() {
                    let atom = atom.atom();
                    if met0_u64_to_usize(*backref as u64) < met0_usize(atom, space) {
                        mat_backref_fast(&mut writer, *backref);
                    } else {
                        mat_atom_fast(&mut writer, atom, space);
                    }
                } else {
                    mat_backref_fast(&mut writer, *backref);
                }
            } else {
                backref_map.insert(noun, writer.bit_len(), space);
                match noun.in_space(space).as_either_atom_cell() {
                    Either::Left(atom) => {
                        mat_atom_fast(&mut writer, atom.atom(), space);
                    }
                    Either::Right(cell) => {
                        // cell tag 1 0
                        writer.write_bit(true);
                        writer.write_bit(false);
                        // push tail then head (LIFO stack)
                        stack.push(cell.tail().noun());
                        stack.push(cell.head().noun());
                    }
                }
            }
        }

        writer.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use habit::BitWriter;
    use nockapp::noun::slab::{slab_noun_equality, CueError, Jammer, NockJammer, NounSlab};
    use nockvm::ext::AtomExt;
    use nockvm::noun::{Atom, Cell, Noun, D, T};
    use quickcheck::{Arbitrary, Gen, TestResult};

    use super::Chaff;

    fn atom_from_bytes(slab: &mut NounSlab<Chaff>, bytes: &[u8]) -> Atom {
        <Atom as AtomExt>::from_bytes(slab, bytes)
    }

    fn build_shared_noun(slab: &mut NounSlab<Chaff>) -> Noun {
        let shared = D(42);
        let cell = Cell::new(slab, shared, shared).as_noun();
        T(slab, &[shared, cell, shared])
    }

    #[test]
    fn chaff_roundtrip_direct_atom() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let noun = D(0);
        slab.set_root(noun);
        let jammed = slab.jam();
        let mut cued_slab: NounSlab<Chaff> = NounSlab::new();
        let cued = cued_slab.cue_into(jammed).expect("cue should succeed");
        assert!(slab_noun_equality(unsafe { slab.root() }, &cued));
    }

    #[test]
    fn chaff_roundtrip_indirect_atom() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let bytes = vec![0xFFu8; 32];
        let atom = atom_from_bytes(&mut slab, &bytes);
        slab.set_root(atom.as_noun());
        let jammed = slab.jam();
        let mut cued_slab: NounSlab<Chaff> = NounSlab::new();
        let cued = cued_slab.cue_into(jammed).expect("cue should succeed");
        assert!(slab_noun_equality(unsafe { slab.root() }, &cued));
    }

    #[test]
    fn chaff_roundtrip_nested_cells() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let noun = T(&mut slab, &[D(1), D(2), D(3), D(4)]);
        slab.set_root(noun);
        let jammed = slab.jam();
        let mut cued_slab: NounSlab<Chaff> = NounSlab::new();
        let cued = cued_slab.cue_into(jammed).expect("cue should succeed");
        assert!(slab_noun_equality(unsafe { slab.root() }, &cued));
    }

    #[test]
    fn chaff_handles_shared_backrefs() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let noun = build_shared_noun(&mut slab);
        slab.set_root(noun);
        let jammed = slab.jam();
        let mut cued_slab: NounSlab<Chaff> = NounSlab::new();
        let cued = cued_slab.cue_into(jammed).expect("cue should succeed");
        assert!(slab_noun_equality(unsafe { slab.root() }, &cued));
    }

    #[test]
    fn chaff_jam_matches_nock_jam() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let noun = T(&mut slab, &[D(5), D(23), D(7)]);
        slab.set_root(noun);
        let chaff_jam = slab.jam();
        let mut nock_slab: NounSlab<NockJammer> = NounSlab::new();
        let copied = nock_slab.copy_into(noun);
        let nock_jam = NockJammer::jam(copied);
        assert_eq!(chaff_jam, nock_jam);
    }

    #[test]
    fn chaff_rejects_truncated_input() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let jammed = Bytes::from_static(&[0b1]);
        let result = slab.cue_into(jammed);
        assert!(matches!(result, Err(CueError::TruncatedBuffer)));
    }

    #[test]
    fn chaff_rejects_bad_backref() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let jammed = Bytes::from_static(&[0b0000_0111]);
        let result = slab.cue_into(jammed);
        assert!(matches!(result, Err(CueError::BadBackref)));
    }

    #[test]
    fn chaff_rejects_backref_before_definition() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let mut writer = BitWriter::new();
        writer.write_bit(true); // tag
        writer.write_bit(true); // backref
        writer.write_zeros(1); // size-of-size zeros
        writer.write_bit(true); // delimiter
        writer.write_bit(false); // backref size = 1
        writer.write_bit(true); // backref value = 1 (missing)
        let jammed = writer.into_bytes();
        let result = slab.cue_into(jammed);
        assert!(matches!(result, Err(CueError::BadBackref)));
    }

    #[test]
    fn chaff_rejects_backref_far_ahead() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let mut writer = BitWriter::new();
        writer.write_bit(true); // tag
        writer.write_bit(true); // backref
        writer.write_zeros(3); // size-of-size zeros
        writer.write_bit(true); // delimiter
        writer.write_bits_from_value(0b11, 2); // backref size = 4
        writer.write_bits_from_value(0b1010, 4); // backref = 10
        let jammed = writer.into_bytes();
        let result = slab.cue_into(jammed);
        assert!(matches!(result, Err(CueError::BadBackref)));
    }

    #[test]
    fn chaff_rejects_self_referential_backref() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let mut writer = BitWriter::new();
        writer.write_bit(true); // tag
        writer.write_bit(true); // backref
        writer.write_zeros(1); // size-of-size zeros
        writer.write_bit(true); // delimiter
        writer.write_bit(false); // backref size = 1
        writer.write_bit(false); // backref value = 0 (self)
        let jammed = writer.into_bytes();
        let result = slab.cue_into(jammed);
        assert!(matches!(result, Err(CueError::BadBackref)));
    }

    #[test]
    fn chaff_rejects_backref_too_big() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let mut writer = BitWriter::new();
        writer.write_bit(true); // tag
        writer.write_bit(true); // backref
        writer.write_zeros(usize::BITS as usize + 1);
        writer.write_bit(true); // delimiter
        let jammed = writer.into_bytes();
        let result = slab.cue_into(jammed);
        assert!(matches!(result, Err(CueError::BackrefTooBig)));
    }

    #[test]
    fn chaff_rejects_truncated_backref_size() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let mut writer = BitWriter::new();
        writer.write_bit(true); // tag
        writer.write_bit(true); // backref
        writer.write_zeros(2); // size-of-size zeros (expect 3 bits for size)
        writer.write_bit(true); // delimiter
        writer.write_bits_from_value(0b01, 2); // only two size bits
        let jammed = writer.into_bytes();
        let result = slab.cue_into(jammed);
        assert!(matches!(result, Err(CueError::TruncatedBuffer)));
    }

    #[test]
    fn chaff_rejects_atom_missing_size_bits() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let mut writer = BitWriter::new();
        writer.write_bit(false); // atom tag
        writer.write_bit(false); // first zero
        writer.write_bit(false); // second zero
                                 // zeros should be 3 (two zeros then delimiter), need 2 bits for size_low
                                 // but we end here - truncated
        let jammed = writer.into_bytes();
        assert!(slab.cue_into(jammed).is_err());
    }

    #[test]
    fn chaff_rejects_truncated_indirect_atom() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let mut writer = BitWriter::new();
        writer.write_bit(false); // atom tag
        writer.write_zeros(6); // zeros = 7, expect 6 bits for size_low
        writer.write_bit(true); // delimiter
        writer.write_bits_from_value(0b100000, 6); // size_low = 33 (bit_count = 65)
        writer.write_bits_from_value(0xFFFF, 16); // partial value (needs 65 bits)
        let jammed = writer.into_bytes();
        let result = slab.cue_into(jammed);
        assert!(matches!(result, Err(CueError::TruncatedBuffer)));
    }

    #[test]
    fn chaff_rejects_zero_atom_bad_encoding() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let mut writer = BitWriter::new();
        writer.write_bit(false); // atom tag
        writer.write_bit(false); // zeros = 0 (should be "0 1" encoding)
        writer.write_bit(false);
        let jammed = writer.into_bytes();
        let result = slab.cue_into(jammed);
        assert!(matches!(result, Err(CueError::TruncatedBuffer)));
    }

    #[derive(Clone, Debug)]
    struct SmallNoun {
        leaves: Vec<u16>,
    }

    impl Arbitrary for SmallNoun {
        fn arbitrary(g: &mut Gen) -> Self {
            let len = 1 + (usize::arbitrary(g) % 8);
            let mut leaves = Vec::with_capacity(len);
            for _ in 0..len {
                leaves.push(u16::arbitrary(g));
            }
            Self { leaves }
        }
    }

    fn build_list(slab: &mut NounSlab<Chaff>, leaves: &[u16]) -> Noun {
        let mut list = D(0);
        for value in leaves.iter().rev() {
            list = Cell::new(slab, D(*value as u64), list).as_noun();
        }
        list
    }

    #[test]
    fn chaff_roundtrip_list_fixture() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let noun = build_list(&mut slab, &[1, 2, 3, 4, 5]);
        slab.set_root(noun);
        let jammed = slab.jam();
        let mut cued: NounSlab<Chaff> = NounSlab::new();
        let cued_noun = cued.cue_into(jammed).expect("cue should succeed");
        assert!(slab_noun_equality(unsafe { slab.root() }, &cued_noun));
    }

    #[test]
    fn chaff_matches_nock_for_shared_fixture() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let noun = build_shared_noun(&mut slab);
        slab.set_root(noun);
        let chaff_jam = slab.jam();
        let mut nock_slab: NounSlab<NockJammer> = NounSlab::new();
        let copied = nock_slab.copy_into(noun);
        let nock_jam = NockJammer::jam(copied);
        assert_eq!(chaff_jam, nock_jam);
    }

    #[test]
    fn chaff_roundtrip_larger_atom_fixture() {
        let mut slab: NounSlab<Chaff> = NounSlab::new();
        let bytes = (0u8..64).collect::<Vec<_>>();
        let atom = atom_from_bytes(&mut slab, &bytes);
        let noun = T(&mut slab, &[D(7), atom.as_noun(), D(9)]);
        slab.set_root(noun);
        let jammed = slab.jam();
        let mut cued: NounSlab<Chaff> = NounSlab::new();
        let cued_noun = cued.cue_into(jammed).expect("cue should succeed");
        assert!(slab_noun_equality(unsafe { slab.root() }, &cued_noun));
    }

    quickcheck::quickcheck! {
        fn prop_chaff_roundtrip(small: SmallNoun) -> TestResult {
            let mut slab: NounSlab<Chaff> = NounSlab::new();
            let noun = build_list(&mut slab, &small.leaves);
            slab.set_root(noun);
            let jammed = slab.jam();
            let mut cued: NounSlab<Chaff> = NounSlab::new();
            let cued_noun = match cued.cue_into(jammed) {
                Ok(noun) => noun,
                Err(_) => return TestResult::failed(),
            };
            TestResult::from_bool(slab_noun_equality(unsafe { slab.root() }, &cued_noun))
        }

        fn prop_chaff_matches_nock(small: SmallNoun) -> TestResult {
            let mut slab: NounSlab<Chaff> = NounSlab::new();
            let noun = build_list(&mut slab, &small.leaves);
            slab.set_root(noun);
            let chaff_jam = slab.jam();
            let mut nock_slab: NounSlab<NockJammer> = NounSlab::new();
            let copied = nock_slab.copy_into(noun);
            let nock_jam = NockJammer::jam(copied);
            TestResult::from_bool(chaff_jam == nock_jam)
        }
    }

    quickcheck::quickcheck! {
        fn prop_chaff_handles_small_atoms(values: Vec<u64>) -> TestResult {
            let mut values = values;
            values.truncate(8);
            let mut slab: NounSlab<Chaff> = NounSlab::new();
            let mut list = D(0);
            for value in values.iter().rev() {
                let bounded = value & (nockvm::noun::DIRECT_MAX >> 1);
                list = Cell::new(&mut slab, D(bounded), list).as_noun();
            }
            slab.set_root(list);
            let jammed = slab.jam();
            let mut cued: NounSlab<Chaff> = NounSlab::new();
            let cued_noun = match cued.cue_into(jammed) {
                Ok(noun) => noun,
                Err(_) => return TestResult::failed(),
            };
            TestResult::from_bool(slab_noun_equality(unsafe { slab.root() }, &cued_noun))
        }
    }
}
