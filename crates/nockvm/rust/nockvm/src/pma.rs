//! Persistent Memory Arena (PMA)
//!
//! The PMA is a file-backed memory region for storing long-lived Nouns.
//! It uses bump allocation and stores nouns in offset form.

use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use thiserror::Error;

use crate::mem::{Arena, NewStackError};
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
pub struct Pma {
    /// The underlying arena for memory management and pointer resolution
    arena: Arc<Arena>,
    /// Current allocation offset in words (bump pointer)
    alloc_offset: AtomicUsize,
    /// Path to the backing file (for future file-backed persistence)
    path: PathBuf,
}

impl Pma {
    /// Create a new PMA with the given size in words
    pub fn new(_size_words: usize, _path: PathBuf) -> Result<Self, PmaError> {
        todo!()
    }

    /// Get the underlying arena
    pub fn arena(&self) -> &Arc<Arena> {
        todo!()
    }

    /// Get the current allocation offset in words
    pub fn alloc_offset(&self) -> usize {
        todo!()
    }

    /// Get the total size of the PMA in words
    pub fn size_words(&self) -> usize {
        todo!()
    }

    /// Get the number of free words remaining
    pub fn free_words(&self) -> usize {
        todo!()
    }

    /// Convert a pointer within the PMA to an offset in words
    pub fn offset_from_ptr(&self, _ptr: *const u8) -> u32 {
        todo!()
    }

    /// Convert an offset in words to a pointer
    pub fn ptr_from_offset(&self, _offset_words: u32) -> *mut u8 {
        todo!()
    }

    /// Check if a pointer is within the PMA's memory region
    pub fn contains_ptr(&self, _ptr: *const u8) -> bool {
        todo!()
    }

    /// Reset the allocation pointer to zero
    pub fn reset(&self) {
        todo!()
    }

    /// Reset the allocation pointer to a specific offset
    pub fn reset_to(&self, _offset: usize) {
        todo!()
    }
}

impl ibig::Stack for Pma {
    unsafe fn alloc_layout(&mut self, _layout: std::alloc::Layout) -> *mut u64 {
        todo!()
    }
}

impl NounAllocator for Pma {
    unsafe fn alloc_indirect(&mut self, _words: usize) -> *mut u64 {
        todo!()
    }

    unsafe fn alloc_cell(&mut self) -> *mut CellMemory {
        todo!()
    }

    unsafe fn alloc_struct<T>(&mut self, _count: usize) -> *mut T {
        todo!()
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
}
