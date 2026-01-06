use std::str;

use bincode::{Decode, Encode};
use bytes::Bytes;
use either::Either::{Left, Right};
use intmap::IntMap;

use crate::interpreter::Error;
use crate::mem::{Arena, NockStack};
use crate::noun::{Atom, IndirectAtom, Noun, NounAllocator, D};
use crate::serialization::{cue, jam};

/// Convenience helpers for working with `Atom`.
pub trait AtomExt {
    fn from_bytes<A: NounAllocator>(allocator: &mut A, bytes: &[u8]) -> Atom;
    fn eq_bytes_with_arena<B: AsRef<[u8]>>(&self, bytes: B, arena: &Arena) -> bool;
    fn to_bytes_until_nul_with_arena(&self, arena: &Arena) -> std::result::Result<Vec<u8>, str::Utf8Error>;
    fn into_string_with_arena(self, arena: &Arena) -> std::result::Result<String, str::Utf8Error>;
}

impl AtomExt for Atom {
    fn from_bytes<A: NounAllocator>(allocator: &mut A, bytes: &[u8]) -> Atom {
        <IndirectAtom as IndirectAtomExt>::from_bytes(allocator, bytes)
    }

    fn eq_bytes_with_arena<B: AsRef<[u8]>>(&self, bytes: B, arena: &Arena) -> bool {
        let bytes_ref = bytes.as_ref();
        let atom_bytes = self.as_ne_bytes_with_arena(arena);
        if bytes_ref.len() > atom_bytes.len() {
            return false;
        }
        if bytes_ref.len() == atom_bytes.len() {
            return atom_bytes == bytes_ref;
        }
        if atom_bytes[bytes_ref.len()..].iter().any(|b| *b != 0) {
            return false;
        }
        &atom_bytes[0..bytes_ref.len()] == bytes_ref
    }

    fn to_bytes_until_nul_with_arena(&self, arena: &Arena) -> std::result::Result<Vec<u8>, str::Utf8Error> {
        str::from_utf8(self.as_ne_bytes_with_arena(arena))
            .map(|bytes| bytes.trim_end_matches('\0').as_bytes().to_vec())
    }

    fn into_string_with_arena(self, arena: &Arena) -> std::result::Result<String, str::Utf8Error> {
        str::from_utf8(self.as_ne_bytes_with_arena(arena)).map(|string| string.trim_end_matches('\0').to_string())
    }
}

/// Extension helpers for safely constructing indirect atoms.
pub trait IndirectAtomExt {
    fn from_bytes<A: NounAllocator>(allocator: &mut A, bytes: &[u8]) -> Atom;

    unsafe fn from_raw_parts<A: NounAllocator>(
        allocator: &mut A,
        size: usize,
        data: *const u8,
    ) -> Atom;
}

impl IndirectAtomExt for IndirectAtom {
    fn from_bytes<A: NounAllocator>(allocator: &mut A, bytes: &[u8]) -> Atom {
        unsafe { Self::from_raw_parts(allocator, bytes.len(), bytes.as_ptr()) }
    }

    unsafe fn from_raw_parts<A: NounAllocator>(
        allocator: &mut A,
        size: usize,
        data: *const u8,
    ) -> Atom {
        // Use normalize_as_atom_stack since new_raw_bytes creates stack-pointer form atoms
        Self::new_raw_bytes(allocator, size, data).normalize_as_atom_stack()
    }
}

/// Helpers for working with nouns directly.
pub trait NounExt {
    fn cue_bytes(stack: &mut NockStack, bytes: &Bytes) -> std::result::Result<Noun, Error>;
    fn cue_bytes_slice(stack: &mut NockStack, bytes: &[u8]) -> std::result::Result<Noun, Error>;
    fn jam_self(self, stack: &mut NockStack) -> JammedNoun;
    fn list_iter_with_arena(self, arena: &Arena) -> NounListIterator;
    fn eq_bytes_with_arena(self, bytes: impl AsRef<[u8]>, arena: &Arena) -> bool;
}

impl NounExt for Noun {
    fn cue_bytes(stack: &mut NockStack, bytes: &Bytes) -> std::result::Result<Noun, Error> {
        let atom = <Atom as AtomExt>::from_bytes(stack, bytes.as_ref());
        cue(stack, atom)
    }

    fn cue_bytes_slice(stack: &mut NockStack, bytes: &[u8]) -> std::result::Result<Noun, Error> {
        let atom = <IndirectAtom as IndirectAtomExt>::from_bytes(stack, bytes);
        cue(stack, atom)
    }

    fn jam_self(self, stack: &mut NockStack) -> JammedNoun {
        JammedNoun::from_noun(stack, self)
    }

    fn list_iter_with_arena(self, arena: &Arena) -> NounListIterator {
        NounListIterator::new(self, arena)
    }

    fn eq_bytes_with_arena(self, bytes: impl AsRef<[u8]>, arena: &Arena) -> bool {
        if let Ok(atom) = self.as_atom() {
            atom.eq_bytes_with_arena(bytes, arena)
        } else {
            false
        }
    }
}

#[derive(Clone, PartialEq, Debug, Encode, Decode)]
pub struct JammedNoun(#[bincode(with_serde)] pub Bytes);

impl JammedNoun {
    pub fn new(bytes: Bytes) -> Self {
        Self(bytes)
    }

    pub fn from_noun(stack: &mut NockStack, noun: Noun) -> Self {
        let jammed_atom = jam(stack, noun);
        let arena = stack.arena_ref();
        JammedNoun(Bytes::copy_from_slice(jammed_atom.as_ne_bytes_with_arena(arena)))
    }

    pub fn cue_self(&self, stack: &mut NockStack) -> std::result::Result<Noun, Error> {
        let atom = <IndirectAtom as IndirectAtomExt>::from_bytes(stack, self.0.as_ref());
        cue(stack, atom)
    }
}

impl From<&[u8]> for JammedNoun {
    fn from(bytes: &[u8]) -> Self {
        JammedNoun::new(Bytes::copy_from_slice(bytes))
    }
}

impl From<Vec<u8>> for JammedNoun {
    fn from(byte_vec: Vec<u8>) -> Self {
        JammedNoun::new(Bytes::from(byte_vec))
    }
}

impl AsRef<Bytes> for JammedNoun {
    fn as_ref(&self) -> &Bytes {
        &self.0
    }
}

impl AsRef<[u8]> for JammedNoun {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl Default for JammedNoun {
    fn default() -> Self {
        JammedNoun::new(Bytes::new())
    }
}

pub struct NounListIterator<'a>(Noun, &'a Arena);

impl<'a> NounListIterator<'a> {
    pub fn new(noun: Noun, arena: &'a Arena) -> Self {
        NounListIterator(noun, arena)
    }
}

impl<'a> Iterator for NounListIterator<'a> {
    type Item = Noun;

    fn next(&mut self) -> Option<Self::Item> {
        if let Ok(cell) = self.0.as_cell() {
            self.0 = cell.tail_with_arena(self.1);
            Some(cell.head_with_arena(self.1))
        } else if unsafe { self.0.raw_equals(&D(0)) } {
            None
        } else {
            panic!("Improper list terminator: {:?}", self.0);
        }
    }
}

pub fn make_tas<A: NounAllocator>(allocator: &mut A, tas: &str) -> Atom {
    <Atom as AtomExt>::from_bytes(allocator, tas.as_bytes())
}

/// Non-unifying structural equality for nouns.
///
/// Compares two nouns for structural equality without modifying them
/// (unlike unifying equality which may merge identical substructures).
/// This is suitable for use with allocators that don't support unification
/// (e.g., Pma, NounSlab) since it doesn't require temporary allocations.
///
/// Uses a worklist algorithm to avoid stack overflow on deep structures.
/// Tracks already-compared pairs to handle structural sharing efficiently.
/// Uses cached mugs (hashes) to quickly reject unequal nouns.
pub fn noun_equality(a: &Noun, b: &Noun, arena: &Arena) -> bool {
    // Track pairs we've already determined to be equal
    // Key is a_raw << 64 | b_raw (or b_raw << 64 | a_raw for symmetry)
    let mut already_equal: IntMap<u128, ()> = IntMap::new();

    fn ae_keys(a: Noun, b: Noun) -> (u128, u128) {
        let a_raw = unsafe { a.as_raw() } as u128;
        let b_raw = unsafe { b.as_raw() } as u128;
        (a_raw << 64 | b_raw, b_raw << 64 | a_raw)
    }

    fn check_ae(ae: &IntMap<u128, ()>, a: Noun, b: Noun) -> bool {
        let (key1, key2) = ae_keys(a, b);
        ae.contains_key(key1) || ae.contains_key(key2)
    }

    fn set_ae(ae: &mut IntMap<u128, ()>, a: Noun, b: Noun) {
        let (key1, _key2) = ae_keys(a, b);
        ae.insert(key1, ());
    }

    // Stack entries: either comparing two nouns, or marking a cell pair as equal after children match
    enum StackEntry {
        Nouns(Noun, Noun),
        MarkEqual(Noun, Noun),
    }

    let mut stack = vec![StackEntry::Nouns(*a, *b)];

    loop {
        let Some(entry) = stack.pop() else {
            // Stack empty means all comparisons succeeded
            return true;
        };

        match entry {
            StackEntry::MarkEqual(a, b) => {
                // Children matched, mark this pair as equal
                set_ae(&mut already_equal, a, b);
            }
            StackEntry::Nouns(a, b) => {
                // Quick check: identical raw values are equal
                if unsafe { a.raw_equals(&b) } {
                    continue;
                }

                // Already compared this pair?
                if check_ae(&already_equal, a, b) {
                    continue;
                }

                match (
                    a.as_ref_either_direct_allocated(),
                    b.as_ref_either_direct_allocated(),
                ) {
                    (Right(a_alloc), Right(b_alloc)) => {
                        // Both allocated - check mugs first for quick rejection
                        if let Some(a_mug) = a_alloc.get_cached_mug() {
                            if let Some(b_mug) = b_alloc.get_cached_mug() {
                                if a_mug != b_mug {
                                    return false;
                                }
                            }
                        }

                        match (a_alloc.as_ref_either(), b_alloc.as_ref_either()) {
                            (Left(a_indirect), Left(b_indirect)) => {
                                // Both indirect atoms - compare byte slices
                                if a_indirect.as_slice_with_arena(arena)
                                    != b_indirect.as_slice_with_arena(arena)
                                {
                                    return false;
                                }
                                set_ae(&mut already_equal, a, b);
                            }
                            (Right(a_cell), Right(b_cell)) => {
                                // Both cells - queue children for comparison
                                // Mark as equal after children are verified
                                stack.push(StackEntry::MarkEqual(a, b));
                                stack.push(StackEntry::Nouns(
                                    a_cell.tail_with_arena(arena),
                                    b_cell.tail_with_arena(arena),
                                ));
                                stack.push(StackEntry::Nouns(
                                    a_cell.head_with_arena(arena),
                                    b_cell.head_with_arena(arena),
                                ));
                            }
                            _ => {
                                // One indirect, one cell - not equal
                                return false;
                            }
                        }
                    }
                    _ => {
                        // At least one direct atom, and raw_equals failed above
                        return false;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::noun_equality;
    use crate::mem::NockStack;
    use crate::noun::{Cell, IndirectAtom, D};

    /// Verifies noun_equality correctly compares nouns for structural equality.
    ///
    /// Tests:
    /// - Direct atoms equal themselves
    /// - Different direct atoms are not equal
    /// - Indirect atoms equal themselves (same data)
    /// - Different indirect atoms are not equal
    /// - Cells equal themselves
    /// - Cells with different contents are not equal
    /// - Nested structures
    /// - Structural sharing (same substructure referenced twice)
    #[test]
    fn test_noun_equality() {
        let mut stack = NockStack::new(1 << 12, 0);
        stack.install_arena(); // Required for Cell::new to work

        // Direct atoms
        let d0 = D(0);
        let d1 = D(1);
        let d42 = D(42);
        let d42_copy = D(42);

        assert!(noun_equality(&d0, &d0), "D(0) == D(0)");
        assert!(noun_equality(&d42, &d42_copy), "D(42) == D(42)");
        assert!(!noun_equality(&d0, &d1), "D(0) != D(1)");
        assert!(!noun_equality(&d1, &d42), "D(1) != D(42)");

        // Indirect atoms
        let data1: [u64; 2] = [0xDEADBEEF_CAFEBABE, 0x12345678];
        let data2: [u64; 2] = [0xDEADBEEF_CAFEBABE, 0x12345678]; // same data
        let data3: [u64; 2] = [0xDEADBEEF_CAFEBABE, 0x87654321]; // different

        let indirect1 = unsafe { IndirectAtom::new_raw(&mut stack, 2, data1.as_ptr()) }.as_noun();
        let indirect2 = unsafe { IndirectAtom::new_raw(&mut stack, 2, data2.as_ptr()) }.as_noun();
        let indirect3 = unsafe { IndirectAtom::new_raw(&mut stack, 2, data3.as_ptr()) }.as_noun();

        assert!(noun_equality(&indirect1, &indirect1), "indirect1 == indirect1 (same ref)");
        assert!(noun_equality(&indirect1, &indirect2), "indirect1 == indirect2 (same data)");
        assert!(!noun_equality(&indirect1, &indirect3), "indirect1 != indirect3 (different data)");
        assert!(!noun_equality(&indirect1, &d42), "indirect != direct");

        // Simple cells
        let cell1 = Cell::new(&mut stack, D(1), D(2)).as_noun();
        let cell2 = Cell::new(&mut stack, D(1), D(2)).as_noun();
        let cell3 = Cell::new(&mut stack, D(1), D(3)).as_noun();
        let cell4 = Cell::new(&mut stack, D(2), D(2)).as_noun();

        assert!(noun_equality(&cell1, &cell1), "[1 2] == [1 2] (same ref)");
        assert!(noun_equality(&cell1, &cell2), "[1 2] == [1 2] (different refs)");
        assert!(!noun_equality(&cell1, &cell3), "[1 2] != [1 3]");
        assert!(!noun_equality(&cell1, &cell4), "[1 2] != [2 2]");
        assert!(!noun_equality(&cell1, &d1), "cell != direct atom");

        // Nested cells - build inner cells first to avoid borrow issues
        let inner1 = Cell::new(&mut stack, D(1), D(2)).as_noun();
        let nested1 = Cell::new(&mut stack, inner1, D(3)).as_noun();
        let inner2 = Cell::new(&mut stack, D(1), D(2)).as_noun();
        let nested2 = Cell::new(&mut stack, inner2, D(3)).as_noun();
        let inner3 = Cell::new(&mut stack, D(1), D(9)).as_noun();
        let nested3 = Cell::new(&mut stack, inner3, D(3)).as_noun();

        assert!(noun_equality(&nested1, &nested2), "[[1 2] 3] == [[1 2] 3]");
        assert!(!noun_equality(&nested1, &nested3), "[[1 2] 3] != [[1 9] 3]");

        // Structural sharing
        let shared = Cell::new(&mut stack, D(5), D(6)).as_noun();
        let with_sharing = Cell::new(&mut stack, shared, shared).as_noun();
        let inner_a = Cell::new(&mut stack, D(5), D(6)).as_noun();
        let inner_b = Cell::new(&mut stack, D(5), D(6)).as_noun();
        let without_sharing = Cell::new(&mut stack, inner_a, inner_b).as_noun();

        // Both should be equal even though one shares and one doesn't
        assert!(noun_equality(&with_sharing, &without_sharing), "[[5 6] [5 6]] with sharing == without sharing");

        // Cells containing indirect atoms
        let cell_indirect1 = Cell::new(&mut stack, indirect1, D(99)).as_noun();
        let cell_indirect2 = Cell::new(&mut stack, indirect2, D(99)).as_noun(); // same indirect data
        let cell_indirect3 = Cell::new(&mut stack, indirect3, D(99)).as_noun(); // different indirect data

        assert!(noun_equality(&cell_indirect1, &cell_indirect2), "cells with same indirect atoms are equal");
        assert!(!noun_equality(&cell_indirect1, &cell_indirect3), "cells with different indirect atoms are not equal");
    }
}
