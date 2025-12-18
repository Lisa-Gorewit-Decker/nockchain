//! Persistent Memory Arena (PMA)
//!
//! The PMA is a file-backed memory region for storing long-lived Nouns.
//! It uses bump allocation and stores nouns in offset form.

use std::path::PathBuf;
use std::sync::Arc;

use thiserror::Error;

use crate::mem::{word_size_of, Arena, NewStackError};
use crate::noun::{CellMemory, NounAllocator};

/// Errors that can occur during PMA operations
#[derive(Debug, Error)]
pub enum PmaError {
    #[error("PMA is full, cannot allocate {requested} words (available: {available})")]
    OutOfMemory { requested: usize, available: usize },

    #[error("PMA not installed in thread-local storage")]
    NotInstalled,

    #[error("Failed to create arena: {0}")]
    ArenaError(#[from] NewStackError),
}

/// The Persistent Memory Arena
///
/// A bump-allocated memory region for storing nouns in offset form.
/// The PMA is backed by a file (in future milestones) and persists across
/// program restarts.
///
/// Currently only suitable for a single reader/writer. In the future,
/// `alloc_offset` will be changed to `AtomicUsize` to allow multiple readers.
pub struct Pma {
    /// The underlying arena for memory management and pointer resolution
    arena: Arc<Arena>,
    /// Current allocation offset in words (bump pointer)
    alloc_offset: usize,
    /// Path to the backing file (for future file-backed persistence)
    path: PathBuf,
}

impl Pma {
    /// Create a new PMA with the given size in words
    pub fn new(size_words: usize, path: PathBuf) -> Result<Self, PmaError> {
        let arena = Arena::allocate(size_words)?;
        Ok(Self {
            arena,
            alloc_offset: 0,
            path,
        })
    }

    /// Get the underlying arena
    pub fn arena(&self) -> &Arc<Arena> {
        todo!()
    }

    /// Get the current allocation offset in words
    pub fn alloc_offset(&self) -> usize {
        self.alloc_offset
    }

    /// Get the total size of the PMA in words
    pub fn size_words(&self) -> usize {
        self.arena.words()
    }

    /// Get the number of free words remaining
    pub fn free_words(&self) -> usize {
        self.size_words().saturating_sub(self.alloc_offset())
    }

    /// Convert a pointer within the PMA to an offset in words
    pub fn offset_from_ptr(&self, ptr: *const u8) -> u32 {
        self.arena.offset_from_ptr(ptr)
    }

    /// Convert an offset in words to a pointer
    pub fn ptr_from_offset(&self, offset_words: u32) -> *mut u8 {
        self.arena.ptr_from_offset(offset_words)
    }

    /// Check if a pointer is within the PMA's memory region
    pub fn contains_ptr(&self, _ptr: *const u8) -> bool {
        todo!()
    }

    /// Reset the allocation pointer to zero
    pub fn reset(&mut self) {
        todo!()
    }

    /// Reset the allocation pointer to a specific offset
    pub fn reset_to(&mut self, _offset: usize) {
        todo!()
    }

    /// Allocate `words` from the PMA, returning a pointer to the allocation.
    /// This is the core bump allocation primitive.
    unsafe fn raw_alloc(&mut self, words: usize) -> *mut u64 {
        let ptr = self.arena.ptr_from_offset(self.alloc_offset as u32) as *mut u64;
        self.alloc_offset += words;
        ptr
    }
}

impl ibig::Stack for Pma {
    unsafe fn alloc_layout(&mut self, layout: std::alloc::Layout) -> *mut u64 {
        // Convert bytes to words, rounding up
        let words = (layout.size() + 7) >> 3;
        self.raw_alloc(words)
    }
}

impl NounAllocator for Pma {
    unsafe fn alloc_indirect(&mut self, words: usize) -> *mut u64 {
        self.raw_alloc(words + 2)
    }

    unsafe fn alloc_cell(&mut self) -> *mut CellMemory {
        self.raw_alloc(word_size_of::<CellMemory>()) as *mut CellMemory
    }

    unsafe fn alloc_struct<T>(&mut self, count: usize) -> *mut T {
        self.raw_alloc(word_size_of::<T>() * count) as *mut T
    }

    unsafe fn equals(&mut self, _a: *mut crate::noun::Noun, _b: *mut crate::noun::Noun) -> bool {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jets::cold::NounListMem;
    use crate::mem::word_size_of;
    use ibig::Stack;
    use std::alloc::Layout;

    /// Helper to create a test PMA with a given size
    fn test_pma(size_words: usize) -> Pma {
        Pma::new(size_words, PathBuf::from("/tmp/test_pma")).expect("Failed to create test PMA")
    }

    /// Verifies bump allocation returns sequential offsets and correctly tracks free space.
    ///
    /// This test exercises:
    /// - Pma::new creates a valid PMA
    /// - alloc_offset() starts at 0
    /// - free_words() equals size initially
    /// - NounAllocator::alloc_indirect bumps the offset correctly
    /// - NounAllocator::alloc_cell allocates CellMemory
    /// - NounAllocator::alloc_struct allocates arbitrary structs
    /// - Sequential allocations don't overlap
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_pma_allocation() {
        let mut pma = test_pma(1000);

        // Initial state: nothing allocated yet
        assert_eq!(pma.alloc_offset(), 0, "Initial alloc_offset should be 0");
        assert_eq!(pma.free_words(), 1000, "Initial free_words should equal size");

        // First allocation: alloc_indirect(10) allocates 10 + 2 = 12 words (data + metadata + size)
        let ptr1 = unsafe { pma.alloc_indirect(10) };
        assert!(!ptr1.is_null(), "First allocation should return non-null pointer");
        assert_eq!(pma.alloc_offset(), 12, "After alloc_indirect(10), offset should be 12");
        assert_eq!(pma.free_words(), 988, "After alloc_indirect(10), free should be 988");

        // Second allocation: alloc_indirect(20) allocates 20 + 2 = 22 words
        let ptr2 = unsafe { pma.alloc_indirect(20) };
        assert!(!ptr2.is_null(), "Second allocation should return non-null pointer");
        assert_eq!(pma.alloc_offset(), 34, "After second alloc, offset should be 34");
        assert_eq!(pma.free_words(), 966, "After second alloc, free should be 966");

        // Third allocation: alloc_cell allocates word_size_of::<CellMemory>() words
        let ptr3 = unsafe { pma.alloc_cell() };
        assert!(!ptr3.is_null(), "Cell allocation should return non-null pointer");
        let cell_words = word_size_of::<CellMemory>();
        let offset_after_cell = 34 + cell_words;
        assert_eq!(
            pma.alloc_offset(),
            offset_after_cell,
            "After cell alloc, offset should increase by CellMemory size"
        );

        // Fourth allocation: alloc_struct for NounListMem
        let struct_words = word_size_of::<NounListMem>();
        let ptr4: *mut NounListMem = unsafe { pma.alloc_struct(1) };
        assert!(!ptr4.is_null(), "Struct allocation should return non-null pointer");
        let offset_after_struct = offset_after_cell + struct_words;
        assert_eq!(
            pma.alloc_offset(),
            offset_after_struct,
            "After struct alloc, offset should increase by struct size in words"
        );

        // Fifth allocation: alloc_struct with count > 1 (allocate array of 3 NounListMem)
        let ptr5: *mut NounListMem = unsafe { pma.alloc_struct(3) };
        assert!(!ptr5.is_null(), "Array struct allocation should return non-null pointer");
        let offset_after_array = offset_after_struct + (struct_words * 3);
        assert_eq!(
            pma.alloc_offset(),
            offset_after_array,
            "After array alloc, offset should increase by struct_size * count"
        );

        // Sixth allocation: alloc_layout for ibig::Stack trait (allocate 8 u64s)
        let layout_words = 8usize;
        let layout = Layout::array::<u64>(layout_words).expect("valid layout");
        let ptr6 = unsafe { pma.alloc_layout(layout) };
        assert!(!ptr6.is_null(), "Layout allocation should return non-null pointer");
        assert_eq!(
            pma.alloc_offset(),
            offset_after_array + layout_words,
            "After layout alloc, offset should increase by layout size in words"
        );

        // Verify all allocations are sequential and non-overlapping
        // For a bump allocator, each pointer should be at or after the end of the previous allocation
        let ptr1_end = unsafe { ptr1.add(12) }; // 12 words for alloc_indirect(10)
        let ptr2_end = unsafe { ptr2.add(22) }; // 22 words for alloc_indirect(20)
        let ptr3_end = unsafe { (ptr3 as *mut u64).add(cell_words) };
        let ptr4_end = unsafe { (ptr4 as *mut u64).add(struct_words) };
        let ptr5_end = unsafe { (ptr5 as *mut u64).add(struct_words * 3) };

        assert!(
            ptr2 >= ptr1_end,
            "ptr2 should start at or after ptr1's end"
        );
        assert!(
            ptr3 as *mut u64 >= ptr2_end,
            "ptr3 should start at or after ptr2's end"
        );
        assert!(
            ptr4 as *mut u64 >= ptr3_end,
            "ptr4 should start at or after ptr3's end"
        );
        assert!(
            ptr5 as *mut u64 >= ptr4_end,
            "ptr5 should start at or after ptr4's end"
        );
        assert!(
            ptr6 >= ptr5_end,
            "ptr6 should start at or after ptr5's end"
        );
    }

    /// Verifies offset-to-pointer and pointer-to-offset conversions are inverses.
    ///
    /// This test exercises:
    /// - ptr_from_offset converts word offset to pointer
    /// - offset_from_ptr converts pointer back to word offset
    /// - Round-trip: offset -> ptr -> offset gives same offset
    /// - Round-trip: ptr -> offset -> ptr gives same ptr
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_pma_offset_round_trip() {
        let mut pma = test_pma(1000);

        // Test with offset 0 (base of PMA)
        let ptr_at_0 = pma.ptr_from_offset(0);
        let offset_from_0 = pma.offset_from_ptr(ptr_at_0);
        assert_eq!(offset_from_0, 0, "Offset at base should be 0");

        // Test with a known offset
        let test_offset: u32 = 42;
        let ptr = pma.ptr_from_offset(test_offset);
        let recovered_offset = pma.offset_from_ptr(ptr);
        assert_eq!(
            recovered_offset, test_offset,
            "Round-trip offset -> ptr -> offset should return same offset"
        );

        // Test with pointer from an allocation
        let alloc_ptr = unsafe { pma.alloc_indirect(10) };
        let alloc_offset = pma.offset_from_ptr(alloc_ptr as *const u8);
        let recovered_ptr = pma.ptr_from_offset(alloc_offset);
        assert_eq!(
            recovered_ptr, alloc_ptr as *mut u8,
            "Round-trip ptr -> offset -> ptr should return same pointer"
        );

        // Test multiple allocations have distinct offsets
        let ptr1 = unsafe { pma.alloc_indirect(5) };
        let ptr2 = unsafe { pma.alloc_indirect(5) };
        let offset1 = pma.offset_from_ptr(ptr1 as *const u8);
        let offset2 = pma.offset_from_ptr(ptr2 as *const u8);
        assert_ne!(
            offset1, offset2,
            "Different allocations should have different offsets"
        );

        // Verify the offsets differ by the expected amount (5 + 2 = 7 words)
        assert_eq!(
            offset2 - offset1,
            7,
            "Second allocation offset should be 7 words after first"
        );
    }
}
