use std::slice::{from_raw_parts, from_raw_parts_mut};
use std::{error, fmt, ptr};

use bitvec::prelude::{BitSlice, Lsb0};
use either::{Either, Left, Right};
use ibig::{Stack, UBig};
use intmap::IntMap;
use nockvm_macros::tas;
use static_assertions::assert_cfg;

use crate::mem::{word_size_of, Arena, MemContext, NockStack};

crate::gdb!();

assert_cfg!(
    target_endian = "little",
    "nockvm will not execute correctly on non-little-endian systems"
);

/** Tag for a direct atom. */
pub(crate) const DIRECT_TAG: u64 = 0x0;

/** Tag mask for a direct atom. */
pub(crate) const DIRECT_MASK: u64 = !(u64::MAX >> 1);

/** Maximum value of a direct atom. Values higher than this must be represented by indirect atoms. */
pub const DIRECT_MAX: u64 = u64::MAX >> 1;

/** Tag for an indirect atom. */
pub(crate) const INDIRECT_TAG: u64 = u64::MAX & DIRECT_MASK;

/** Tag mask for an indirect atom. */
pub(crate) const INDIRECT_MASK: u64 = !(u64::MAX >> 2);

/** Tag for a cell. */
pub(crate) const CELL_TAG: u64 = u64::MAX & INDIRECT_MASK;

/** Tag mask for a cell. */
pub(crate) const CELL_MASK: u64 = !(u64::MAX >> 3);

const LOCATION_BIT: u64 = 1 << 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PtrLocation {
    Stack,
    Offset,
}

#[derive(Debug, Clone, Copy)]
struct TaggedPtr(u64);

impl TaggedPtr {
    #[inline(always)]
    fn from_raw(raw: u64) -> Self {
        Self(raw)
    }

    #[inline(always)]
    unsafe fn from_stack_ptr(ptr: *const u8, tag: u64) -> Self {
        debug_assert!(
            (ptr as usize) & 0x7 == 0,
            "Stack pointer {:p} not 8-byte aligned",
            ptr
        );
        Self(((ptr as u64) >> 3) | tag)
    }

    #[inline(always)]
    fn from_offset(words: u32, tag: u64) -> Self {
        debug_assert!(
            (words as u64) < LOCATION_BIT,
            "offset {} exceeds payload capacity",
            words
        );
        Self((words as u64) | LOCATION_BIT | tag)
    }

    #[inline(always)]
    fn location(self) -> PtrLocation {
        if self.0 & LOCATION_BIT == 0 {
            PtrLocation::Stack
        } else {
            PtrLocation::Offset
        }
    }

    #[inline(always)]
    fn payload(self, mask: u64) -> u64 {
        self.0 & !(mask | LOCATION_BIT)
    }

    fn resolve_const(self, mask: u64, arena: &Arena) -> *const u8 {
        match self.location() {
            PtrLocation::Stack => ((self.payload(mask)) << 3) as *const u8,
            PtrLocation::Offset => arena.ptr_from_offset(self.payload(mask) as u32) as *const u8,
        }
    }

    #[inline(always)]
    fn resolve_mut(self, mask: u64, arena: &Arena) -> *mut u8 {
        self.resolve_const(mask, arena) as *mut u8
    }

    #[inline(always)]
    fn raw(self) -> u64 {
        self.0
    }
}

/*  A note on forwarding pointers:
 *
 *  Forwarding pointers are only used temporarily during copies between NockStack frames and between
 *  the NockStack and the PMA. Since unifying equality checks can create structural sharing between
 *  Noun objects, forwarding pointers act as a signal that a Noun has already been copied to the
 *  "to" space. The old Noun object in the "from" space is given a forwarding pointer so that any
 *  future refernces to the same structure know that it has already been copied and that they should
 *  retain the structural sharing relationship by referencing the new copy in the "to" copy space.
 *
 *  The Nouns in the "from" space marked with forwarding pointers are dangling pointers after a copy
 *  operation. No code outside of the copying code checks for forwarding pointers. This invariant
 *  must be enforced in two ways:
 *      1. The current frame must be immediately popped after preserving data, when
 *          copying from a junior NockStack frame to a senior NockStack frame.
 *      2. All persistent derived state (e.g. Hot state, Warm state) must be preserved
 *          and the root NockStack frame flipped after saving data to the PMA.
 */

/** Tag for a forwarding pointer */
const FORWARDING_TAG: u64 = u64::MAX & CELL_MASK;

/** Tag mask for a forwarding pointer */
const FORWARDING_MASK: u64 = CELL_MASK;

/** Shorthand for 0's that actually are ~ **/
pub const SIG: Noun = D(0);

/** Loobeans */
pub const YES: Noun = D(0);
pub const NO: Noun = D(1);
pub const NONE: Noun = unsafe { DirectAtom::new_unchecked(tas!(b"MORMAGIC")).as_noun() };

#[cfg(feature = "check_acyclic")]
#[macro_export]
macro_rules! assert_acyclic {
    ( $x:expr ) => {
        assert_no_alloc::permit_alloc(|| {
            assert!(crate::noun::acyclic_noun($x));
        })
    };
}

#[cfg(not(feature = "check_acyclic"))]
#[macro_export]
macro_rules! assert_acyclic {
    ( $x:expr ) => {};
}

pub fn acyclic_noun(noun: Noun) -> bool {
    let mut seen = IntMap::new();
    acyclic_noun_go(noun, &mut seen)
}

fn acyclic_noun_go(noun: Noun, seen: &mut IntMap<u64, ()>) -> bool {
    match noun.as_either_atom_cell() {
        Left(_atom) => true,
        Right(cell) => {
            if seen.get(cell.0).is_some() {
                false
            } else {
                seen.insert(cell.0, ());
                // SAFETY: This is debug code that only runs on stack-allocated nouns
                unsafe {
                    if acyclic_noun_go(cell.head_stack(), seen) {
                        if acyclic_noun_go(cell.tail_stack(), seen) {
                            seen.remove(cell.0);
                            true
                        } else {
                            false
                        }
                    } else {
                        false
                    }
                }
            }
        }
    }
}

#[cfg(feature = "check_forwarding")]
#[macro_export]
macro_rules! assert_no_forwarding_pointers {
    ( $x:expr ) => {
        assert_no_alloc::permit_alloc(|| {
            assert!(crate::noun::no_forwarding_pointers($x));
        })
    };
}

#[cfg(not(feature = "check_forwarding"))]
#[macro_export]
macro_rules! assert_no_forwarding_pointers {
    ( $x:expr ) => {};
}

pub fn no_forwarding_pointers(noun: Noun) -> bool {
    let mut dbg_stack = Vec::new();
    dbg_stack.push(noun);

    while !dbg_stack.is_empty() {
        if let Some(noun) = dbg_stack.pop() {
            if unsafe { noun.raw & FORWARDING_MASK == FORWARDING_TAG } {
                return false;
            } else if let Ok(cell) = noun.as_cell() {
                // SAFETY: This is debug code that only runs on stack-allocated nouns
                unsafe {
                    dbg_stack.push(cell.tail_stack());
                    dbg_stack.push(cell.head_stack());
                }
            }
        } else {
            break;
        }
    }

    true
}

/** Test if a noun is a direct atom. */
fn is_direct_atom(noun: u64) -> bool {
    noun & DIRECT_MASK == DIRECT_TAG
}

/** Test if a noun is an indirect atom. */
fn is_indirect_atom(noun: u64) -> bool {
    noun & INDIRECT_MASK == INDIRECT_TAG
}

/** Test if a noun is a cell. */
fn is_cell(noun: u64) -> bool {
    noun & CELL_MASK == CELL_TAG
}

/** A noun-related error. */
#[derive(Debug, PartialEq)]
pub enum Error {
    /** Expected type [`Allocated`]. */
    NotAllocated,
    /** Expected type [`Atom`]. */
    NotAtom,
    /** Expected type [`Cell`]. */
    NotCell,
    /** Expected type [`DirectAtom`]. */
    NotDirectAtom,
    /** Expected type [`IndirectAtom`]. */
    NotIndirectAtom,
    /** The value can't be represented by the given type. */
    NotRepresentable,
}

impl error::Error for Error {}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotAllocated => f.write_str("not an allocated noun"),
            Error::NotAtom => f.write_str("not an atom"),
            Error::NotCell => f.write_str("not a cell"),
            Error::NotDirectAtom => f.write_str("not a direct atom"),
            Error::NotIndirectAtom => f.write_str("not an indirect atom"),
            Error::NotRepresentable => f.write_str("unrepresentable value"),
        }
    }
}

impl From<Error> for () {
    fn from(_: Error) -> Self {}
}

/** A [`Result`] that returns an [`Error`] on error. */
pub type Result<T> = std::result::Result<T, Error>;

/** A direct atom.
 *
 * Direct atoms represent an atom up to and including DIRECT_MAX as a machine word.
 */
#[derive(Copy, Clone)]
#[repr(C)]
#[repr(packed(8))]
pub struct DirectAtom(u64);

impl DirectAtom {
    /** Create a new direct atom, or panic if the value is greater than DIRECT_MAX */
    pub const fn new_panic(value: u64) -> Self {
        if value > DIRECT_MAX {
            panic!("Number is greater than DIRECT_MAX")
        } else {
            DirectAtom(value)
        }
    }

    /** Create a new direct atom, or return Err if the value is greater than DIRECT_MAX */
    pub const fn new(value: u64) -> Result<Self> {
        if value > DIRECT_MAX {
            Err(Error::NotRepresentable)
        } else {
            Ok(DirectAtom(value))
        }
    }

    /** Create a new direct atom. This is unsafe because the value is not checked.
     *
     * Attempting to create a direct atom with a value greater than DIRECT_MAX will
     * result in this value being interpreted by the runtime as a cell or indirect atom,
     * with corresponding memory accesses. Thus, this function is marked as unsafe.
     */
    pub const unsafe fn new_unchecked(value: u64) -> Self {
        DirectAtom(value)
    }

    pub fn bit_size(self) -> usize {
        (64 - self.0.leading_zeros()) as usize
    }

    pub fn as_atom(self) -> Atom {
        Atom { direct: self }
    }

    pub fn as_ubig<S: Stack>(self, _stack: &mut S) -> UBig {
        UBig::from(self.0)
    }

    pub const fn as_noun(self) -> Noun {
        Noun { direct: self }
    }

    pub fn data(self) -> u64 {
        self.0
    }

    pub fn as_bitslice(&self) -> &BitSlice<u64, Lsb0> {
        BitSlice::from_element(&self.0)
    }

    pub fn as_bitslice_mut(&mut self) -> &mut BitSlice<u64, Lsb0> {
        BitSlice::from_element_mut(&mut self.0)
    }

    pub fn as_ne_bytes(&self) -> &[u8] {
        let bytes: &[u8; 8] = unsafe { std::mem::transmute(&self.0) };
        &bytes[..]
    }

    /// Returns Vec<u8> under native-endian of the machine
    pub fn to_ne_bytes(&self) -> Vec<u8> {
        self.as_ne_bytes().to_vec()
    }

    pub fn to_be_bytes(&self) -> Vec<u8> {
        self.0.to_be_bytes().to_vec()
    }

    pub fn to_le_bytes(&self) -> Vec<u8> {
        self.0.to_le_bytes().to_vec()
    }
}

impl fmt::Debug for DirectAtom {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.0 == 0 {
            return write!(f, "0");
        }

        let mut null = false;
        let mut n = 0;
        let bytes = self.0.to_le_bytes();
        for byte in bytes.iter() {
            if *byte == 0 {
                null = true;
                continue;
            }
            if (null && *byte != 0) || *byte < 33 || *byte > 126 {
                return write!(f, "{}", self.0);
            }
            n += 1;
        }
        if n > 1 {
            write!(f, "%{}", unsafe {
                std::str::from_utf8_unchecked(&bytes[..n])
            })
        } else {
            write!(f, "{}", self.0)
        }
    }
}

#[allow(non_snake_case)]
pub const fn D(n: u64) -> Noun {
    DirectAtom::new_panic(n).as_noun()
}

#[allow(non_snake_case)]
pub fn T<A: NounAllocator>(allocator: &mut A, tup: &[Noun]) -> Noun {
    Cell::new_tuple(allocator, tup).as_noun()
}

/// Create $tape Noun from ASCII string
pub fn tape<A: NounAllocator>(allocator: &mut A, text: &str) -> Noun {
    //  XX: Needs unit tests
    let mut res = D(0);
    for c in text.bytes().rev() {
        res = T(allocator, &[D(c as u64), res])
    }
    res
}

/** An indirect atom.
 *
 *  Indirect atoms represent atoms above DIRECT_MAX as a tagged pointer to a memory buffer
 *  structured as:
 *
 *  - first word: metadata
 *  - second word: size in 64-bit words
 *  - remaining words: data
 *
 *  Indirect atoms are always stored in little-endian byte order
 */
#[derive(Copy, Clone)]
#[repr(C)]
#[repr(packed(8))]
pub struct IndirectAtom(u64);

impl IndirectAtom {
    /** Tag the pointer and type it as an indirect atom. */
    pub unsafe fn from_raw_pointer(ptr: *const u64) -> Self {
        IndirectAtom(TaggedPtr::from_stack_ptr(ptr as *const u8, INDIRECT_TAG).raw())
    }

    /// Create an IndirectAtom from a PMA offset.
    ///
    /// **IMPORTANT**: This should ONLY be used for PMA-resident data.
    /// Stack data should always use `from_raw_pointer()` (LOCATION_BIT=0).
    /// PMA data uses offset form (LOCATION_BIT=1) and requires the PMA arena
    /// for resolution.
    pub fn from_pma_offset(words: u32) -> Self {
        IndirectAtom(TaggedPtr::from_offset(words, INDIRECT_TAG).raw())
    }

    /** Strip the tag from an indirect atom and return it as a mutable pointer to its memory buffer. */
    unsafe fn to_raw_pointer_mut_with_arena(&mut self, arena: &Arena) -> *mut u64 {
        TaggedPtr::from_raw(self.0).resolve_mut(INDIRECT_MASK, arena) as *mut u64
    }

    /** Strip the tag from an indirect atom and return it as a pointer to its memory buffer. */
    pub unsafe fn to_raw_pointer_with_arena(&self, arena: &Arena) -> *const u64 {
        TaggedPtr::from_raw(self.0).resolve_const(INDIRECT_MASK, arena) as *const u64
    }

    /// Get raw pointer, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn to_raw_pointer(&self) -> *const u64 {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            ((tagged.payload(INDIRECT_MASK)) << 3) as *const u64
        } else {
            Arena::with_current(|arena| unsafe { self.to_raw_pointer_with_arena(arena) })
        }
    }

    /// Get raw pointer for stack-pointer form atoms only
    pub unsafe fn to_raw_pointer_stack(&self) -> *const u64 {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            ((tagged.payload(INDIRECT_MASK)) << 3) as *const u64
        } else {
            panic!("expected stack-pointer Noun, got offset instead");
        }
    }

    /// Returns Some(ptr) if stack-pointer form, None if PMA form.
    /// Used by NounSlab to check if a noun is in the slab or PMA.
    #[inline(always)]
    pub fn stack_data_pointer(&self) -> Option<*const u64> {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            Some(((tagged.payload(INDIRECT_MASK)) << 3) as *const u64)
        } else {
            None
        }
    }

    /// Get mutable raw pointer for stack-pointer form atoms only
    pub unsafe fn to_raw_pointer_mut_stack(&mut self) -> *mut u64 {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            ((tagged.payload(INDIRECT_MASK)) << 3) as *mut u64
        } else {
            panic!("expected stack-pointer Noun, got offset instead");
        }
    }

    pub unsafe fn set_forwarding_pointer_with_arena(&mut self, new_me: *const u64, arena: &Arena) {
        // This is OK because the size is stored as 64 bit words, not bytes.
        // Thus, a true size value will never be larger than U64::MAX >> 3, and so
        // any of the high bits set as an MSB
        *self.to_raw_pointer_mut_with_arena(arena).add(1) =
            TaggedPtr::from_stack_ptr(new_me as *const u8, FORWARDING_TAG).raw();
    }

    pub unsafe fn forwarding_pointer_with_arena(&self, arena: &Arena) -> Option<IndirectAtom> {
        let size_raw = *self.to_raw_pointer_with_arena(arena).add(1);
        if size_raw & FORWARDING_MASK == FORWARDING_TAG {
            let ptr =
                TaggedPtr::from_raw(size_raw).resolve_const(FORWARDING_MASK, arena) as *const u64;
            Some(Self::from_raw_pointer(ptr))
        } else {
            None
        }
    }

    /// Set forwarding pointer (stack-pointer form only)
    pub unsafe fn set_forwarding_pointer_stack(&mut self, new_me: *const u64) {
        *self.to_raw_pointer_mut_stack().add(1) =
            TaggedPtr::from_stack_ptr(new_me as *const u8, FORWARDING_TAG).raw();
    }

    /// Set forwarding pointer, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub unsafe fn set_forwarding_pointer(&mut self, new_me: *const u64) {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            self.set_forwarding_pointer_stack(new_me)
        } else {
            Arena::with_current(|arena| self.set_forwarding_pointer_with_arena(new_me, arena))
        }
    }

    /// Get forwarding pointer (stack-pointer form only)
    pub unsafe fn forwarding_pointer_stack(&self) -> Option<IndirectAtom> {
        let size_raw = *self.to_raw_pointer_stack().add(1);
        if size_raw & FORWARDING_MASK == FORWARDING_TAG {
            // Forwarding pointers always point to stack addresses
            let ptr = (TaggedPtr::from_raw(size_raw).payload(FORWARDING_MASK) << 3) as *const u64;
            Some(Self::from_raw_pointer(ptr))
        } else {
            None
        }
    }

    /// Get forwarding pointer, auto-dispatching based on LOCATION_BIT.
    /// Note: Forwarding pointers always point to stack addresses.
    #[inline(always)]
    pub unsafe fn forwarding_pointer(&self) -> Option<IndirectAtom> {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            self.forwarding_pointer_stack()
        } else {
            Arena::with_current(|arena| self.forwarding_pointer_with_arena(arena))
        }
    }

    /** Make an indirect atom by copying from other memory.
     *
     *  Note: size is in 64-bit words, not bytes.
     */
    pub unsafe fn new_raw<A: NounAllocator>(
        allocator: &mut A,
        size: usize,
        data: *const u64,
    ) -> Self {
        let (mut indirect, buffer) = Self::new_raw_mut(allocator, size);
        ptr::copy_nonoverlapping(data, buffer, size);
        // Use normalize_stack since new_raw_mut creates stack-pointer form atoms
        *(indirect.normalize_stack())
    }

    /** Make an indirect atom by copying from other memory.
     *
     *  Note: size is bytes, not words
     */
    pub unsafe fn new_raw_bytes<A: NounAllocator>(
        allocator: &mut A,
        size: usize,
        data: *const u8,
    ) -> Self {
        let (mut indirect, buffer) = Self::new_raw_mut_bytes(allocator, size);
        ptr::copy_nonoverlapping(data, buffer.as_mut_ptr(), size);
        // Use normalize_stack since new_raw_mut_bytes creates stack-pointer form atoms
        *(indirect.normalize_stack())
    }

    pub unsafe fn new_raw_bytes_ref<A: NounAllocator>(allocator: &mut A, data: &[u8]) -> Self {
        IndirectAtom::new_raw_bytes(allocator, data.len(), data.as_ptr())
    }

    /** Make an indirect atom that can be written into. Return the atom (which should not be used
     * until it is written and normalized) and a mutable pointer which is the data buffer for the
     * indirect atom, to be written into.
     */
    pub unsafe fn new_raw_mut<A: NounAllocator>(
        allocator: &mut A,
        size: usize,
    ) -> (Self, *mut u64) {
        debug_assert!(size > 0);
        let buffer = allocator.alloc_indirect(size);
        *buffer = 0;
        *buffer.add(1) = size as u64;
        (Self::from_raw_pointer(buffer), buffer.add(2))
    }

    /** Make an indirect atom that can be written into, and zero the whole data buffer.
     * Return the atom (which should not be used until it is written and normalized) and a mutable
     * pointer which is the data buffer for the indirect atom, to be written into.
     */
    pub unsafe fn new_raw_mut_zeroed<A: NounAllocator>(
        allocator: &mut A,
        size: usize,
    ) -> (Self, *mut u64) {
        let allocation = Self::new_raw_mut(allocator, size);
        ptr::write_bytes(allocation.1, 0, size);
        allocation
    }

    /** Make an indirect atom that can be written into as a bitslice. The constraints of
     * [new_raw_mut_zeroed] also apply here
     */
    pub unsafe fn new_raw_mut_bitslice<'a, A: NounAllocator>(
        allocator: &mut A,
        size: usize,
    ) -> (Self, &'a mut BitSlice<u64, Lsb0>) {
        let (noun, ptr) = Self::new_raw_mut_zeroed(allocator, size);
        (
            noun,
            BitSlice::from_slice_mut(from_raw_parts_mut(ptr, size)),
        )
    }

    /** Make an indirect atom that can be written into as a slice of bytes. The constraints of
     * [new_raw_mut_zeroed] also apply here
     *
     * Note: size is bytes, not words
     */
    pub unsafe fn new_raw_mut_bytes<'a, A: NounAllocator>(
        allocator: &mut A,
        size: usize,
    ) -> (Self, &'a mut [u8]) {
        let word_size = (size + 7) >> 3;
        let (noun, ptr) = Self::new_raw_mut_zeroed(allocator, word_size);
        (noun, from_raw_parts_mut(ptr as *mut u8, size))
    }

    /// Create an indirect atom backed by a fixed-size array
    pub unsafe fn new_raw_mut_bytearray<'a, const N: usize, A: NounAllocator>(
        allocator: &mut A,
    ) -> (Self, &'a mut [u8; N]) {
        let word_size = (std::mem::size_of::<[u8; N]>() + 7) >> 3;
        let (noun, ptr) = Self::new_raw_mut_zeroed(allocator, word_size);
        (noun, &mut *(ptr as *mut [u8; N]))
    }

    /** Size of an indirect atom in 64-bit words */
    pub fn size_with_arena(&self, arena: &Arena) -> usize {
        unsafe { *(self.to_raw_pointer_with_arena(arena).add(1)) as usize }
    }

    /** Size of an indirect atom in 64-bit words (stack-pointer form only) */
    pub unsafe fn size_stack(&self) -> usize {
        *(self.to_raw_pointer_stack().add(1)) as usize
    }

    /** Memory size of an indirect atom (including size + metadata fields) in 64-bit words */
    pub fn raw_size_with_arena(&self, arena: &Arena) -> usize {
        self.size_with_arena(arena) + 2
    }

    /// Memory size of an indirect atom (including size + metadata fields) in 64-bit words.
    /// Auto-dispatches based on LOCATION_BIT. Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn raw_size(&self) -> usize {
        self.size() + 2
    }

    pub fn bit_size_with_arena(&self, arena: &Arena) -> usize {
        unsafe {
            ((self.size_with_arena(arena) - 1) << 6) + 64
                - (*(self
                    .to_raw_pointer_with_arena(arena)
                    .add(2 + self.size_with_arena(arena) - 1)))
                .leading_zeros() as usize
        }
    }

    /// Get bit size, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn bit_size(&self) -> usize {
        unsafe {
            ((self.size() - 1) << 6) + 64
                - (*(self.data_pointer().add(self.size() - 1))).leading_zeros() as usize
        }
    }

    /** Pointer to data for indirect atom */
    pub fn data_pointer_with_arena(&self, arena: &Arena) -> *const u64 {
        unsafe { self.to_raw_pointer_with_arena(arena).add(2) }
    }

    pub fn data_pointer_mut_with_arena(&mut self, arena: &Arena) -> *mut u64 {
        unsafe { self.to_raw_pointer_mut_with_arena(arena).add(2) }
    }

    /// Get data pointer for stack-pointer form only. Returns base pointer + 2 (skipping header).
    pub unsafe fn data_pointer_stack(&self) -> *const u64 {
        self.to_raw_pointer_stack().add(2)
    }

    pub fn as_slice_with_arena(&self, arena: &Arena) -> &[u64] {
        unsafe {
            from_raw_parts(
                self.data_pointer_with_arena(arena),
                self.size_with_arena(arena),
            )
        }
    }

    /// Get slice of data words for stack-pointer form only
    pub unsafe fn as_slice_stack(&self) -> &[u64] {
        from_raw_parts(self.data_pointer_stack(), self.size_stack())
    }

    pub fn as_mut_slice_with_arena(&mut self, arena: &Arena) -> &mut [u64] {
        unsafe {
            from_raw_parts_mut(
                self.data_pointer_mut_with_arena(arena),
                self.size_with_arena(arena),
            )
        }
    }

    pub fn as_ne_bytes_with_arena(&self, arena: &Arena) -> &[u8] {
        unsafe {
            from_raw_parts(
                self.data_pointer_with_arena(arena) as *const u8,
                self.size_with_arena(arena) << 3,
            )
        }
    }

    /// Get bytes (stack-pointer form only)
    pub unsafe fn as_ne_bytes_stack(&self) -> &[u8] {
        from_raw_parts(
            self.data_pointer_stack() as *const u8,
            self.size_stack() << 3,
        )
    }

    pub fn to_ne_bytes_with_arena(&self, arena: &Arena) -> Vec<u8> {
        self.as_ne_bytes_with_arena(arena).to_vec()
    }

    #[allow(unused)]
    pub fn to_be_bytes_with_arena(&self, arena: &Arena) -> Vec<u8> {
        if self.size_with_arena(arena) == 1 {
            let num = unsafe { *(self.data_pointer_with_arena(arena)) };
            num.to_be_bytes().to_vec()
        } else {
            let mut bytes_ne = self.to_ne_bytes_with_arena(arena);
            #[cfg(target_endian = "little")]
            {
                bytes_ne.reverse()
            }
            bytes_ne
        }
    }

    #[allow(unused)]
    pub fn to_le_bytes_with_arena(&self, arena: &Arena) -> Vec<u8> {
        if self.size_with_arena(arena) == 1 {
            let num = unsafe { *(self.data_pointer_with_arena(arena)) };
            num.to_le_bytes().to_vec()
        } else {
            let mut bytes_ne = self.to_ne_bytes_with_arena(arena);
            #[cfg(target_endian = "big")]
            {
                bytes_ne.reverse()
            }

            bytes_ne
        }
    }

    /// Returns Vec<u8> in native-endian order (stack-pointer form only)
    #[allow(unused)]
    pub unsafe fn to_ne_bytes_stack(&self) -> Vec<u8> {
        let size = self.size_stack();
        let mut v = Vec::with_capacity(size << 3);
        for i in 0..size {
            v.extend_from_slice(&(*self.data_pointer_stack().add(i)).to_ne_bytes())
        }
        v
    }

    /// Returns Vec<u8> in big-endian order (stack-pointer form only)
    #[allow(unused)]
    pub unsafe fn to_be_bytes_stack(&self) -> Vec<u8> {
        if self.size_stack() == 1 {
            let num = *(self.data_pointer_stack());
            num.to_be_bytes().to_vec()
        } else {
            let mut bytes_ne = self.to_ne_bytes_stack();
            #[cfg(target_endian = "little")]
            {
                bytes_ne.reverse()
            }
            bytes_ne
        }
    }

    /// Returns Vec<u8> in little-endian order (stack-pointer form only)
    #[allow(unused)]
    pub unsafe fn to_le_bytes_stack(&self) -> Vec<u8> {
        if self.size_stack() == 1 {
            let num = *(self.data_pointer_stack());
            num.to_le_bytes().to_vec()
        } else {
            let mut bytes_ne = self.to_ne_bytes_stack();
            #[cfg(target_endian = "big")]
            {
                bytes_ne.reverse()
            }
            bytes_ne
        }
    }

    /** BitSlice view on an indirect atom, with lifetime tied to reference to indirect atom. */
    pub fn as_bitslice_with_arena(&self, arena: &Arena) -> &BitSlice<u64, Lsb0> {
        BitSlice::from_slice(self.as_slice_with_arena(arena))
    }

    /// BitSlice view for stack-pointer form only
    pub unsafe fn as_bitslice_stack(&self) -> &BitSlice<u64, Lsb0> {
        BitSlice::from_slice(self.as_slice_stack())
    }

    pub fn as_bitslice_mut_with_arena(&mut self, arena: &Arena) -> &mut BitSlice<u64, Lsb0> {
        BitSlice::from_slice_mut(self.as_mut_slice_with_arena(arena))
    }

    pub fn as_ubig_with_arena<S: Stack>(&self, stack: &mut S, arena: &Arena) -> UBig {
        let bytes_mem_repr = self.as_ne_bytes_with_arena(arena);

        #[cfg(target_endian = "little")]
        {
            UBig::from_le_bytes_stack(stack, bytes_mem_repr)
        }
        #[cfg(not(target_endian = "little"))]
        {
            UBig::from_be_bytes_stack(stack, bytes_mem_repr)
        }
    }

    /// Get as UBig, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn as_ubig<S: Stack>(&self, stack: &mut S) -> UBig {
        let bytes_mem_repr = self.as_ne_bytes();

        #[cfg(target_endian = "little")]
        {
            UBig::from_le_bytes_stack(stack, bytes_mem_repr)
        }
        #[cfg(not(target_endian = "little"))]
        {
            UBig::from_be_bytes_stack(stack, bytes_mem_repr)
        }
    }

    pub unsafe fn as_u64_with_arena(self, arena: &Arena) -> Result<u64> {
        if self.size_with_arena(arena) == 1 {
            Ok(*(self.data_pointer_with_arena(arena)))
        } else {
            Err(Error::NotRepresentable)
        }
    }

    /// Get as u64 for stack-pointer form only
    pub unsafe fn as_u64_stack(self) -> Result<u64> {
        if self.size_stack() == 1 {
            Ok(*(self.data_pointer_stack()))
        } else {
            Err(Error::NotRepresentable)
        }
    }

    /** Produce a SoftFloat-compatible ordered pair of 64-bit words */
    pub fn as_u64_pair_with_arena(self, arena: &Arena) -> Result<[u64; 2]> {
        if self.size_with_arena(arena) <= 2 {
            let u128_array = &mut [0u64; 2];
            u128_array.copy_from_slice(&(self.as_slice_with_arena(arena)[0..2]));
            Ok(*u128_array)
        } else {
            Err(Error::NotRepresentable)
        }
    }

    /// Produce a SoftFloat-compatible ordered pair of 64-bit words,
    /// auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn as_u64_pair(self) -> Result<[u64; 2]> {
        if self.size() <= 2 {
            let u128_array = &mut [0u64; 2];
            u128_array.copy_from_slice(&(self.as_slice()[0..2]));
            Ok(*u128_array)
        } else {
            Err(Error::NotRepresentable)
        }
    }

    /** Ensure that the size does not contain any trailing 0 words */
    pub unsafe fn normalize_with_arena(&mut self, arena: &Arena) -> &Self {
        let mut index = self.size_with_arena(arena) - 1;
        let data = self.data_pointer_with_arena(arena);
        loop {
            if index == 0 || *(data.add(index)) != 0 {
                break;
            }
            index -= 1;
        }
        *(self.to_raw_pointer_mut_with_arena(arena).add(1)) = (index + 1) as u64;
        self
    }

    /// Normalize a stack-pointer form indirect atom (no arena needed).
    /// Panics if the atom is in offset form.
    pub unsafe fn normalize_stack(&mut self) -> &Self {
        let ptr = self
            .to_raw_pointer_mut_stack();
        let mut index = (*(ptr.add(1)) as usize) - 1; // size is at offset 1
        let data = ptr.add(2); // data starts at offset 2
        loop {
            if index == 0 || *(data.add(index)) != 0 {
                break;
            }
            index -= 1;
        }
        *(ptr.add(1)) = (index + 1) as u64;
        self
    }

    /** Normalize, but convert to direct atom if it will fit */
    pub unsafe fn normalize_as_atom_with_arena(&mut self, arena: &Arena) -> Atom {
        self.normalize_with_arena(arena);
        if self.size_with_arena(arena) == 1
            && *(self.data_pointer_with_arena(arena)) <= DIRECT_MAX
        {
            Atom {
                direct: DirectAtom(*(self.data_pointer_with_arena(arena))),
            }
        } else {
            Atom { indirect: *self }
        }
    }

    /// Normalize a stack-pointer form atom, converting to direct if it fits.
    /// Panics if the atom is in offset form.
    pub unsafe fn normalize_as_atom_stack(&mut self) -> Atom {
        self.normalize_stack();
        let ptr = self
            .to_raw_pointer_stack();
        let size = *(ptr.add(1)) as usize;
        let data = ptr.add(2);
        if size == 1 && *data <= DIRECT_MAX {
            Atom {
                direct: DirectAtom(*data),
            }
        } else {
            Atom { indirect: *self }
        }
    }

    pub fn as_atom(self) -> Atom {
        Atom { indirect: self }
    }

    pub fn as_allocated(self) -> Allocated {
        Allocated { indirect: self }
    }

    pub fn as_noun(self) -> Noun {
        Noun { indirect: self }
    }

    // Auto-dispatch methods that check LOCATION_BIT and use thread-local arena for PMA

    /// Size of an indirect atom in 64-bit words, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn size(&self) -> usize {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            unsafe { self.size_stack() }
        } else {
            Arena::with_current(|arena| self.size_with_arena(arena))
        }
    }

    /// Pointer to data for indirect atom, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn data_pointer(&self) -> *const u64 {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            unsafe { self.data_pointer_stack() }
        } else {
            Arena::with_current(|arena| self.data_pointer_with_arena(arena))
        }
    }

    /// Get slice of data words, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn as_slice(&self) -> &[u64] {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            unsafe { self.as_slice_stack() }
        } else {
            Arena::with_current(|arena| self.as_slice_with_arena(arena))
        }
    }

    /// Get as u64, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn as_u64(self) -> Result<u64> {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            unsafe { self.as_u64_stack() }
        } else {
            Arena::with_current(|arena| unsafe { self.as_u64_with_arena(arena) })
        }
    }

    /// Get bytes, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn as_ne_bytes(&self) -> &[u8] {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            unsafe { self.as_ne_bytes_stack() }
        } else {
            Arena::with_current(|arena| self.as_ne_bytes_with_arena(arena))
        }
    }

    /// Get Vec<u8> in native-endian order, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn to_ne_bytes(&self) -> Vec<u8> {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            unsafe { self.to_ne_bytes_stack() }
        } else {
            Arena::with_current(|arena| self.to_ne_bytes_with_arena(arena))
        }
    }

    /// Get Vec<u8> in big-endian order, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn to_be_bytes(&self) -> Vec<u8> {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            unsafe { self.to_be_bytes_stack() }
        } else {
            Arena::with_current(|arena| self.to_be_bytes_with_arena(arena))
        }
    }

    /// Get Vec<u8> in little-endian order, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn to_le_bytes(&self) -> Vec<u8> {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            unsafe { self.to_le_bytes_stack() }
        } else {
            Arena::with_current(|arena| self.to_le_bytes_with_arena(arena))
        }
    }

    /// BitSlice view on an indirect atom, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn as_bitslice(&self) -> &BitSlice<u64, Lsb0> {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            unsafe { self.as_bitslice_stack() }
        } else {
            Arena::with_current(|arena| self.as_bitslice_with_arena(arena))
        }
    }

    /// Normalize and convert to direct atom if it will fit, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub unsafe fn normalize_as_atom(&mut self) -> Atom {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            self.normalize_as_atom_stack()
        } else {
            Arena::with_current(|arena| self.normalize_as_atom_with_arena(arena))
        }
    }
}

// Debug impl cannot access arena, so just print raw value
impl fmt::Debug for IndirectAtom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IndirectAtom({:#x})", self.0)
    }
}

/**
 * A cell.
 *
 * A cell is represented by a tagged pointer to a memory buffer with metadata, a word describing
 * the noun which is the cell's head, and a word describing a noun which is the cell's tail, each
 * at a fixed offset.
 */
#[derive(Copy, Clone)]
#[repr(C)]
#[repr(packed(8))]
pub struct Cell(u64);

impl Cell {
    pub unsafe fn from_raw_pointer(ptr: *const CellMemory) -> Self {
        Cell(TaggedPtr::from_stack_ptr(ptr as *const u8, CELL_TAG).raw())
    }

    /// Create a Cell from a PMA offset.
    ///
    /// **IMPORTANT**: This should ONLY be used for PMA-resident data.
    /// Stack data should always use `from_raw_pointer()` (LOCATION_BIT=0).
    /// PMA data uses offset form (LOCATION_BIT=1) and requires the PMA arena
    /// for resolution.
    pub fn from_pma_offset(words: u32) -> Self {
        Cell(TaggedPtr::from_offset(words, CELL_TAG).raw())
    }

    pub unsafe fn to_raw_pointer_with_arena(&self, arena: &Arena) -> *const CellMemory {
        TaggedPtr::from_raw(self.0).resolve_const(CELL_MASK, arena) as *const CellMemory
    }

    /// Get raw pointer, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn to_raw_pointer(&self) -> *const CellMemory {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            ((tagged.payload(CELL_MASK)) << 3) as *const CellMemory
        } else {
            Arena::with_current(|arena| unsafe { self.to_raw_pointer_with_arena(arena) })
        }
    }

    pub unsafe fn to_raw_pointer_mut_with_arena(&mut self, arena: &Arena) -> *mut CellMemory {
        TaggedPtr::from_raw(self.0).resolve_mut(CELL_MASK, arena) as *mut CellMemory
    }

    #[inline(always)]
    pub fn stack_memory_pointer(&self) -> Option<*const CellMemory> {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            Some(((tagged.payload(CELL_MASK)) << 3) as *const CellMemory)
        } else {
            None
        }
    }

    /// Get raw pointer for stack-pointer form cells only
    pub unsafe fn to_raw_pointer_stack(&self) -> *const CellMemory {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            ((tagged.payload(CELL_MASK)) << 3) as *const CellMemory
        } else {
            panic!("expected stack-pointer Cell, got offset instead");
        }
    }

    /// Get head of cell (stack-pointer form only)
    pub unsafe fn head_stack(&self) -> Noun {
        (*self.to_raw_pointer_stack()).head
    }

    /// Get tail of cell (stack-pointer form only)
    pub unsafe fn tail_stack(&self) -> Noun {
        (*self.to_raw_pointer_stack()).tail
    }

    /// Get mutable raw pointer for stack-pointer form cells only
    pub unsafe fn to_raw_pointer_mut_stack(&mut self) -> *mut CellMemory {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            ((tagged.payload(CELL_MASK)) << 3) as *mut CellMemory
        } else {
            panic!("expected stack-pointer Cell, got offset instead");
        }
    }

    /// Get mutable pointer to head (stack-pointer form only)
    pub unsafe fn head_as_mut_stack(mut self) -> *mut Noun {
        &mut (*self.to_raw_pointer_mut_stack()).head as *mut Noun
    }

    /// Get mutable pointer to tail (stack-pointer form only)
    pub unsafe fn tail_as_mut_stack(mut self) -> *mut Noun {
        &mut (*self.to_raw_pointer_mut_stack()).tail as *mut Noun
    }

    pub unsafe fn head_as_mut_with_arena(mut self, arena: &Arena) -> *mut Noun {
        &mut (*self.to_raw_pointer_mut_with_arena(arena)).head as *mut Noun
    }

    pub unsafe fn tail_as_mut_with_arena(mut self, arena: &Arena) -> *mut Noun {
        &mut (*self.to_raw_pointer_mut_with_arena(arena)).tail as *mut Noun
    }

    pub unsafe fn set_forwarding_pointer_with_arena(
        &mut self,
        new_me: *const CellMemory,
        arena: &Arena,
    ) {
        (*self.to_raw_pointer_mut_with_arena(arena)).head = Noun {
            raw: TaggedPtr::from_stack_ptr(new_me as *const u8, FORWARDING_TAG).raw(),
        }
    }

    pub unsafe fn forwarding_pointer_with_arena(&self, arena: &Arena) -> Option<Cell> {
        let head_raw = (*self.to_raw_pointer_with_arena(arena)).head.raw;
        if head_raw & FORWARDING_MASK == FORWARDING_TAG {
            let ptr = TaggedPtr::from_raw(head_raw).resolve_const(FORWARDING_MASK, arena)
                as *const CellMemory;
            Some(Self::from_raw_pointer(ptr))
        } else {
            None
        }
    }

    /// Set forwarding pointer (stack-pointer form only)
    pub unsafe fn set_forwarding_pointer_stack(&mut self, new_me: *const CellMemory) {
        (*self.to_raw_pointer_mut_stack()).head = Noun {
            raw: TaggedPtr::from_stack_ptr(new_me as *const u8, FORWARDING_TAG).raw(),
        }
    }

    /// Set forwarding pointer, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub unsafe fn set_forwarding_pointer(&mut self, new_me: *const CellMemory) {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            self.set_forwarding_pointer_stack(new_me)
        } else {
            Arena::with_current(|arena| self.set_forwarding_pointer_with_arena(new_me, arena))
        }
    }

    /// Get forwarding pointer (stack-pointer form only)
    pub unsafe fn forwarding_pointer_stack(&self) -> Option<Cell> {
        let head_raw = (*self.to_raw_pointer_stack()).head.raw;
        if head_raw & FORWARDING_MASK == FORWARDING_TAG {
            // Forwarding pointers always point to stack addresses
            let ptr = (TaggedPtr::from_raw(head_raw).payload(FORWARDING_MASK) << 3) as *const CellMemory;
            Some(Self::from_raw_pointer(ptr))
        } else {
            None
        }
    }

    /// Get forwarding pointer, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub unsafe fn forwarding_pointer(&self) -> Option<Cell> {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            self.forwarding_pointer_stack()
        } else {
            Arena::with_current(|arena| self.forwarding_pointer_with_arena(arena))
        }
    }

    pub fn new<T: NounAllocator>(allocator: &mut T, head: Noun, tail: Noun) -> Cell {
        unsafe {
            let (cell, memory) = Self::new_raw_mut(allocator);
            (*memory).head = head;
            (*memory).tail = tail;
            cell
        }
    }

    pub fn new_tuple<A: NounAllocator>(allocator: &mut A, tup: &[Noun]) -> Cell {
        if tup.len() < 2 {
            panic!("Cannot create tuple with fewer than 2 elements");
        }

        let len = tup.len();
        let mut cell = Cell::new(allocator, tup[len - 2], tup[len - 1]);
        for i in (0..len - 2).rev() {
            cell = Cell::new(allocator, tup[i], cell.as_noun());
        }
        cell
    }

    pub unsafe fn new_raw_mut<A: NounAllocator>(allocator: &mut A) -> (Cell, *mut CellMemory) {
        let memory = allocator.alloc_cell();
        assert!(
            memory as usize % std::mem::align_of::<CellMemory>() == 0,
            "Memory is not aligned, {} {}",
            memory as usize,
            std::mem::align_of::<CellMemory>()
        );
        (*memory).metadata = 0;
        (Self::from_raw_pointer(memory), memory)
    }

    // TODO: idk about making these owned independently of their parent
    pub fn head_with_arena(&self, arena: &Arena) -> Noun {
        unsafe { (*(self.to_raw_pointer_with_arena(arena))).head }
    }

    /// Get head, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn head(&self) -> Noun {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            unsafe { self.head_stack() }
        } else {
            Arena::with_current(|arena| self.head_with_arena(arena))
        }
    }

    /// Get head using MemContext for pointer resolution.
    /// This is the preferred method for new code.
    #[inline(always)]
    pub fn head_with_mem(&self, mem: &MemContext<'_>) -> Noun {
        let ptr = mem.resolve(self.0, CELL_MASK) as *const CellMemory;
        unsafe { (*ptr).head }
    }

    // TODO: Ditto, etc.
    pub fn tail_with_arena(&self, arena: &Arena) -> Noun {
        unsafe { (*(self.to_raw_pointer_with_arena(arena))).tail }
    }

    /// Get tail, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn tail(&self) -> Noun {
        let tagged = TaggedPtr::from_raw(self.0);
        if tagged.location() == PtrLocation::Stack {
            unsafe { self.tail_stack() }
        } else {
            Arena::with_current(|arena| self.tail_with_arena(arena))
        }
    }

    /// Get tail using MemContext for pointer resolution.
    /// This is the preferred method for new code.
    #[inline(always)]
    pub fn tail_with_mem(&self, mem: &MemContext<'_>) -> Noun {
        let ptr = mem.resolve(self.0, CELL_MASK) as *const CellMemory;
        unsafe { (*ptr).tail }
    }

    /// Get both head and tail using MemContext.
    /// More efficient than calling head_with_mem and tail_with_mem separately.
    #[inline(always)]
    pub fn head_tail_with_mem(&self, mem: &MemContext<'_>) -> (Noun, Noun) {
        let ptr = mem.resolve(self.0, CELL_MASK) as *const CellMemory;
        unsafe { ((*ptr).head, (*ptr).tail) }
    }

    pub fn head_ref_with_arena<'a>(&'a self, arena: &'a Arena) -> &'a Noun {
        unsafe {
            self.to_raw_pointer_with_arena(arena)
                .as_ref()
                .map(|cell| &cell.head)
                .unwrap_or_else(|| panic!("head_ref: invalid pointer"))
        }
    }

    // TODO: Ditto, etc.
    pub fn tail_ref_with_arena<'a>(&'a self, arena: &'a Arena) -> &'a Noun {
        unsafe {
            self.to_raw_pointer_with_arena(arena)
                .as_ref()
                .map(|cell| &cell.tail)
                .unwrap_or_else(|| panic!("head_ref: invalid pointer"))
        }
    }

    pub fn as_allocated(&self) -> Allocated {
        Allocated { cell: *self }
    }

    pub fn as_noun(&self) -> Noun {
        Noun { cell: *self }
    }
}

impl fmt::Debug for Cell {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        // Debug impl cannot access arena, so just print raw value
        write!(f, "Cell({:#x})", self.0)
    }
}

pub struct FullDebugCell<'a>(pub &'a Cell, pub &'a Arena);

impl fmt::Debug for FullDebugCell<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fn do_fmt(cell: &Cell, arena: &Arena, brackets: bool, f: &mut fmt::Formatter) -> fmt::Result {
            if brackets {
                write!(f, "[")?;
            }
            match cell.head_with_arena(arena).as_cell() {
                Ok(head_cell) => {
                    do_fmt(&head_cell, arena, true, f)?;
                    write!(f, " ")?;
                }
                Err(_) => {
                    write!(f, "{:?} ", cell.head_with_arena(arena))?;
                }
            }
            match cell.tail_with_arena(arena).as_cell() {
                Ok(next_cell) => {
                    do_fmt(&next_cell, arena, false, f)?;
                }
                Err(_) => {
                    write!(f, "{:?}", cell.tail_with_arena(arena))?;
                }
            }
            if brackets {
                write!(f, "]")?;
            }
            Ok(())
        }

        do_fmt(&*self.0, self.1, true, f)?;
        Ok(())
    }
}

// Render a path which is a linked-list of cells of of atoms (direct and indirect strings)
pub struct DebugPath<'a>(pub &'a Cell, pub &'a Arena);

impl fmt::Debug for DebugPath<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "[")?;
        let mut cell = *self.0;
        let arena = self.1;
        loop {
            let head = cell.head_with_arena(arena).as_atom();
            match head {
                Ok(atom) => {
                    if atom.is_direct() {
                        write!(f, "{:?}", atom.as_direct())?;
                    } else if atom.is_indirect() {
                        write!(f, "{:?}", atom.as_indirect())?;
                    } else {
                        write!(f, "{atom:?}")?;
                    }
                }
                Err(_) => {
                    write!(f, "ERR, not atom")?;
                }
            }
            match cell.tail_with_arena(arena).as_cell() {
                Ok(next_cell) => {
                    write!(f, " ")?;
                    cell = next_cell;
                }
                Err(_) => {
                    write!(f, " {:?}]", cell.tail_with_arena(arena))?;
                    break;
                }
            }
        }
        Ok(())
    }
}

// Axis iteration helpers for direct axes (u64)
pub struct DirectAxisIterator {
    axis: u64,
    cursor: usize,
}

impl DirectAxisIterator {
    #[inline(always)]
    pub fn new(axis: u64) -> Option<Self> {
        if axis == 0 {
            None
        } else {
            let cursor = if axis == 1 {
                0
            } else {
                63 - axis.leading_zeros() as usize
            };
            Some(DirectAxisIterator { axis, cursor })
        }
    }

    #[inline(always)]
    pub fn next(&mut self) -> Option<bool> {
        if self.cursor == 0 {
            None
        } else {
            self.cursor -= 1;
            Some(((self.axis >> self.cursor) & 1) != 0)
        }
    }
}

// Axis iteration helpers for indirect axes (slice of u64)
pub struct IndirectAxisIterator<'a> {
    words: &'a [u64],
    cursor: usize,
}

impl<'a> IndirectAxisIterator<'a> {
    #[inline(always)]
    pub fn new(words: &'a [u64]) -> Option<Self> {
        if words.is_empty() {
            return None;
        }

        // Find highest bit in the axis
        let mut highest_word_idx = words.len() - 1;
        while highest_word_idx > 0 && words[highest_word_idx] == 0 {
            highest_word_idx -= 1;
        }

        let highest_word = words[highest_word_idx];
        if highest_word == 0 {
            return None;
        }

        let highest_bit_in_word = 63 - highest_word.leading_zeros() as usize;
        let cursor = (highest_word_idx << 6) + highest_bit_in_word;

        Some(IndirectAxisIterator { words, cursor })
    }

    #[inline(always)]
    pub fn next(&mut self) -> Option<bool> {
        if self.cursor == 0 {
            None
        } else {
            self.cursor -= 1;
            let word_idx = self.cursor >> 6;
            let bit_idx = self.cursor & 63;
            Some(((self.words[word_idx] >> bit_idx) & 1) != 0)
        }
    }
}

// Direct axis traversal without bitvec - for u64 axes (auto-dispatch based on LOCATION_BIT)
#[inline(always)]
fn slot_direct(cell: &Cell, axis: u64) -> Result<Noun> {
    if axis == 0 {
        return Err(Error::NotRepresentable);
    }
    if axis == 1 {
        return Ok(cell.as_noun());
    }

    let highest = 63 - axis.leading_zeros() as usize;
    let mut current = *cell;
    let mut noun = current.as_noun();

    for idx in (0..highest).rev() {
        let descend_tail = ((axis >> idx) & 1) != 0;
        // Use auto-dispatch to_raw_pointer which handles both stack and offset forms
        let memory = current.to_raw_pointer();
        noun = unsafe {
            if descend_tail {
                (*memory).tail
            } else {
                (*memory).head
            }
        };

        if idx != 0 {
            if noun.is_cell() {
                current = unsafe { noun.cell };
            } else {
                return Err(Error::NotRepresentable);
            }
        }
    }

    Ok(noun)
}

impl Slots for Cell {}

// Indirect axis traversal - for large axes stored in word slices (auto-dispatch based on LOCATION_BIT)
#[inline(always)]
fn slot_indirect(cell: &Cell, words: &[u64]) -> Result<Noun> {
    if words.is_empty() {
        return Err(Error::NotRepresentable);
    }

    // Find highest bit in the axis
    let mut highest_word_idx = words.len() - 1;
    while highest_word_idx > 0 && words[highest_word_idx] == 0 {
        highest_word_idx -= 1;
    }

    let highest_word = words[highest_word_idx];
    if highest_word == 0 {
        return Err(Error::NotRepresentable);
    }

    let highest_bit_in_word = 63 - highest_word.leading_zeros() as usize;
    let highest = (highest_word_idx << 6) + highest_bit_in_word;

    if highest == 0 {
        return Ok(cell.as_noun());
    }

    let mut current = *cell;
    let mut noun = current.as_noun();
    let mut idx = highest;

    while idx != 0 {
        idx -= 1;
        let word_idx = idx >> 6;
        let bit_idx = idx & 63;
        let descend_tail = ((words[word_idx] >> bit_idx) & 1) != 0;

        // Use auto-dispatch to_raw_pointer which handles both stack and offset forms
        let memory = current.to_raw_pointer();
        noun = unsafe {
            if descend_tail {
                (*memory).tail
            } else {
                (*memory).head
            }
        };

        if idx != 0 {
            if noun.is_cell() {
                current = unsafe { noun.cell };
            } else {
                return Err(Error::NotRepresentable);
            }
        }
    }

    Ok(noun)
}

// Arena-aware versions of slot functions

/// Direct axis traversal with explicit arena - for u64 axes
#[inline(always)]
fn slot_direct_with_arena(cell: &Cell, axis: u64, arena: &Arena) -> Result<Noun> {
    if axis == 0 {
        return Err(Error::NotRepresentable);
    }
    if axis == 1 {
        return Ok(cell.as_noun());
    }

    let highest = 63 - axis.leading_zeros() as usize;
    let mut current = *cell;
    let mut noun = current.as_noun();

    for idx in (0..highest).rev() {
        let descend_tail = ((axis >> idx) & 1) != 0;
        let memory = unsafe { current.to_raw_pointer_with_arena(arena) };
        noun = unsafe {
            if descend_tail {
                (*memory).tail
            } else {
                (*memory).head
            }
        };

        if idx != 0 {
            if noun.is_cell() {
                current = unsafe { noun.cell };
            } else {
                return Err(Error::NotRepresentable);
            }
        }
    }

    Ok(noun)
}

/// Indirect axis traversal with explicit arena - for large axes stored in word slices
#[inline(always)]
fn slot_indirect_with_arena(cell: &Cell, words: &[u64], arena: &Arena) -> Result<Noun> {
    if words.is_empty() {
        return Err(Error::NotRepresentable);
    }

    // Find highest bit in the axis
    let mut highest_word_idx = words.len() - 1;
    while highest_word_idx > 0 && words[highest_word_idx] == 0 {
        highest_word_idx -= 1;
    }

    let highest_word = words[highest_word_idx];
    if highest_word == 0 {
        return Err(Error::NotRepresentable);
    }

    let highest_bit_in_word = 63 - highest_word.leading_zeros() as usize;
    let highest = (highest_word_idx << 6) + highest_bit_in_word;

    if highest == 0 {
        return Ok(cell.as_noun());
    }

    let mut current = *cell;
    let mut noun = current.as_noun();
    let mut idx = highest;

    while idx != 0 {
        idx -= 1;
        let word_idx = idx >> 6;
        let bit_idx = idx & 63;
        let descend_tail = ((words[word_idx] >> bit_idx) & 1) != 0;

        let memory = unsafe { current.to_raw_pointer_with_arena(arena) };
        noun = unsafe {
            if descend_tail {
                (*memory).tail
            } else {
                (*memory).head
            }
        };

        if idx != 0 {
            if noun.is_cell() {
                current = unsafe { noun.cell };
            } else {
                return Err(Error::NotRepresentable);
            }
        }
    }

    Ok(noun)
}

impl private::RawSlots for Cell {
    #[inline(always)]
    fn raw_slot_direct(&self, axis: u64) -> Result<Noun> {
        slot_direct(self, axis)
    }

    #[inline(always)]
    fn raw_slot_indirect(&self, axis: &[u64]) -> Result<Noun> {
        slot_indirect(self, axis)
    }
}

impl Cell {
    /// Retrieve component Noun at given axis using explicit arena
    #[inline(always)]
    pub fn slot_with_arena(&self, axis: u64, arena: &Arena) -> Result<Noun> {
        slot_direct_with_arena(self, axis, arena)
    }

    /// Retrieve component Noun at axis given as Atom using explicit arena
    #[inline(always)]
    pub fn slot_atom_with_arena(&self, atom: Atom, arena: &Arena) -> Result<Noun> {
        match atom.as_either() {
            either::Left(direct) => slot_direct_with_arena(self, direct.data(), arena),
            either::Right(indirect) => slot_indirect_with_arena(self, indirect.as_slice_with_arena(arena), arena),
        }
    }
}

/**
 * Memory representation of the contents of a cell
 */
#[derive(Copy, Clone)]
#[repr(C)]
#[repr(packed(8))]
pub struct CellMemory {
    pub metadata: u64,
    pub head: Noun,
    pub tail: Noun,
}

#[derive(Copy, Clone)]
#[repr(C)]
#[repr(packed(8))]
pub union Atom {
    pub(crate) raw: u64,
    direct: DirectAtom,
    indirect: IndirectAtom,
}

impl Atom {
    pub fn new<A: NounAllocator>(allocator: &mut A, value: u64) -> Atom {
        if value <= DIRECT_MAX {
            unsafe { DirectAtom::new_unchecked(value).as_atom() }
        } else {
            unsafe { IndirectAtom::new_raw(allocator, 1, &value).as_atom() }
        }
    }

    // to_le_bytes and new_raw are copies.  We should be able to do this completely without copies
    // if we integrate with ibig properly.
    pub fn from_ubig<A: NounAllocator>(allocator: &mut A, big: &UBig) -> Atom {
        let bit_size = big.bit_len();
        let buffer = big.to_le_bytes_stack();
        if bit_size < 64 {
            let mut value = 0u64;
            for i in (0..bit_size).step_by(8) {
                value |= (buffer[i / 8] as u64) << i;
            }
            unsafe { DirectAtom::new_unchecked(value).as_atom() }
        } else {
            let byte_size = (big.bit_len() + 7) >> 3;
            unsafe { IndirectAtom::new_raw_bytes(allocator, byte_size, buffer.as_ptr()).as_atom() }
        }
    }

    pub fn is_direct(&self) -> bool {
        unsafe { is_direct_atom(self.raw) }
    }

    pub fn is_indirect(&self) -> bool {
        unsafe { is_indirect_atom(self.raw) }
    }

    pub fn is_normalized_with_arena(&self, arena: &Arena) -> bool {
        unsafe {
            if let Some(indirect) = self.indirect() {
                if (indirect.size_with_arena(arena) == 1
                    && *indirect.data_pointer_with_arena(arena) <= DIRECT_MAX)
                    || *indirect
                        .data_pointer_with_arena(arena)
                        .add(indirect.size_with_arena(arena) - 1)
                        == 0
                {
                    return false;
                }
            } // nothing to do for direct atom
        };

        true
    }

    pub fn as_direct(&self) -> Result<DirectAtom> {
        if self.is_direct() {
            unsafe { Ok(self.direct) }
        } else {
            Err(Error::NotDirectAtom)
        }
    }

    pub fn as_indirect(&self) -> Result<IndirectAtom> {
        if self.is_indirect() {
            unsafe { Ok(self.indirect) }
        } else {
            Err(Error::NotIndirectAtom)
        }
    }

    pub fn as_either(&self) -> Either<DirectAtom, IndirectAtom> {
        if self.is_indirect() {
            unsafe { Right(self.indirect) }
        } else {
            unsafe { Left(self.direct) }
        }
    }

    pub fn as_noun(self) -> Noun {
        Noun { atom: self }
    }

    /// Returns a slice of bytes in native-endian order. Currently, Sword only supports
    /// little-endian machines, so this will return little-endian.
    pub fn as_ne_bytes_with_arena(&self, arena: &Arena) -> &[u8] {
        if self.is_direct() {
            unsafe { self.direct.as_ne_bytes() }
        } else {
            unsafe { self.indirect.as_ne_bytes_with_arena(arena) }
        }
    }

    /// Returns a slice of bytes (stack-pointer form only)
    pub unsafe fn as_ne_bytes_stack(&self) -> &[u8] {
        if self.is_direct() {
            self.direct.as_ne_bytes()
        } else {
            self.indirect.as_ne_bytes_stack()
        }
    }

    /// Returns Vec<u8> in native-endian order
    pub fn to_ne_bytes_with_arena(&self, arena: &Arena) -> Vec<u8> {
        if self.is_direct() {
            unsafe { self.direct.to_ne_bytes() }
        } else {
            unsafe { self.indirect.to_ne_bytes_with_arena(arena) }
        }
    }

    /// Returns Vec<u8> in big-endian order
    pub fn to_be_bytes_with_arena(self, arena: &Arena) -> Vec<u8> {
        if self.is_direct() {
            unsafe { self.direct.to_be_bytes() }
        } else {
            unsafe { self.indirect.to_be_bytes_with_arena(arena) }
        }
    }

    /// Returns Vec<u8> in big-endian order (stack-pointer form only)
    pub unsafe fn to_be_bytes_stack(self) -> Vec<u8> {
        if self.is_direct() {
            self.direct.to_be_bytes()
        } else {
            self.indirect.to_be_bytes_stack()
        }
    }

    /// Returns Vec<u8> in little-endian order
    pub fn to_le_bytes_with_arena(self, arena: &Arena) -> Vec<u8> {
        if self.is_direct() {
            unsafe { self.direct.to_le_bytes() }
        } else {
            unsafe { self.indirect.to_le_bytes_with_arena(arena) }
        }
    }

    pub fn as_u64_with_arena(self, arena: &Arena) -> Result<u64> {
        if self.is_direct() {
            Ok(unsafe { self.direct.data() })
        } else {
            unsafe { self.indirect.as_u64_with_arena(arena) }
        }
    }

    /// Get as u64 for stack-pointer form atoms only
    pub unsafe fn as_u64_stack(self) -> Result<u64> {
        if self.is_direct() {
            Ok(self.direct.data())
        } else {
            self.indirect.as_u64_stack()
        }
    }

    pub fn as_bool(self) -> Result<bool> {
        if self.is_direct() {
            Ok(unsafe { self.direct.data() == 0 })
        } else {
            Err(Error::NotRepresentable)
        }
    }

    /** Produce a SoftFloat-compatible ordered pair of 64-bit words */
    pub unsafe fn as_u64_pair_with_arena(self, arena: &Arena) -> Result<[u64; 2]> {
        if self.is_direct() {
            let u128_array = &mut [0u64; 2];
            u128_array[0] = self.as_direct()?.data();
            u128_array[1] = 0x0_u64;
            Ok(*u128_array)
        } else {
            unsafe { self.indirect.as_u64_pair_with_arena(arena) }
        }
    }

    pub fn as_bitslice_with_arena(&self, arena: &Arena) -> &BitSlice<u64, Lsb0> {
        if self.is_indirect() {
            unsafe { self.indirect.as_bitslice_with_arena(arena) }
        } else {
            unsafe { self.direct.as_bitslice() }
        }
    }

    /// BitSlice view for stack-pointer form atoms only
    pub unsafe fn as_bitslice_stack(&self) -> &BitSlice<u64, Lsb0> {
        if self.is_indirect() {
            self.indirect.as_bitslice_stack()
        } else {
            self.direct.as_bitslice()
        }
    }

    pub fn as_bitslice_mut_with_arena(&mut self, arena: &Arena) -> &mut BitSlice<u64, Lsb0> {
        if self.is_indirect() {
            unsafe { self.indirect.as_bitslice_mut_with_arena(arena) }
        } else {
            unsafe { self.direct.as_bitslice_mut() }
        }
    }

    pub fn as_ubig_with_arena<S: Stack>(self, stack: &mut S, arena: &Arena) -> UBig {
        if self.is_indirect() {
            unsafe { self.indirect.as_ubig_with_arena(stack, arena) }
        } else {
            unsafe { self.direct.as_ubig(stack) }
        }
    }

    pub fn direct(&self) -> Option<DirectAtom> {
        if self.is_direct() {
            unsafe { Some(self.direct) }
        } else {
            None
        }
    }

    pub fn indirect(&self) -> Option<IndirectAtom> {
        if self.is_indirect() {
            unsafe { Some(self.indirect) }
        } else {
            None
        }
    }

    pub fn size_with_arena(&self, arena: &Arena) -> usize {
        match self.as_either() {
            Left(_direct) => 1,
            Right(indirect) => indirect.size_with_arena(arena),
        }
    }

    pub fn bit_size_with_arena(&self, arena: &Arena) -> usize {
        match self.as_either() {
            Left(direct) => direct.bit_size(),
            Right(indirect) => indirect.bit_size_with_arena(arena),
        }
    }

    pub fn data_pointer_with_arena(&self, arena: &Arena) -> *const u64 {
        match self.as_either() {
            Left(_direct) => (self as *const Atom) as *const u64,
            Right(indirect) => indirect.data_pointer_with_arena(arena),
        }
    }

    /// Get raw data pointer for stack-pointer form atoms only
    pub unsafe fn data_pointer_stack(&self) -> *const u64 {
        match self.as_either() {
            Left(_direct) => (self as *const Atom) as *const u64,
            Right(indirect) => indirect.data_pointer_stack(),
        }
    }

    pub unsafe fn normalize_with_arena(&mut self, arena: &Arena) -> Atom {
        if self.is_indirect() {
            self.indirect.normalize_as_atom_with_arena(arena)
        } else {
            *self
        }
    }

    /** Make an atom from a raw u64
     *
     * # Safety
     *
     * Note that the [u64] parameter is *not*, in general, the value of the atom!
     *
     * In particular, anything with the high bit set will be treated as a tagged pointer.
     * This method is only to be used to restore an atom from the raw [u64] representation
     * returned by [Noun::as_raw], and should only be used if we are sure the restored noun is in
     * fact an atom.
     */
    pub unsafe fn from_raw(raw: u64) -> Atom {
        Atom { raw }
    }

    // Auto-dispatch methods that check LOCATION_BIT and use thread-local arena for PMA

    /// Get as u64, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn as_u64(self) -> Result<u64> {
        if self.is_direct() {
            Ok(unsafe { self.direct.data() })
        } else {
            unsafe { self.indirect.as_u64() }
        }
    }

    /// Get bytes, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn as_ne_bytes(&self) -> &[u8] {
        if self.is_direct() {
            unsafe { self.direct.as_ne_bytes() }
        } else {
            unsafe { self.indirect.as_ne_bytes() }
        }
    }

    /// Get Vec<u8> in native-endian order, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn to_ne_bytes(&self) -> Vec<u8> {
        if self.is_direct() {
            unsafe { self.direct.to_ne_bytes() }
        } else {
            unsafe { self.indirect.to_ne_bytes() }
        }
    }

    /// Get Vec<u8> in big-endian order, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn to_be_bytes(self) -> Vec<u8> {
        if self.is_direct() {
            unsafe { self.direct.to_be_bytes() }
        } else {
            unsafe { self.indirect.to_be_bytes() }
        }
    }

    /// Get Vec<u8> in little-endian order, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn to_le_bytes(self) -> Vec<u8> {
        if self.is_direct() {
            unsafe { self.direct.to_le_bytes() }
        } else {
            unsafe { self.indirect.to_le_bytes() }
        }
    }

    /// BitSlice view on an atom, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn as_bitslice(&self) -> &BitSlice<u64, Lsb0> {
        if self.is_indirect() {
            unsafe { self.indirect.as_bitslice() }
        } else {
            unsafe { self.direct.as_bitslice() }
        }
    }

    /// Size of atom in 64-bit words, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn size(&self) -> usize {
        match self.as_either() {
            Left(_direct) => 1,
            Right(indirect) => indirect.size(),
        }
    }

    /// Bit size of atom, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn bit_size(&self) -> usize {
        match self.as_either() {
            Left(direct) => direct.bit_size(),
            Right(indirect) => indirect.bit_size(),
        }
    }

    /// Data pointer, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn data_pointer(&self) -> *const u64 {
        match self.as_either() {
            Left(_direct) => (self as *const Atom) as *const u64,
            Right(indirect) => indirect.data_pointer(),
        }
    }

    /// Get as UBig, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn as_ubig<S: Stack>(self, stack: &mut S) -> UBig {
        if self.is_indirect() {
            unsafe { self.indirect.as_ubig(stack) }
        } else {
            unsafe { self.direct.as_ubig(stack) }
        }
    }

    /// Produce a SoftFloat-compatible ordered pair of 64-bit words,
    /// auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub unsafe fn as_u64_pair(self) -> Result<[u64; 2]> {
        if self.is_direct() {
            let u128_array = &mut [0u64; 2];
            u128_array[0] = self.as_direct()?.data();
            u128_array[1] = 0x0_u64;
            Ok(*u128_array)
        } else {
            self.indirect.as_u64_pair()
        }
    }
}

impl fmt::Debug for Atom {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.as_noun().fmt(f)
    }
}

#[derive(Copy, Clone)]
#[repr(C)]
#[repr(packed(8))]
pub union Allocated {
    raw: u64,
    indirect: IndirectAtom,
    cell: Cell,
}

impl Allocated {
    pub fn is_indirect(&self) -> bool {
        unsafe { is_indirect_atom(self.raw) }
    }

    pub fn is_cell(&self) -> bool {
        unsafe { is_cell(self.raw) }
    }

    /// Returns true if this allocated noun is in stack-pointer form (LOCATION_BIT = 0)
    #[inline]
    pub fn is_stack_allocated(&self) -> bool {
        unsafe { self.raw & LOCATION_BIT == 0 }
    }

    pub unsafe fn to_raw_pointer_with_arena(&self, arena: &Arena) -> *const u64 {
        let tagged = TaggedPtr::from_raw(self.raw);
        if self.is_indirect() {
            tagged.resolve_const(INDIRECT_MASK, arena) as *const u64
        } else {
            tagged.resolve_const(CELL_MASK, arena) as *const u64
        }
    }

    pub unsafe fn to_raw_pointer_mut_with_arena(&mut self, arena: &Arena) -> *mut u64 {
        let tagged = TaggedPtr::from_raw(self.raw);
        if self.is_indirect() {
            tagged.resolve_mut(INDIRECT_MASK, arena) as *mut u64
        } else {
            tagged.resolve_mut(CELL_MASK, arena) as *mut u64
        }
    }

    unsafe fn const_to_raw_pointer_mut_with_arena(self, arena: &Arena) -> *mut u64 {
        let tagged = TaggedPtr::from_raw(self.raw);
        if self.is_indirect() {
            tagged.resolve_mut(INDIRECT_MASK, arena) as *mut u64
        } else {
            tagged.resolve_mut(CELL_MASK, arena) as *mut u64
        }
    }

    /// Get raw pointer for stack-pointer form allocated nouns only
    pub unsafe fn to_raw_pointer_stack(&self) -> *const u64 {
        let tagged = TaggedPtr::from_raw(self.raw);
        if tagged.location() != PtrLocation::Stack {
            panic!("expected stack-pointer Allocated, got offset instead");
        }
        if self.is_indirect() {
            (tagged.payload(INDIRECT_MASK) << 3) as *const u64
        } else {
            (tagged.payload(CELL_MASK) << 3) as *const u64
        }
    }

    /// Get forwarding pointer (stack-pointer form only)
    pub unsafe fn forwarding_pointer_stack(&self) -> Option<Allocated> {
        match self.as_either() {
            Left(indirect) => indirect
                .forwarding_pointer_stack()
                .map(|i| i.as_allocated()),
            Right(cell) => cell
                .forwarding_pointer_stack()
                .map(|c| c.as_allocated()),
        }
    }

    pub unsafe fn forwarding_pointer_with_arena(&self, arena: &Arena) -> Option<Allocated> {
        match self.as_either() {
            Left(indirect) => indirect
                .forwarding_pointer_with_arena(arena)
                .map(|i| i.as_allocated()),
            Right(cell) => cell
                .forwarding_pointer_with_arena(arena)
                .map(|c| c.as_allocated()),
        }
    }

    /// Get forwarding pointer, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub unsafe fn forwarding_pointer(&self) -> Option<Allocated> {
        match self.as_either() {
            Left(indirect) => indirect.forwarding_pointer().map(|i| i.as_allocated()),
            Right(cell) => cell.forwarding_pointer().map(|c| c.as_allocated()),
        }
    }

    pub unsafe fn get_metadata_with_arena(&self, arena: &Arena) -> u64 {
        *(self.to_raw_pointer_with_arena(arena))
    }

    pub unsafe fn set_metadata_with_arena(&mut self, metadata: u64, arena: &Arena) {
        *(self.const_to_raw_pointer_mut_with_arena(arena)) = metadata;
    }

    /// Get metadata for stack-pointer form allocated nouns only
    pub unsafe fn get_metadata_stack(&self) -> u64 {
        *(self.to_raw_pointer_stack())
    }

    /// Get metadata, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    #[inline(always)]
    pub fn get_metadata(&self) -> u64 {
        if self.is_stack_allocated() {
            unsafe { self.get_metadata_stack() }
        } else {
            Arena::with_current(|arena| unsafe { self.get_metadata_with_arena(arena) })
        }
    }

    /// Set metadata for stack-pointer form allocated nouns only
    pub unsafe fn set_metadata_stack(&mut self, metadata: u64) {
        let tagged = TaggedPtr::from_raw(self.raw);
        if tagged.location() != PtrLocation::Stack {
            panic!("expected stack-pointer Allocated, got offset instead");
        }
        let ptr = if self.is_indirect() {
            (tagged.payload(INDIRECT_MASK) << 3) as *mut u64
        } else {
            (tagged.payload(CELL_MASK) << 3) as *mut u64
        };
        *ptr = metadata;
    }

    pub fn as_either(&self) -> Either<IndirectAtom, Cell> {
        if self.is_indirect() {
            unsafe { Left(self.indirect) }
        } else {
            unsafe { Right(self.cell) }
        }
    }

    pub fn as_ref_either(&self) -> Either<&IndirectAtom, &Cell> {
        if self.is_indirect() {
            unsafe { Left(&self.indirect) }
        } else {
            unsafe { Right(&self.cell) }
        }
    }

    pub fn cell(&self) -> Option<Cell> {
        if self.is_cell() {
            unsafe { Some(self.cell) }
        } else {
            None
        }
    }

    pub fn as_noun(&self) -> Noun {
        Noun { allocated: *self }
    }

    /// Get cached mug from allocated noun metadata, auto-dispatching based on LOCATION_BIT.
    /// Uses thread-local arena for PMA pointers.
    pub fn get_cached_mug(self: Allocated) -> Option<u32> {
        let bottom_metadata = self.get_metadata() as u32 & 0x7FFFFFFF; // magic number: LS 31 bits
        if bottom_metadata > 0 {
            Some(bottom_metadata)
        } else {
            None
        }
    }
}

impl fmt::Debug for Allocated {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.as_noun().fmt(f)
    }
}

#[derive(Copy, Clone)]
#[repr(C)]
#[repr(packed(8))]
pub union Noun {
    pub(crate) raw: u64,
    direct: DirectAtom,
    indirect: IndirectAtom,
    atom: Atom,
    cell: Cell,
    allocated: Allocated,
}

impl Noun {
    pub fn is_none(self) -> bool {
        unsafe { self.raw == u64::MAX }
    }

    pub fn is_direct(&self) -> bool {
        unsafe { is_direct_atom(self.raw) }
    }

    pub fn is_indirect(&self) -> bool {
        unsafe { is_indirect_atom(self.raw) }
    }

    pub fn is_atom(&self) -> bool {
        self.is_direct() || self.is_indirect()
    }

    pub fn is_allocated(&self) -> bool {
        self.is_indirect() || self.is_cell()
    }

    #[inline]
    pub fn is_stack_allocated(&self) -> bool {
        self.is_allocated() && unsafe { self.as_raw() & LOCATION_BIT == 0 }
    }

    pub fn is_cell(&self) -> bool {
        unsafe { is_cell(self.raw) }
    }

    pub fn as_direct(&self) -> Result<DirectAtom> {
        if self.is_direct() {
            unsafe { Ok(self.direct) }
        } else {
            Err(Error::NotDirectAtom)
        }
    }

    pub fn as_indirect(&self) -> Result<IndirectAtom> {
        if self.is_indirect() {
            unsafe { Ok(self.indirect) }
        } else {
            Err(Error::NotIndirectAtom)
        }
    }

    pub fn as_cell(&self) -> Result<Cell> {
        if self.is_cell() {
            unsafe { Ok(self.cell) }
        } else {
            Err(Error::NotCell)
        }
    }

    pub fn as_atom(&self) -> Result<Atom> {
        if self.is_atom() {
            unsafe { Ok(self.atom) }
        } else {
            Err(Error::NotAtom)
        }
    }

    pub fn as_allocated(&self) -> Result<Allocated> {
        if self.is_allocated() {
            unsafe { Ok(self.allocated) }
        } else {
            Err(Error::NotAllocated)
        }
    }

    pub fn as_either_atom_cell(&self) -> Either<Atom, Cell> {
        if self.is_cell() {
            unsafe { Right(self.cell) }
        } else {
            unsafe { Left(self.atom) }
        }
    }

    pub fn as_either_direct_allocated(self) -> Either<DirectAtom, Allocated> {
        if self.is_direct() {
            unsafe { Left(self.direct) }
        } else {
            unsafe { Right(self.allocated) }
        }
    }

    pub fn as_ref_either_direct_allocated(&self) -> Either<&DirectAtom, &Allocated> {
        if self.is_direct() {
            unsafe { Left(&self.direct) }
        } else {
            unsafe { Right(&self.allocated) }
        }
    }

    pub fn as_ref_mut_either_direct_allocated(
        &mut self,
    ) -> Either<&mut DirectAtom, &mut Allocated> {
        if self.is_direct() {
            unsafe { Left(&mut self.direct) }
        } else {
            unsafe { Right(&mut self.allocated) }
        }
    }

    pub fn atom(&self) -> Option<Atom> {
        if self.is_atom() {
            unsafe { Some(self.atom) }
        } else {
            None
        }
    }

    pub fn cell(&self) -> Option<Cell> {
        if self.is_cell() {
            unsafe { Some(self.cell) }
        } else {
            None
        }
    }

    pub fn direct(&self) -> Option<DirectAtom> {
        if self.is_direct() {
            unsafe { Some(self.direct) }
        } else {
            None
        }
    }

    pub fn indirect(&self) -> Option<IndirectAtom> {
        if self.is_indirect() {
            unsafe { Some(self.indirect) }
        } else {
            None
        }
    }

    pub fn allocated(&self) -> Option<Allocated> {
        if self.is_allocated() {
            unsafe { Some(self.allocated) }
        } else {
            None
        }
    }

    /** Are these the same noun */
    pub unsafe fn raw_equals(&self, other: &Noun) -> bool {
        self.raw == other.raw
    }

    pub unsafe fn as_raw(&self) -> u64 {
        self.raw
    }

    pub unsafe fn from_raw(raw: u64) -> Noun {
        Noun { raw }
    }

    /// Retrieve component Noun at given axis using explicit arena
    ///
    /// For atoms, axis 1 returns self, any other axis fails.
    /// For cells, traverses based on axis bits (2=head, 3=tail, etc.)
    #[inline(always)]
    pub fn slot_with_arena(&self, axis: u64, arena: &Arena) -> Result<Noun> {
        match self.as_either_atom_cell() {
            Right(cell) => cell.slot_with_arena(axis, arena),
            Left(_atom) => {
                if axis == 1 {
                    Ok(*self)
                } else {
                    Err(Error::NotCell)
                }
            }
        }
    }

    /// Retrieve component Noun at axis given as Atom using explicit arena
    #[inline(always)]
    pub fn slot_atom_with_arena(&self, atom: Atom, arena: &Arena) -> Result<Noun> {
        match self.as_either_atom_cell() {
            Right(cell) => cell.slot_atom_with_arena(atom, arena),
            Left(_atom) => {
                match atom.as_either() {
                    Left(direct) => {
                        if direct.data() == 1 {
                            Ok(*self)
                        } else {
                            Err(Error::NotCell)
                        }
                    }
                    Right(indirect) => {
                        let words = indirect.as_slice_with_arena(arena);
                        if words.len() == 1 && words[0] == 1 {
                            Ok(*self)
                        } else if words.is_empty() || (words.len() == 1 && words[0] == 0) {
                            Err(Error::NotRepresentable)
                        } else {
                            Err(Error::NotCell)
                        }
                    }
                }
            }
        }
    }

    /** Produce the total size of a noun, in words
     *
     * This counts the total size, see mass_frame() to count the size in the current frame.
     */
    pub fn mass(self) -> usize {
        unsafe {
            let res = self.mass_wind(&|_| true);
            self.mass_unwind(&|_| true);
            res
        }
    }

    /** Produce the size of a noun in the current frame, in words */
    pub fn mass_frame(self, stack: &NockStack) -> usize {
        unsafe {
            let res = self.mass_wind(&|p| stack.is_in_frame(p));
            self.mass_unwind(&|p| stack.is_in_frame(p));
            res
        }
    }

    /** Produce the total size of a noun, in words
     *
     * `inside` determines whether a pointer should be counted.  If it returns false, we also do
     * not recurse into that noun if it is a cell.  See mass_frame() for an example.
     *
     * This "winds up" the mass calculation, which includes setting the 32nd bit of the metadata to
     * mark nouns that have already been counted.
     *
     * This is unsafe because you *must* call mass_unwind() with the same `inside` function to
     * unmark the noun.  This is exposed so that you can count several the "mass difference" of a
     * series of nouns.  If you call this twice consecutively, the first result will be the mass of
     * the first noun, and the second will be the mass of the second noun minus the overlap with
     * the first noun.
     */
    pub unsafe fn mass_wind(self, inside: &impl Fn(*const u64) -> bool) -> usize {
        if let Ok(mut allocated) = self.as_allocated() {
            if inside(allocated.to_raw_pointer_stack()) {
                if allocated.get_metadata_stack() & (1 << 32) == 0 {
                    allocated.set_metadata_stack(allocated.get_metadata_stack() | (1 << 32));
                    match allocated.as_either() {
                        Left(indirect) => indirect.size_stack() + 2,
                        Right(cell) => {
                            word_size_of::<CellMemory>()
                                + cell.head_stack().mass_wind(inside)
                                + cell.tail_stack().mass_wind(inside)
                        }
                    }
                } else {
                    0
                }
            } else {
                0
            }
        } else {
            0
        }
    }

    /** See mass_wind() */
    pub unsafe fn mass_unwind(self, inside: &impl Fn(*const u64) -> bool) {
        if let Ok(mut allocated) = self.as_allocated() {
            if inside(allocated.to_raw_pointer_stack()) {
                allocated.set_metadata_stack(allocated.get_metadata_stack() & !(1 << 32));
                if let Right(cell) = allocated.as_either() {
                    cell.head_stack().mass_unwind(inside);
                    cell.tail_stack().mass_unwind(inside);
                }
            }
        }
    }
}

impl fmt::Debug for Noun {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        unsafe {
            if self.is_direct() {
                write!(f, "{:?}", self.direct)
            } else if self.is_indirect() {
                write!(f, "{:?}", self.indirect)
            } else if self.is_cell() {
                write!(f, "{:?}", self.cell)
            } else if self.allocated.forwarding_pointer_stack().is_some() {
                write!(
                    f,
                    "Noun::Forwarding({:?})",
                    self.allocated
                        .forwarding_pointer_stack()
                        .unwrap_or_else(|| panic!(
                            "Panicked at {}:{} (git sha: {:?})",
                            file!(),
                            line!(),
                            option_env!("GIT_SHA")
                        ))
                )
            } else {
                write!(f, "Noun::Unknown({:x})", self.raw)
            }
        }
    }
}

impl Slots for Noun {}
impl private::RawSlots for Noun {
    #[inline(always)]
    fn raw_slot_direct(&self, axis: u64) -> Result<Noun> {
        match self.as_either_atom_cell() {
            Right(cell) => cell.raw_slot_direct(axis),
            Left(_atom) => {
                if axis == 1 {
                    Ok(*self)
                } else {
                    // Axis tried to descend through atom
                    Err(Error::NotCell)
                }
            }
        }
    }

    #[inline(always)]
    fn raw_slot_indirect(&self, axis: &[u64]) -> Result<Noun> {
        match self.as_either_atom_cell() {
            Right(cell) => cell.raw_slot_indirect(axis),
            Left(_atom) => {
                // Check if axis is 1 (all words are 0 except word[0] & 1 == 1)
                if axis.len() == 1 && axis[0] == 1 {
                    Ok(*self)
                } else if axis.is_empty() || (axis.len() == 1 && axis[0] == 0) {
                    Err(Error::NotRepresentable)
                } else {
                    // Axis tried to descend through atom
                    Err(Error::NotCell)
                }
            }
        }
    }
}

/**
 * An allocation object (probably a mem::NockStack) which can allocate a memory buffer sized to
 * a certain number of nouns
 */
pub trait NounAllocator: Sized + Stack {
    /** Allocate memory for some multiple of the size of a noun
     *
     * This should allocate *two more* `u64`s than `words` to make space for the size and metadata
     */
    unsafe fn alloc_indirect(&mut self, words: usize) -> *mut u64;

    /** Allocate memory for a cell */
    unsafe fn alloc_cell(&mut self) -> *mut CellMemory;

    /** Allocate space for a struct in a stack frame */
    unsafe fn alloc_struct<T>(&mut self, count: usize) -> *mut T;

    /** Check if two allocated nouns are equal **/
    unsafe fn equals(&mut self, a: *mut Noun, b: *mut Noun) -> bool;
}

/**
 * Implementing types allow component Nouns to be retreived by numeric axis
 */
pub trait Slots: private::RawSlots {
    /**
     * Retrieve component Noun at given axis, or fail with descriptive error
     */
    fn slot(&self, axis: u64) -> Result<Noun> {
        self.raw_slot_direct(axis)
    }

    /**
     * Retrieve component Noun at axis given as Atom, or fail with descriptive error
     *
     * SAFETY: This method assumes the atom is in stack-pointer form (LOCATION_BIT=0)
     */
    fn slot_atom(&self, atom: Atom) -> Result<Noun> {
        match atom.as_either() {
            Left(direct) => self.raw_slot_direct(direct.data()),
            // SAFETY: Operating on stack-allocated indirect atoms
            Right(indirect) => self.raw_slot_indirect(unsafe { indirect.as_slice_stack() }),
        }
    }

    /**
     * Retrieve component Noun at axis given as Atom, or fail with descriptive error.
     *
     * Auto-dispatches based on LOCATION_BIT - works with both stack-pointer
     * and offset-form atoms.
     */
    fn slot_atom_auto(&self, atom: Atom) -> Result<Noun> {
        match atom.as_either() {
            Left(direct) => self.raw_slot_direct(direct.data()),
            Right(indirect) => {
                // Use auto-dispatch to handle both stack and offset forms
                crate::mem::Arena::with_current(|arena| {
                    self.raw_slot_indirect(indirect.as_slice_with_arena(arena))
                })
            }
        }
    }
}

/**
 * Implementation methods that should not be made available to derived crates
 */
mod private {
    use crate::noun::{Noun, Result};

    /**
     * Implementation of the Slots trait
     */
    pub trait RawSlots {
        /**
         * Actual logic of retreiving Noun object at some axis (direct)
         */
        fn raw_slot_direct(&self, axis: u64) -> Result<Noun>;

        /**
         * Actual logic of retreiving Noun object at some axis (indirect)
         */
        fn raw_slot_indirect(&self, axis: &[u64]) -> Result<Noun>;
    }
}

// =============================================================================
// NounRef API - Lifetime-bound noun access with explicit memory context
// =============================================================================

/// Lifetime-bound noun reference. Prevents use-after-free by tying
/// access to the lifetime of the underlying memory context.
///
/// # Overview
///
/// `NounRef` provides safe access to nouns while ensuring the backing memory
/// (NockStack or PMA) remains valid. All noun traversal operations go through
/// this type.
///
/// # Usage
///
/// ```ignore
/// let mem = MemContext::new(&stack, &pma_arena);
/// let noun_ref = NounRef::bind(noun, &mem);
///
/// if let Ok(cell) = noun_ref.as_cell() {
///     let head = cell.head();
///     let tail = cell.tail();
/// }
///
/// // Store the raw noun when done
/// let stored: Noun = noun_ref.unbind();
/// ```
#[derive(Copy, Clone)]
pub struct NounRef<'a> {
    raw: u64,
    mem: &'a MemContext<'a>,
}

impl<'a> NounRef<'a> {
    /// Bind a raw Noun to a memory context, creating a lifetime-bound reference.
    #[inline(always)]
    pub fn bind(noun: Noun, mem: &'a MemContext<'a>) -> Self {
        NounRef {
            raw: unsafe { noun.raw },
            mem,
        }
    }

    /// Unbind this reference, returning the raw Noun for storage.
    /// The returned Noun has no lifetime protection.
    #[inline(always)]
    pub fn unbind(self) -> Noun {
        Noun { raw: self.raw }
    }

    /// Get the raw u64 value (for advanced use cases).
    #[inline(always)]
    pub fn raw(&self) -> u64 {
        self.raw
    }

    /// Get a reference to the memory context.
    #[inline(always)]
    pub fn mem(&self) -> &'a MemContext<'a> {
        self.mem
    }

    // -------------------------------------------------------------------------
    // Type queries (no memory access required)
    // -------------------------------------------------------------------------

    /// Check if this is a direct atom (value fits in 63 bits).
    #[inline(always)]
    pub fn is_direct(&self) -> bool {
        is_direct_atom(self.raw)
    }

    /// Check if this is an indirect atom (heap-allocated).
    #[inline(always)]
    pub fn is_indirect(&self) -> bool {
        is_indirect_atom(self.raw)
    }

    /// Check if this is a cell.
    #[inline(always)]
    pub fn is_cell(&self) -> bool {
        is_cell(self.raw)
    }

    /// Check if this is any kind of atom (direct or indirect).
    #[inline(always)]
    pub fn is_atom(&self) -> bool {
        self.is_direct() || self.is_indirect()
    }

    /// Check if this is an allocated noun (indirect atom or cell).
    #[inline(always)]
    pub fn is_allocated(&self) -> bool {
        !self.is_direct()
    }

    /// Check if this noun is stored in PMA (persistent memory).
    #[inline(always)]
    pub fn is_pma(&self) -> bool {
        MemContext::is_pma(self.raw)
    }

    /// Check if this noun is stored on the stack (ephemeral memory).
    #[inline(always)]
    pub unsafe fn is_stack(&self) -> bool {
        MemContext::is_stack(self.raw)
    }

    // -------------------------------------------------------------------------
    // Type conversions
    // -------------------------------------------------------------------------

    /// Try to convert this to a CellRef.
    #[inline(always)]
    pub fn as_cell(&self) -> Result<CellRef<'a>> {
        if self.is_cell() {
            Ok(CellRef {
                raw: self.raw,
                mem: self.mem,
            })
        } else {
            Err(Error::NotCell)
        }
    }

    /// Try to convert this to an AtomRef.
    #[inline(always)]
    pub fn as_atom(&self) -> Result<AtomRef<'a>> {
        if self.is_atom() {
            Ok(AtomRef {
                raw: self.raw,
                mem: self.mem,
            })
        } else {
            Err(Error::NotAtom)
        }
    }

    /// Try to get this as a direct atom value.
    #[inline(always)]
    pub fn as_direct(&self) -> Result<u64> {
        if self.is_direct() {
            Ok(self.raw & DIRECT_MAX)
        } else {
            Err(Error::NotDirectAtom)
        }
    }

    /// Convert to Either<AtomRef, CellRef>.
    #[inline(always)]
    pub fn as_either(&self) -> Either<AtomRef<'a>, CellRef<'a>> {
        if self.is_cell() {
            Right(CellRef {
                raw: self.raw,
                mem: self.mem,
            })
        } else {
            Left(AtomRef {
                raw: self.raw,
                mem: self.mem,
            })
        }
    }

    // -------------------------------------------------------------------------
    // Equality
    // -------------------------------------------------------------------------

    /// Raw bit equality (for hash maps, etc.)
    #[inline(always)]
    pub fn raw_equals(&self, other: &NounRef<'_>) -> bool {
        self.raw == other.raw
    }
}

impl<'a> fmt::Debug for NounRef<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "NounRef({:#x})", self.raw)
    }
}

/// Lifetime-bound cell reference. Provides safe access to cell head/tail.
#[derive(Copy, Clone)]
pub struct CellRef<'a> {
    raw: u64,
    mem: &'a MemContext<'a>,
}

impl<'a> CellRef<'a> {
    /// Get the head of this cell.
    #[inline(always)]
    pub fn head(&self) -> NounRef<'a> {
        let ptr = self.mem.resolve(self.raw, CELL_MASK) as *const CellMemory;
        let head_raw = unsafe { (*ptr).head.raw };
        NounRef {
            raw: head_raw,
            mem: self.mem,
        }
    }

    /// Get the tail of this cell.
    #[inline(always)]
    pub fn tail(&self) -> NounRef<'a> {
        let ptr = self.mem.resolve(self.raw, CELL_MASK) as *const CellMemory;
        let tail_raw = unsafe { (*ptr).tail.raw };
        NounRef {
            raw: tail_raw,
            mem: self.mem,
        }
    }

    /// Get both head and tail at once.
    #[inline(always)]
    pub fn head_tail(&self) -> (NounRef<'a>, NounRef<'a>) {
        let ptr = self.mem.resolve(self.raw, CELL_MASK) as *const CellMemory;
        unsafe {
            let head_raw = (*ptr).head.raw;
            let tail_raw = (*ptr).tail.raw;
            (
                NounRef {
                    raw: head_raw,
                    mem: self.mem,
                },
                NounRef {
                    raw: tail_raw,
                    mem: self.mem,
                },
            )
        }
    }

    /// Unbind this reference, returning the raw Cell for storage.
    #[inline(always)]
    pub fn unbind(self) -> Cell {
        Cell(self.raw)
    }

    /// Convert to NounRef.
    #[inline(always)]
    pub fn as_noun(&self) -> NounRef<'a> {
        NounRef {
            raw: self.raw,
            mem: self.mem,
        }
    }

    /// Get the raw u64 value.
    #[inline(always)]
    pub fn raw(&self) -> u64 {
        self.raw
    }

    /// Check if this cell is stored in PMA (persistent memory).
    #[inline(always)]
    pub fn is_pma(&self) -> bool {
        MemContext::is_pma(self.raw)
    }

    /// Check if this cell is stored on the stack (ephemeral memory).
    #[inline(always)]
    pub unsafe fn is_stack(&self) -> bool {
        MemContext::is_stack(self.raw)
    }
}

impl<'a> fmt::Debug for CellRef<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "CellRef({:#x})", self.raw)
    }
}

/// Lifetime-bound atom reference. Provides safe access to atom data.
#[derive(Copy, Clone)]
pub struct AtomRef<'a> {
    raw: u64,
    mem: &'a MemContext<'a>,
}

impl<'a> AtomRef<'a> {
    /// Check if this is a direct atom.
    #[inline(always)]
    pub fn is_direct(&self) -> bool {
        is_direct_atom(self.raw)
    }

    /// Check if this is an indirect atom.
    #[inline(always)]
    pub fn is_indirect(&self) -> bool {
        is_indirect_atom(self.raw)
    }

    /// Get the value if this is a direct atom.
    #[inline(always)]
    pub fn as_direct(&self) -> Result<u64> {
        if self.is_direct() {
            Ok(self.raw & DIRECT_MAX)
        } else {
            Err(Error::NotDirectAtom)
        }
    }

    /// Get the size in 64-bit words (1 for direct atoms).
    #[inline(always)]
    pub fn size(&self) -> usize {
        if self.is_direct() {
            1
        } else {
            let ptr = self.mem.resolve(self.raw, INDIRECT_MASK) as *const u64;
            unsafe { *ptr as usize }
        }
    }

    /// Get byte slice for indirect atoms, or convert direct to bytes.
    ///
    /// For indirect atoms, returns a slice into the allocated memory.
    /// For direct atoms, this method is not available (use as_direct() instead).
    pub fn as_bytes(&self) -> Result<&'a [u8]> {
        if self.is_direct() {
            Err(Error::NotIndirectAtom)
        } else {
            let ptr = self.mem.resolve(self.raw, INDIRECT_MASK) as *const u64;
            unsafe {
                let size = *ptr as usize;
                let data_ptr = ptr.add(2) as *const u8;
                Ok(from_raw_parts(data_ptr, size * 8))
            }
        }
    }

    /// Get the data slice as u64 words for indirect atoms.
    pub fn as_slice(&self) -> Result<&'a [u64]> {
        if self.is_direct() {
            Err(Error::NotIndirectAtom)
        } else {
            let ptr = self.mem.resolve(self.raw, INDIRECT_MASK) as *const u64;
            unsafe {
                let size = *ptr as usize;
                let data_ptr = ptr.add(2);
                Ok(from_raw_parts(data_ptr, size))
            }
        }
    }

    /// Unbind this reference, returning the raw Atom for storage.
    #[inline(always)]
    pub fn unbind(self) -> Atom {
        Atom { raw: self.raw }
    }

    /// Convert to NounRef.
    #[inline(always)]
    pub fn as_noun(&self) -> NounRef<'a> {
        NounRef {
            raw: self.raw,
            mem: self.mem,
        }
    }

    /// Get the raw u64 value.
    #[inline(always)]
    pub fn raw(&self) -> u64 {
        self.raw
    }

    /// Check if this atom is stored in PMA (persistent memory).
    #[inline(always)]
    pub fn is_pma(&self) -> bool {
        self.is_indirect() && MemContext::is_pma(self.raw)
    }

    /// Check if this atom is stored on the stack (ephemeral memory).
    #[inline(always)]
    pub unsafe fn is_stack(&self) -> bool {
        self.is_direct() || MemContext::is_stack(self.raw)
    }
}

impl<'a> fmt::Debug for AtomRef<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.is_direct() {
            write!(f, "AtomRef::Direct({:#x})", self.raw & DIRECT_MAX)
        } else {
            write!(f, "AtomRef::Indirect({:#x})", self.raw)
        }
    }
}

// =============================================================================
// Memory Resolver Trait and Implementations
// =============================================================================

/// Trait for resolving noun pointers to memory locations.
///
/// This enables a single `NounDecode` implementation to work with both
/// stack-allocated and PMA-allocated nouns without code duplication.
///
/// # Implementations
/// - [`StackResolver`]: Zero-size type for stack-only access (fastest)
/// - [`PmaResolver`]: Carries arena reference for PMA access
/// - [`UnifiedResolver`]: Handles both via LOCATION_BIT check
pub trait MemoryResolver {
    /// Resolve a cell's head noun.
    fn resolve_head(&self, cell: Cell) -> Noun;

    /// Resolve a cell's tail noun.
    fn resolve_tail(&self, cell: Cell) -> Noun;

    /// Resolve an indirect atom's data slice.
    ///
    /// The slice borrows from the underlying memory which the IndirectAtom points to.
    fn resolve_slice<'a>(&self, indirect: &'a IndirectAtom) -> &'a [u64];

    /// Resolve an indirect atom's size in words.
    fn resolve_size(&self, indirect: &IndirectAtom) -> usize;
}

/// Zero-size resolver for stack-only noun access.
///
/// This resolver has **zero runtime overhead** because:
/// - It's a zero-sized type (ZST)
/// - All methods inline to the same code as raw `_stack` methods
///
/// # Panics
/// Methods will panic if called on PMA-form nouns (LOCATION_BIT=1).
///
/// # Example
/// ```ignore
/// let (x, y): (u64, String) = noun.decode_stack()?;  // Uses StackResolver
/// ```
#[derive(Copy, Clone, Debug, Default)]
pub struct StackResolver;

impl MemoryResolver for StackResolver {
    #[inline(always)]
    fn resolve_head(&self, cell: Cell) -> Noun {
        unsafe { cell.head_stack() }
    }

    #[inline(always)]
    fn resolve_tail(&self, cell: Cell) -> Noun {
        unsafe { cell.tail_stack() }
    }

    #[inline(always)]
    fn resolve_slice<'a>(&self, indirect: &'a IndirectAtom) -> &'a [u64] {
        // The slice's lifetime is tied to the IndirectAtom which holds the pointer
        unsafe { indirect.as_slice_stack() }
    }

    #[inline(always)]
    fn resolve_size(&self, indirect: &IndirectAtom) -> usize {
        unsafe { indirect.size_stack() }
    }
}

/// Resolver for PMA (Persistent Memory Arena) noun access.
///
/// Carries a reference to the Arena for pointer resolution.
#[derive(Copy, Clone)]
pub struct PmaResolver<'a> {
    arena: &'a Arena,
}

impl<'a> PmaResolver<'a> {
    /// Create a new PMA resolver.
    #[inline]
    pub fn new(arena: &'a Arena) -> Self {
        Self { arena }
    }

    /// Get the underlying arena reference.
    #[inline]
    pub fn arena(&self) -> &'a Arena {
        self.arena
    }
}

impl MemoryResolver for PmaResolver<'_> {
    #[inline(always)]
    fn resolve_head(&self, cell: Cell) -> Noun {
        cell.head_with_arena(self.arena)
    }

    #[inline(always)]
    fn resolve_tail(&self, cell: Cell) -> Noun {
        cell.tail_with_arena(self.arena)
    }

    #[inline(always)]
    fn resolve_slice<'a>(&self, indirect: &'a IndirectAtom) -> &'a [u64] {
        indirect.as_slice_with_arena(self.arena)
    }

    #[inline(always)]
    fn resolve_size(&self, indirect: &IndirectAtom) -> usize {
        indirect.size_with_arena(self.arena)
    }
}

/// Unified resolver that handles both stack and PMA nouns.
///
/// Uses the LOCATION_BIT to determine which resolution path to take.
/// Slightly slower than specialized resolvers but handles mixed graphs.
#[derive(Copy, Clone)]
pub struct UnifiedResolver<'a> {
    arena: &'a Arena,
}

impl<'a> UnifiedResolver<'a> {
    /// Create a new unified resolver.
    #[inline]
    pub fn new(arena: &'a Arena) -> Self {
        Self { arena }
    }

    /// Get the underlying arena reference.
    #[inline]
    pub fn arena(&self) -> &'a Arena {
        self.arena
    }
}

impl MemoryResolver for UnifiedResolver<'_> {
    #[inline(always)]
    fn resolve_head(&self, cell: Cell) -> Noun {
        // _with_arena handles both stack and PMA based on LOCATION_BIT
        cell.head_with_arena(self.arena)
    }

    #[inline(always)]
    fn resolve_tail(&self, cell: Cell) -> Noun {
        cell.tail_with_arena(self.arena)
    }

    #[inline(always)]
    fn resolve_slice<'a>(&self, indirect: &'a IndirectAtom) -> &'a [u64] {
        indirect.as_slice_with_arena(self.arena)
    }

    #[inline(always)]
    fn resolve_size(&self, indirect: &IndirectAtom) -> usize {
        indirect.size_with_arena(self.arena)
    }
}

// =============================================================================
// Lifetime-Bound Stack Types
// =============================================================================

/// A Cell proven to reside on the NockStack, with lifetime bound.
///
/// This type provides safe access to cell data without runtime checks on every
/// access. The safety invariants are verified once at construction time.
///
/// # Safety Guarantees
/// - LOCATION_BIT = 0 (validated at construction)
/// - Stack outlives this reference (enforced by Rust borrow checker)
///
/// # Zero-Cost Abstraction
/// This type is `#[repr(transparent)]` over `Cell`, meaning it has the same
/// memory layout and all methods inline to identical code as raw `_stack` methods.
///
/// # Example
/// ```ignore
/// let sc = cell.with_stack(stack).expect("cell not on stack");
/// let head = sc.head();  // Safe! No runtime check needed
/// let tail = sc.tail();  // Safe!
/// ```
#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct StackCell<'a> {
    inner: Cell,
    _lifetime: std::marker::PhantomData<&'a NockStack>,
}

impl<'a> StackCell<'a> {
    /// Create from a Cell with runtime validation.
    ///
    /// Returns `None` if the cell is in PMA form (LOCATION_BIT=1).
    #[inline]
    pub fn new(cell: Cell, _stack: &'a NockStack) -> Option<Self> {
        // Check LOCATION_BIT via the cell's raw value
        if MemContext::is_stack(cell.0) {
            Some(Self {
                inner: cell,
                _lifetime: std::marker::PhantomData,
            })
        } else {
            None
        }
    }

    /// Create from a Cell without runtime validation.
    ///
    /// # Safety
    /// Caller must ensure the cell is in stack-pointer form (LOCATION_BIT=0).
    #[inline]
    pub unsafe fn new_unchecked(cell: Cell, _stack: &'a NockStack) -> Self {
        debug_assert!(
            MemContext::is_stack(cell.0),
            "StackCell::new_unchecked called on non-stack cell"
        );
        Self {
            inner: cell,
            _lifetime: std::marker::PhantomData,
        }
    }

    /// Get the head of this cell.
    ///
    /// This is safe because the type proves stack residency and lifetime.
    #[inline(always)]
    pub fn head(&self) -> Noun {
        // SAFETY: Validated at construction that cell is on stack
        unsafe { self.inner.head_stack() }
    }

    /// Get the tail of this cell.
    #[inline(always)]
    pub fn tail(&self) -> Noun {
        unsafe { self.inner.tail_stack() }
    }

    /// Get both head and tail.
    #[inline(always)]
    pub fn head_tail(&self) -> (Noun, Noun) {
        (self.head(), self.tail())
    }

    /// Get a mutable pointer to the head slot.
    ///
    /// Used for unification and other in-place modifications.
    #[inline(always)]
    pub fn head_ptr(&self) -> *mut Noun {
        unsafe { self.inner.head_as_mut_stack() }
    }

    /// Get a mutable pointer to the tail slot.
    #[inline(always)]
    pub fn tail_ptr(&self) -> *mut Noun {
        unsafe { self.inner.tail_as_mut_stack() }
    }

    /// Get the raw pointer to the cell's memory.
    #[inline(always)]
    pub fn raw_pointer(&self) -> *const CellMemory {
        unsafe { self.inner.to_raw_pointer_stack() }
    }

    /// Get the raw mutable pointer to the cell's memory.
    #[inline(always)]
    pub fn raw_pointer_mut(&mut self) -> *mut CellMemory {
        unsafe { self.inner.to_raw_pointer_mut_stack() }
    }

    /// Unwrap to the inner Cell.
    #[inline(always)]
    pub fn into_inner(self) -> Cell {
        self.inner
    }

    /// Get a reference to the inner Cell.
    #[inline(always)]
    pub fn as_cell(&self) -> Cell {
        self.inner
    }

    /// Try to get the head as a StackCell.
    ///
    /// Returns `Some` if the head is also a cell on the stack.
    #[inline]
    pub fn head_as_stack_cell(&self, stack: &'a NockStack) -> Option<StackCell<'a>> {
        self.head().as_cell().ok().and_then(|c| StackCell::new(c, stack))
    }

    /// Try to get the tail as a StackCell.
    #[inline]
    pub fn tail_as_stack_cell(&self, stack: &'a NockStack) -> Option<StackCell<'a>> {
        self.tail().as_cell().ok().and_then(|c| StackCell::new(c, stack))
    }
}

impl fmt::Debug for StackCell<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "StackCell({:?})", self.inner)
    }
}

/// An IndirectAtom proven to reside on the NockStack, with lifetime bound.
///
/// Similar to [`StackCell`], this provides safe access to atom data after
/// validating stack residency once at construction.
///
/// # Zero-Cost Abstraction
/// This type is `#[repr(transparent)]` over `IndirectAtom`.
#[derive(Copy, Clone)]
#[repr(transparent)]
pub struct StackAtom<'a> {
    inner: IndirectAtom,
    _lifetime: std::marker::PhantomData<&'a NockStack>,
}

impl<'a> StackAtom<'a> {
    /// Create from an IndirectAtom with runtime validation.
    ///
    /// Returns `None` if the atom is in PMA form.
    #[inline]
    pub fn new(atom: IndirectAtom, _stack: &'a NockStack) -> Option<Self> {
        // Check LOCATION_BIT via the atom's raw value
        if MemContext::is_stack(atom.0) {
            Some(Self {
                inner: atom,
                _lifetime: std::marker::PhantomData,
            })
        } else {
            None
        }
    }

    /// Create from an IndirectAtom without runtime validation.
    ///
    /// # Safety
    /// Caller must ensure the atom is in stack-pointer form (LOCATION_BIT=0).
    #[inline]
    pub unsafe fn new_unchecked(atom: IndirectAtom, _stack: &'a NockStack) -> Self {
        debug_assert!(
            MemContext::is_stack(atom.0),
            "StackAtom::new_unchecked called on non-stack atom"
        );
        Self {
            inner: atom,
            _lifetime: std::marker::PhantomData,
        }
    }

    /// Get the size in 64-bit words.
    #[inline(always)]
    pub fn size(&self) -> usize {
        unsafe { self.inner.size_stack() }
    }

    /// Get the data as a slice of u64 words.
    #[inline(always)]
    pub fn as_slice(&self) -> &[u64] {
        unsafe { self.inner.as_slice_stack() }
    }

    /// Get the data as a BitSlice.
    #[inline(always)]
    pub fn as_bitslice(&self) -> &BitSlice<u64, Lsb0> {
        unsafe { self.inner.as_bitslice_stack() }
    }

    /// Get the data as a byte slice in native endian order.
    #[inline(always)]
    pub fn as_ne_bytes(&self) -> &[u8] {
        unsafe { self.inner.as_ne_bytes_stack() }
    }

    /// Try to convert to a u64 if the atom fits.
    #[inline(always)]
    pub fn as_u64(&self) -> Result<u64> {
        unsafe { self.inner.as_u64_stack() }
    }

    /// Get the raw data pointer.
    #[inline(always)]
    pub fn data_pointer(&self) -> *const u64 {
        unsafe { self.inner.data_pointer_stack() }
    }

    /// Get the raw pointer to the atom's metadata.
    #[inline(always)]
    pub fn raw_pointer(&self) -> *const u64 {
        unsafe { self.inner.to_raw_pointer_stack() }
    }

    /// Unwrap to the inner IndirectAtom.
    #[inline(always)]
    pub fn into_inner(self) -> IndirectAtom {
        self.inner
    }

    /// Normalize this atom (remove leading zeros).
    #[inline]
    pub fn normalize(&mut self) -> &IndirectAtom {
        unsafe { self.inner.normalize_stack() }
    }

    /// Normalize and convert to Atom (may become DirectAtom if small enough).
    #[inline]
    pub fn normalize_as_atom(&mut self) -> Atom {
        unsafe { self.inner.normalize_as_atom_stack() }
    }
}

impl fmt::Debug for StackAtom<'_> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "StackAtom({:?})", self.inner)
    }
}

// =============================================================================
// WithStack Extension Trait
// =============================================================================

/// Fluent API for binding nouns to a NockStack lifetime.
///
/// This trait provides `.with_stack()` and `.with_stack_unchecked()` methods
/// for converting raw noun types to their lifetime-bound counterparts.
///
/// # Example
/// ```ignore
/// use nockvm::noun::WithStack;
///
/// // Checked conversion (returns Option)
/// if let Some(sc) = cell.with_stack(stack) {
///     let head = sc.head();  // Safe!
/// }
///
/// // Unchecked conversion for proven-safe hot paths
/// let sc = unsafe { cell.with_stack_unchecked(stack) };
/// ```
pub trait WithStack: Sized {
    /// The lifetime-bound type produced by this trait.
    type Bound<'a>;

    /// Bind to a NockStack with runtime validation.
    ///
    /// Returns `None` if the noun is not on the stack.
    fn with_stack<'a>(self, stack: &'a NockStack) -> Option<Self::Bound<'a>>;

    /// Bind to a NockStack without runtime validation.
    ///
    /// # Safety
    /// Caller must ensure the noun is in stack-pointer form.
    unsafe fn with_stack_unchecked<'a>(self, stack: &'a NockStack) -> Self::Bound<'a>;
}

impl WithStack for Cell {
    type Bound<'a> = StackCell<'a>;

    #[inline]
    fn with_stack<'a>(self, stack: &'a NockStack) -> Option<StackCell<'a>> {
        StackCell::new(self, stack)
    }

    #[inline]
    unsafe fn with_stack_unchecked<'a>(self, stack: &'a NockStack) -> StackCell<'a> {
        StackCell::new_unchecked(self, stack)
    }
}

impl WithStack for IndirectAtom {
    type Bound<'a> = StackAtom<'a>;

    #[inline]
    fn with_stack<'a>(self, stack: &'a NockStack) -> Option<StackAtom<'a>> {
        StackAtom::new(self, stack)
    }

    #[inline]
    unsafe fn with_stack_unchecked<'a>(self, stack: &'a NockStack) -> StackAtom<'a> {
        StackAtom::new_unchecked(self, stack)
    }
}

#[cfg(test)]
mod tests {
    use crate::jets::util::test::init_context;
    use crate::noun::{Cell, Slots, D};

    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_slot_direct_simple() {
        let mut context = init_context();
        let cell = Cell::new(&mut context.stack, D(1), D(2));

        // axis 1 returns the whole cell
        assert_eq!(
            unsafe { cell.slot(1).unwrap().raw_equals(&cell.as_noun()) },
            true
        );

        // axis 2 returns head
        assert_eq!(unsafe { cell.slot(2).unwrap().raw_equals(&D(1)) }, true);

        // axis 3 returns tail
        assert_eq!(unsafe { cell.slot(3).unwrap().raw_equals(&D(2)) }, true);
    }

    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_slot_direct_nested() {
        let mut context = init_context();
        let inner = Cell::new(&mut context.stack, D(3), D(4));
        // cell = [1 [3 4]]
        let cell = Cell::new(&mut context.stack, D(1), inner.as_noun());

        // axis 6 = 110 binary = tail then head = head of tail = 3
        assert_eq!(unsafe { cell.slot(6).unwrap().raw_equals(&D(3)) }, true);

        // axis 7 = 111 binary = tail then tail = tail of tail = 4
        assert_eq!(unsafe { cell.slot(7).unwrap().raw_equals(&D(4)) }, true);

        // axis 4 = 100 binary = head then stop = should fail (head is atom)
        assert!(cell.slot(4).is_err());

        // cell2 = [[3 4] 2]
        let cell2 = Cell::new(&mut context.stack, inner.as_noun(), D(2));
        // axis 5 = 101 binary = head then tail = tail of head = 4
        assert_eq!(unsafe { cell2.slot(5).unwrap().raw_equals(&D(4)) }, true);
    }

    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_slot_zero_axis() {
        let mut context = init_context();
        let cell = Cell::new(&mut context.stack, D(1), D(2));

        // axis 0 should fail
        assert!(cell.slot(0).is_err());
    }
}

#[cfg(test)]
mod test {
    use ibig::ubig;

    use crate::jets::util::test::init_context;
    use crate::noun::Atom;

    #[test]
    //  APOLOGIA: ibig/ubig ManuallyDrops Vec, we are aware, we plan on purging it
    #[cfg_attr(miri, ignore)]
    fn test_to_ne_bytes_direct() {
        let mut context = init_context();
        let big = ubig!(0x1234567890abcdefa0);
        let atom = Atom::from_ubig(&mut context.stack, &big);
        let bytes = atom.to_ne_bytes();
        #[cfg(target_endian = "little")]
        {
            assert_eq!(
                bytes,
                vec![
                    0xa0, 0xef, 0xcd, 0xab, 0x90, 0x78, 0x56, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00,
                    0x00, 0x00, 0x00
                ]
            );
        }
        #[cfg(target_endian = "big")]
        {
            assert_eq!(
                bytes,
                vec![
                    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0x90, 0xab,
                    0xcd, 0xef, 0xa0
                ]
            );
        }
    }

    #[test]
    //  APOLOGIA: ibig/ubig ManuallyDrops Vec, we are aware, we plan on purging it
    #[cfg_attr(miri, ignore)]
    fn test_to_ne_bytes_indirect() {
        let mut context = init_context();
        let atom = Atom::new(&mut context.stack, 0x1234);
        let bytes = atom.to_ne_bytes();
        #[cfg(target_endian = "little")]
        {
            assert_eq!(bytes, vec![0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
        }
        #[cfg(target_endian = "big")]
        {
            assert_eq!(bytes, vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34]);
        }
    }

    #[test]
    //  APOLOGIA: ibig/ubig ManuallyDrops Vec, we are aware, we plan on purging it
    #[cfg_attr(miri, ignore)]
    fn test_to_x_bytes_direct() {
        let mut context = init_context();
        let atom = Atom::new(&mut context.stack, 0x1234);
        let bytes_le = atom.to_le_bytes();
        assert_eq!(
            bytes_le,
            vec![0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]
        );

        let bytes_be = atom.to_be_bytes();
        assert_eq!(
            bytes_be,
            vec![0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34]
        );
    }

    #[test]
    //  APOLOGIA: ibig/ubig ManuallyDrops Vec, we are aware, we plan on purging it
    #[cfg_attr(miri, ignore)]
    fn test_to_le_bytes_indirect() {
        let mut context = init_context();
        let big = ubig!(0x1234567890abcd);
        let atom = Atom::from_ubig(&mut context.stack, &big);
        let bytes = atom.to_le_bytes();
        assert_eq!(bytes, vec![0xcd, 0xab, 0x90, 0x78, 0x56, 0x34, 0x12, 0x00]);
        //
        let big = ubig!(0x1234567890abcdefa0);
        let atom = Atom::from_ubig(&mut context.stack, &big);
        let bytes = atom.to_le_bytes();
        assert_eq!(
            bytes,
            vec![
                0xa0, 0xef, 0xcd, 0xab, 0x90, 0x78, 0x56, 0x34, 0x12, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00
            ],
        );
    }

    #[test]
    //  APOLOGIA: ibig/ubig ManuallyDrops Vec, we are aware, we plan on purging it
    #[cfg_attr(miri, ignore)]
    fn test_to_be_bytes_indirect() {
        let mut context = init_context();
        let big = ubig!(0x34567890abcdef);
        let atom = Atom::from_ubig(&mut context.stack, &big);
        let bytes = atom.to_be_bytes();
        assert_eq!(bytes, vec![0x00, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd, 0xef]);
        //
        let big = ubig!(0x1234567890abcdefa0);
        let atom = Atom::from_ubig(&mut context.stack, &big);
        let bytes = atom.to_be_bytes();
        assert_eq!(
            bytes,
            vec![
                0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78, 0x90, 0xab, 0xcd,
                0xef, 0xa0
            ]
        );
    }
}

#[cfg(test)]
mod noun_ref_tests {
    use crate::jets::util::test::init_context;
    use crate::mem::MemContext;
    use crate::noun::{AtomRef, Cell, CellRef, NounRef, D};

    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_noun_ref_bind_unbind() {
        let mut context = init_context();
        let noun = D(42);
        let mem = MemContext::new(&context.stack, context.stack.arena());
        let noun_ref = NounRef::bind(noun, &mem);

        assert!(noun_ref.is_direct());
        assert!(!noun_ref.is_cell());
        assert!(noun_ref.is_atom());

        let unbound = noun_ref.unbind();
        assert_eq!(unsafe { unbound.raw }, unsafe { noun.raw });
    }

    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_noun_ref_cell_traversal() {
        let mut context = init_context();
        let cell = Cell::new(&mut context.stack, D(1), D(2));
        let noun = cell.as_noun();

        let mem = MemContext::new(&context.stack, context.stack.arena());
        let noun_ref = NounRef::bind(noun, &mem);

        assert!(noun_ref.is_cell());
        let cell_ref = noun_ref.as_cell().expect("should be cell");

        let head = cell_ref.head();
        let tail = cell_ref.tail();

        assert!(head.is_direct());
        assert!(tail.is_direct());
        assert_eq!(head.as_direct().unwrap(), 1);
        assert_eq!(tail.as_direct().unwrap(), 2);
    }

    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_noun_ref_nested_cells() {
        let mut context = init_context();
        let inner = Cell::new(&mut context.stack, D(3), D(4));
        let outer = Cell::new(&mut context.stack, D(1), inner.as_noun());

        let mem = MemContext::new(&context.stack, context.stack.arena());
        let noun_ref = NounRef::bind(outer.as_noun(), &mem);

        // [1 [3 4]]
        let cell_ref = noun_ref.as_cell().expect("should be cell");
        assert_eq!(cell_ref.head().as_direct().unwrap(), 1);

        let inner_ref = cell_ref.tail().as_cell().expect("tail should be cell");
        assert_eq!(inner_ref.head().as_direct().unwrap(), 3);
        assert_eq!(inner_ref.tail().as_direct().unwrap(), 4);
    }

    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_noun_ref_head_tail() {
        let mut context = init_context();
        let cell = Cell::new(&mut context.stack, D(10), D(20));

        let mem = MemContext::new(&context.stack, context.stack.arena());
        let cell_ref = CellRef {
            raw: cell.0,
            mem: &mem,
        };

        let (head, tail) = cell_ref.head_tail();
        assert_eq!(head.as_direct().unwrap(), 10);
        assert_eq!(tail.as_direct().unwrap(), 20);
    }

    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_atom_ref_direct() {
        let mut context = init_context();
        let noun = D(12345);

        let mem = MemContext::new(&context.stack, context.stack.arena());
        let noun_ref = NounRef::bind(noun, &mem);

        let atom_ref = noun_ref.as_atom().expect("should be atom");
        assert!(atom_ref.is_direct());
        assert!(!atom_ref.is_indirect());
        assert_eq!(atom_ref.as_direct().unwrap(), 12345);
        assert_eq!(atom_ref.size(), 1);
    }

    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_noun_ref_as_either() {
        let mut context = init_context();

        // Create cell first before borrowing stack for MemContext
        let cell = Cell::new(&mut context.stack, D(1), D(2));

        let mem = MemContext::new(&context.stack, context.stack.arena());

        // Test atom
        let atom = D(5);
        let atom_ref = NounRef::bind(atom, &mem);
        assert!(atom_ref.as_either().is_left());

        // Test cell
        let cell_noun_ref = NounRef::bind(cell.as_noun(), &mem);
        assert!(cell_noun_ref.as_either().is_right());
    }

    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_noun_ref_is_stack() {
        let mut context = init_context();
        let cell = Cell::new(&mut context.stack, D(1), D(2));

        let mem = MemContext::new(&context.stack, context.stack.arena());
        let noun_ref = NounRef::bind(cell.as_noun(), &mem);

        // Stack-allocated data should have LOCATION_BIT=0
        // SAFETY: noun_ref is bound to a valid MemContext
        unsafe {
            assert!(noun_ref.is_stack());
            assert!(!noun_ref.is_pma());
        }
    }
}
