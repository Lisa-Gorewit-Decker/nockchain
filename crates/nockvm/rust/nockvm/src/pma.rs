//! Persistent Memory Arena (PMA)
//!
//! The PMA is a file-backed memory region for storing long-lived Nouns.
//! It uses bump allocation and stores nouns in offset form.

use std::path::PathBuf;
use std::ptr::copy_nonoverlapping;
use std::sync::Arc;

use either::Either::{Left, Right};
use thiserror::Error;

use crate::ext::noun_equality;
use crate::mem::{word_size_of, Arena, NewStackError, NockStack};
use crate::noun::{Cell, CellMemory, IndirectAtom, Noun, NounAllocator};

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
        &self.arena
    }

    /// Install the PMA's arena in thread-local storage.
    ///
    /// Returns a guard that automatically clears the thread-local when dropped.
    /// This allows `Arena::with_current()` to access the PMA's arena.
    pub fn install(&self) -> PmaInstallGuard {
        Arena::set_thread_local(&self.arena);
        PmaInstallGuard { _private: () }
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
    pub fn contains_ptr(&self, ptr: *const u8) -> bool {
        let base = self.arena.base_ptr() as usize;
        let end = base + self.arena.len_bytes();
        let ptr_addr = ptr as usize;
        ptr_addr >= base && ptr_addr < end
    }

    /// Reset the allocation pointer to zero
    pub fn reset(&mut self) {
        self.alloc_offset = 0;
    }

    /// Reset the allocation pointer to a specific offset
    ///
    /// # Panics
    /// Panics if `offset` is greater than the PMA size.
    pub fn reset_to(&mut self, offset: usize) {
        assert!(
            offset <= self.size_words(),
            "reset_to offset {} exceeds PMA size {}",
            offset,
            self.size_words()
        );
        self.alloc_offset = offset;
    }

    /// Check if an allocation of `words` would exceed available space.
    ///
    /// # Panics
    /// Panics with `PmaError::OutOfMemory` if there isn't enough space.
    pub fn alloc_would_oom(&self, words: usize) {
        if words > self.free_words() {
            panic!(
                "{}",
                PmaError::OutOfMemory {
                    requested: words,
                    available: self.free_words(),
                }
            );
        }
    }

    /// Allocate `words` from the PMA, returning a pointer to the allocation.
    ///
    /// # Panics
    /// Panics if there isn't enough space in the PMA.
    unsafe fn raw_alloc(&mut self, words: usize) -> *mut u64 {
        self.alloc_would_oom(words);
        let ptr = self.arena.ptr_from_offset(self.alloc_offset as u32) as *mut u64;
        self.alloc_offset += words;
        ptr
    }
}

/// RAII guard for PMA arena installation.
///
/// When this guard is dropped, it automatically clears the thread-local arena.
/// This ensures the arena is only installed for the lifetime of the guard.
///
/// Note: Using `()` makes this a zero-sized type. If we need the ability to
/// "disarm" the guard (skip cleanup on drop), we could switch to a `bool` field
/// like `ReplicaInstallGuard` uses. See `ReplicaInstallGuard` in mem.rs for comparison.
pub struct PmaInstallGuard {
    /// Private field to prevent construction outside of Pma::install()
    _private: (),
}

impl Drop for PmaInstallGuard {
    fn drop(&mut self) {
        Arena::clear_thread_local();
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

    unsafe fn equals(&mut self, a: *mut Noun, b: *mut Noun) -> bool {
        let a = &*a;
        let b = &*b;
        noun_equality(a, b)
    }
}

/// Trait for types that can be copied into the PMA.
///
/// This is used to evacuate nouns from the NockStack to the PMA for persistence.
pub trait PmaCopy {
    /// Copy this value into the PMA.
    ///
    /// For nouns, this evacuates allocated data (indirect atoms, cells) to the PMA
    /// and converts pointers to offset form. Direct atoms are unchanged since they
    /// fit in a single word.
    ///
    /// # Safety
    /// The caller must ensure that the stack's arena is installed in thread-local storage.
    unsafe fn copy_to_pma(&mut self, stack: &NockStack, pma: &mut Pma);

    /// Assert that this value is fully contained within the PMA.
    ///
    /// For nouns, this verifies that all allocated data (indirect atoms, cells)
    /// resides in the PMA. Direct atoms always pass since they have no allocations.
    ///
    /// # Panics
    /// Panics if any part of this value is not in the PMA.
    fn assert_in_pma(&self, pma: &Pma);
}

impl PmaCopy for Noun {
    /// Copy a noun and all its allocated substructure to the PMA.
    ///
    /// Uses a worklist algorithm to avoid stack overflow on deep structures.
    /// Structural sharing is preserved via forwarding pointers: if the same
    /// substructure is referenced multiple times, it's only copied once.
    ///
    /// # Algorithm
    /// 1. Push (noun, destination_ptr) onto worklist
    /// 2. Pop and process each item:
    ///    - Direct atoms: write directly to destination
    ///    - Already in PMA (offset form): write directly to destination
    ///    - Has forwarding pointer: write forwarded offset-form to destination
    ///    - Indirect atom: copy to PMA, set forwarding pointer, write offset-form
    ///    - Cell: copy metadata to PMA, set forwarding pointer, queue head/tail
    ///
    /// # Safety
    /// - The PMA arena should be installed for reading evacuated nouns afterward
    /// - Source nouns will have forwarding pointers set (corrupting the stack data)
    unsafe fn copy_to_pma(&mut self, _stack: &NockStack, pma: &mut Pma) {
        // Direct atoms fit in a single word and don't need evacuation
        if self.is_direct() {
            return;
        }

        // Already in offset form (already in PMA) - nothing to do
        if !self.is_stack_allocated() {
            return;
        }

        // Clone the Arc to avoid borrow conflicts during mutation
        //TODO not sure this is right
        let arena = Arc::clone(pma.arena());

        // Worklist of (source noun, destination pointer)
        // Destination pointers are either the root noun or fields within PMA cells
        let mut work: Vec<(Noun, *mut Noun)> = Vec::with_capacity(32);
        work.push((*self, self as *mut Noun));

        while let Some((noun, dest_ptr)) = work.pop() {
            match noun.as_either_direct_allocated() {
                Left(_direct) => {
                    // Direct atoms are copied as-is (no allocation needed)
                    *dest_ptr = noun;
                }
                Right(allocated) => {
                    // Check for forwarding pointer (already evacuated, structural sharing)
                    if let Some(forwarded) = allocated.forwarding_pointer_with_arena(&arena) {
                        // Convert forwarded pointer to offset form
                        let pma_ptr = forwarded.to_raw_pointer_with_arena(&arena);
                        let offset = pma.offset_from_ptr(pma_ptr as *const u8);
                        if allocated.is_indirect() {
                            *dest_ptr = IndirectAtom::from_offset_words(offset).as_noun();
                        } else {
                            *dest_ptr = Cell::from_offset_words(offset).as_noun();
                        }
                        continue;
                    }

                    // Already in offset form (already in PMA)
                    if !noun.is_stack_allocated() {
                        *dest_ptr = noun;
                        continue;
                    }

                    match allocated.as_either() {
                        Left(mut indirect) => {
                            // Get size and source pointer before allocating
                            let raw_size = indirect.raw_size_with_arena(&arena);
                            let src_ptr = indirect.to_raw_pointer_with_arena(&arena);

                            // Allocate in PMA
                            let pma_ptr = pma.raw_alloc(raw_size);

                            // Copy all data (metadata + size + data words)
                            copy_nonoverlapping(src_ptr, pma_ptr, raw_size);

                            // Set forwarding pointer in source for structural sharing
                            indirect.set_forwarding_pointer_with_arena(pma_ptr, &arena);

                            // Write offset-form noun to destination
                            let offset = pma.offset_from_ptr(pma_ptr as *const u8);
                            *dest_ptr = IndirectAtom::from_offset_words(offset).as_noun();
                        }
                        Right(mut cell) => {
                            // Get source cell pointer
                            let src_cell = cell.to_raw_pointer_with_arena(&arena);

                            // Allocate cell in PMA
                            let pma_ptr = pma.raw_alloc(word_size_of::<CellMemory>());
                            let pma_cell = pma_ptr as *mut CellMemory;

                            // Copy metadata
                            (*pma_cell).metadata = (*src_cell).metadata;

                            // Get head and tail BEFORE setting forwarding pointer
                            // (forwarding pointer overwrites head field)
                            let head = (*src_cell).head;
                            let tail = (*src_cell).tail;

                            // Set forwarding pointer in source for structural sharing
                            cell.set_forwarding_pointer_with_arena(pma_cell, &arena);

                            // Queue head and tail for processing
                            // Destinations are the head/tail slots in the PMA cell
                            work.push((tail, &mut (*pma_cell).tail));
                            work.push((head, &mut (*pma_cell).head));

                            // Write offset-form cell to destination
                            let offset = pma.offset_from_ptr(pma_ptr as *const u8);
                            *dest_ptr = Cell::from_offset_words(offset).as_noun();
                        }
                    }
                }
            }
        }
    }

    /// Assert that this noun and all its substructure is in the PMA.
    ///
    /// # Panics
    /// Panics if any allocated part of the noun is stack-allocated rather than
    /// in offset form (PMA).
    ///
    /// # Note
    /// The PMA arena must be installed before calling this for cells, as it needs
    /// to resolve cell head/tail pointers.
    fn assert_in_pma(&self, pma: &Pma) {
        // Direct atoms have no allocations, so they're trivially "in" the PMA
        if self.is_direct() {
            return;
        }

        // Check that allocated nouns are in offset form (not stack-allocated)
        assert!(
            !self.is_stack_allocated(),
            "Noun is stack-allocated, not in PMA"
        );

        // For cells, recursively check head and tail
        if self.is_cell() {
            let cell = self.as_cell().expect("checked is_cell");
            // Arena must be installed by caller for head()/tail() to work
            cell.head().assert_in_pma(pma);
            cell.tail().assert_in_pma(pma);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hamt::Hamt;
    use crate::jets::cold::NounListMem;
    use crate::mem::{word_size_of, Arena, NockStack};
    use crate::noun::{D, DIRECT_MAX};
    use ibig::Stack;
    use std::alloc::Layout;
    use std::sync::Arc;

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

    /// Verifies contains_ptr correctly identifies pointers inside vs outside the PMA.
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_pma_contains_ptr() {
        let mut pma = test_pma(1000);

        // Get base pointer and compute some test pointers
        let base = pma.arena().base_ptr();
        let len_bytes = pma.arena().len_bytes();

        // Base pointer should be in PMA
        assert!(pma.contains_ptr(base), "Base pointer should be in PMA");

        // Pointer at offset 0 should be in PMA
        let ptr_at_0 = pma.ptr_from_offset(0);
        assert!(pma.contains_ptr(ptr_at_0), "Pointer at offset 0 should be in PMA");

        // Pointer in the middle should be in PMA
        let middle_offset = 500u32;
        let ptr_middle = pma.ptr_from_offset(middle_offset);
        assert!(pma.contains_ptr(ptr_middle), "Pointer in middle should be in PMA");

        // Last valid byte should be in PMA
        let last_byte = unsafe { base.add(len_bytes - 1) };
        assert!(pma.contains_ptr(last_byte), "Last byte should be in PMA");

        // Pointer just past the end should NOT be in PMA
        let past_end = unsafe { base.add(len_bytes) };
        assert!(!pma.contains_ptr(past_end), "Pointer past end should not be in PMA");

        // Pointer well past the end should NOT be in PMA
        let way_past_end = unsafe { base.add(len_bytes + 1000) };
        assert!(!pma.contains_ptr(way_past_end), "Pointer way past end should not be in PMA");

        // Pointer before the base should NOT be in PMA (if base > 0)
        if base as usize > 0 {
            let before_base = unsafe { base.sub(1) };
            assert!(!pma.contains_ptr(before_base), "Pointer before base should not be in PMA");
        }

        // Null pointer should NOT be in PMA
        assert!(!pma.contains_ptr(std::ptr::null()), "Null pointer should not be in PMA");

        // Allocated pointer should be in PMA
        let alloc_ptr = unsafe { pma.alloc_indirect(10) };
        assert!(pma.contains_ptr(alloc_ptr as *const u8), "Allocated pointer should be in PMA");
    }

    /// Verifies allocation fails gracefully when PMA is full.
    ///
    /// This test exercises:
    /// - alloc_would_oom() does not panic when there's space
    /// - alloc_would_oom() panics when there isn't enough space
    /// - Exact-fit allocations succeed
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_pma_out_of_memory() {
        use std::panic::{catch_unwind, AssertUnwindSafe};

        let mut pma = test_pma(100); // Small PMA: 100 words

        // alloc_would_oom should not panic when there's space
        pma.alloc_would_oom(50); // Should not panic
        pma.alloc_would_oom(100); // Should not panic (exact fit)

        // alloc_would_oom should panic when there isn't space
        let result = catch_unwind(AssertUnwindSafe(|| {
            pma.alloc_would_oom(101);
        }));
        assert!(result.is_err(), "alloc_would_oom(101) should panic with 100 free");

        // Allocate some space
        unsafe { pma.alloc_indirect(10) }; // 12 words (10 + 2 for metadata/size)
        assert_eq!(pma.alloc_offset(), 12);
        assert_eq!(pma.free_words(), 88);

        // alloc_would_oom should reflect remaining space
        pma.alloc_would_oom(88); // Should not panic
        let result = catch_unwind(AssertUnwindSafe(|| {
            pma.alloc_would_oom(89);
        }));
        assert!(result.is_err(), "alloc_would_oom(89) should panic with 88 free");

        // Fill the rest
        unsafe { pma.alloc_struct::<u64>(88) };
        assert_eq!(pma.alloc_offset(), 100);
        assert_eq!(pma.free_words(), 0);

        // alloc_would_oom should panic for any non-zero allocation when full
        let result = catch_unwind(AssertUnwindSafe(|| {
            pma.alloc_would_oom(1);
        }));
        assert!(result.is_err(), "alloc_would_oom(1) should panic when full");

        // But 0 words should not panic
        pma.alloc_would_oom(0); // Should not panic

        // Reset and verify we can allocate again
        pma.reset();
        assert_eq!(pma.free_words(), 100);
        pma.alloc_would_oom(100); // Should not panic after reset

        // Verify exact-fit allocation works
        unsafe { pma.alloc_struct::<u64>(100) };
        assert_eq!(pma.alloc_offset(), 100);
        assert_eq!(pma.free_words(), 0);
    }

    /// Verifies reset() and reset_to() correctly manage the allocation pointer.
    ///
    /// This test exercises:
    /// - reset() sets alloc_offset back to 0
    /// - reset_to(offset) sets alloc_offset to a specific value
    /// - After reset, free_words equals size again
    /// - Allocations after reset start from the reset point
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_pma_reset() {
        let mut pma = test_pma(1000);

        // Allocate some space
        unsafe { pma.alloc_indirect(10) }; // 12 words
        unsafe { pma.alloc_indirect(20) }; // 22 words
        assert_eq!(pma.alloc_offset(), 34);
        assert_eq!(pma.free_words(), 966);

        // Reset to zero
        pma.reset();
        assert_eq!(pma.alloc_offset(), 0, "reset() should set offset to 0");
        assert_eq!(pma.free_words(), 1000, "reset() should restore all free space");

        // Allocations after reset should start from 0
        let ptr_after_reset = unsafe { pma.alloc_indirect(5) }; // 7 words
        assert_eq!(pma.alloc_offset(), 7);
        let offset_after_reset = pma.offset_from_ptr(ptr_after_reset as *const u8);
        assert_eq!(offset_after_reset, 0, "First allocation after reset should be at offset 0");

        // Allocate more to create a checkpoint
        unsafe { pma.alloc_indirect(10) }; // 12 more words
        let checkpoint = pma.alloc_offset();
        assert_eq!(checkpoint, 19); // 7 + 12

        // Allocate even more
        unsafe { pma.alloc_indirect(30) }; // 32 more words
        assert_eq!(pma.alloc_offset(), 51); // 19 + 32

        // Reset to checkpoint
        pma.reset_to(checkpoint);
        assert_eq!(pma.alloc_offset(), 19, "reset_to() should set offset to checkpoint");
        assert_eq!(pma.free_words(), 981, "reset_to() should restore free space from checkpoint");

        // Next allocation should start at the checkpoint
        let ptr_after_reset_to = unsafe { pma.alloc_indirect(3) }; // 5 words
        let offset_after_reset_to = pma.offset_from_ptr(ptr_after_reset_to as *const u8);
        assert_eq!(offset_after_reset_to, 19, "Allocation after reset_to should start at checkpoint");
        assert_eq!(pma.alloc_offset(), 24); // 19 + 5
    }

    /// Verifies reset_to panics when given an offset outside the PMA bounds.
    #[test]
    #[should_panic(expected = "reset_to offset")]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_pma_reset_to_out_of_bounds() {
        let mut pma = test_pma(1000);
        pma.reset_to(1001); // Should panic: offset exceeds PMA size
    }

    /// Verifies thread-local PMA installation, access via with_current(), and RAII cleanup.
    ///
    /// This test exercises:
    /// - pma.install() installs the PMA's arena in thread-local storage
    /// - Arena::with_current() can access the installed arena
    /// - The installed arena matches the PMA's arena
    /// - PmaInstallGuard automatically clears the arena when dropped
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_pma_thread_local() {
        let pma = test_pma(1000);
        let pma_arena_ptr = Arc::as_ptr(pma.arena());

        {
            // Install the PMA's arena - guard will clear on drop
            let _guard = pma.install();

            // Verify we can access it via with_current and it's the same arena
            Arena::with_current(|arena| {
                let current_ptr = arena as *const Arena;
                assert_eq!(
                    current_ptr, pma_arena_ptr,
                    "Installed arena should match PMA's arena"
                );
            });
        } // _guard dropped here, arena should be cleared

        // Verify the guard cleared the thread-local by checking that
        // with_current would now panic (we don't call it here to avoid panic,
        // but we test this in test_pma_thread_local_not_installed)
    }

    /// Verifies Arena::with_current panics when no arena is installed.
    #[test]
    #[should_panic(expected = "Arena::with_current called without an installed Arena")]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_pma_thread_local_not_installed() {
        // Ensure no arena is installed
        Arena::clear_thread_local();

        // This should panic
        Arena::with_current(|_arena| {});
    }

    /// Verifies PmaInstallGuard clears the arena when dropped.
    #[test]
    #[should_panic(expected = "Arena::with_current called without an installed Arena")]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_pma_guard_clears_on_drop() {
        let pma = test_pma(1000);

        // Ensure no arena is installed initially
        Arena::clear_thread_local();

        {
            let _guard = pma.install();
            // Arena is installed here, with_current would work
        } // _guard dropped, arena should be cleared

        // This should panic because the guard cleared the arena
        Arena::with_current(|_arena| {});
    }

    /// Verifies direct atoms are unchanged by evacuation since they fit in a single word.
    ///
    /// Direct atoms don't require any allocation - they're just 64-bit values with
    /// the MSB = 0. Evacuation should leave them completely unchanged.
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_evacuate_direct_atom() {
        let stack = NockStack::new(1 << 10, 0);
        let mut pma = test_pma(1000);

        // Test several direct atom values
        let test_values: [u64; 5] = [0, 1, 42, 12345, DIRECT_MAX];

        for &val in &test_values {
            let mut noun = D(val);
            let original_raw = unsafe { noun.as_raw() };

            // Evacuate to PMA
            unsafe { noun.copy_to_pma(&stack, &mut pma) };

            // Direct atoms should be completely unchanged
            let after_raw = unsafe { noun.as_raw() };
            assert_eq!(
                original_raw, after_raw,
                "Direct atom {} should be unchanged after evacuation",
                val
            );

            // Verify it's still a direct atom
            assert!(noun.is_direct(), "Should still be a direct atom after evacuation");

            // Direct atoms should trivially pass assert_in_pma (no allocations to check)
            noun.assert_in_pma(&pma);
        }

        // PMA should have no allocations (direct atoms don't need space)
        assert_eq!(
            pma.alloc_offset(),
            0,
            "No allocations should be made for direct atoms"
        );
    }

    /// Verifies indirect atoms (too large for direct representation) are copied to PMA
    /// and converted to offset form.
    ///
    /// This test exercises:
    /// - Creating an indirect atom on the NockStack
    /// - Evacuating it to the PMA via copy_to_pma
    /// - Verifying the atom is now in offset form (LOCATION_BIT set)
    /// - Verifying the data can be read correctly via the PMA arena
    /// - Verifying PMA allocations were made
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_evacuate_indirect_atom() {
        let mut stack = NockStack::new(1 << 10, 0);
        let mut pma = test_pma(1000);

        // Create an indirect atom on the stack (value > DIRECT_MAX requires indirect storage)
        // We'll use a 2-word value to ensure it's indirect
        let data: [u64; 2] = [0xDEADBEEF_CAFEBABE, 0x12345678_9ABCDEF0];
        let indirect = unsafe { IndirectAtom::new_raw(&mut stack, 2, data.as_ptr()) };
        let mut noun = indirect.as_noun();

        // Verify it's an indirect atom on the stack
        assert!(noun.is_indirect(), "Should be an indirect atom");
        assert!(
            !noun.is_direct(),
            "Should not be a direct atom"
        );
        assert!(
            noun.is_stack_allocated(),
            "Should be stack-allocated before evacuation"
        );

        // Record the initial PMA offset
        let initial_offset = pma.alloc_offset();
        assert_eq!(initial_offset, 0, "PMA should start empty");

        // Install the PMA arena for pointer resolution after evacuation
        let _guard = pma.install();

        // Evacuate to PMA
        unsafe { noun.copy_to_pma(&stack, &mut pma) };

        // Verify PMA allocation was made
        // Indirect atom needs: metadata (1) + size (1) + data (2) = 4 words
        assert!(
            pma.alloc_offset() > initial_offset,
            "PMA should have allocations after evacuation"
        );
        assert_eq!(
            pma.alloc_offset(),
            4, // metadata + size + 2 data words
            "Indirect atom should allocate 4 words in PMA"
        );

        // Verify the noun is now in offset form (not stack-allocated)
        assert!(
            !noun.is_stack_allocated(),
            "Should be in offset form after evacuation"
        );
        assert!(noun.is_indirect(), "Should still be an indirect atom");

        // Verify data is readable and correct via PMA arena
        let atom = noun.as_atom().expect("Should be an atom");
        let read_indirect = atom.as_indirect().expect("Should be indirect");

        // Read the size - should be 2 words
        let size = read_indirect.size();
        assert_eq!(size, 2, "Indirect atom should have size 2");

        // Read the data back and verify it matches
        let data_ptr = read_indirect.data_pointer();
        let read_data = unsafe { std::slice::from_raw_parts(data_ptr, 2) };
        assert_eq!(
            read_data[0], data[0],
            "First data word should match"
        );
        assert_eq!(
            read_data[1], data[1],
            "Second data word should match"
        );

        // Verify assert_in_pma passes
        noun.assert_in_pma(&pma);
    }

    /// Verifies a simple cell with direct atom contents is evacuated and readable from PMA.
    ///
    /// This test exercises:
    /// - Creating a cell [head tail] on the NockStack
    /// - Evacuating it to the PMA
    /// - Verifying the cell is in offset form
    /// - Verifying head and tail are readable and correct
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_evacuate_simple_cell() {
        let mut stack = NockStack::new(1 << 10, 0);
        let mut pma = test_pma(1000);

        // Create a simple cell [42 123] with direct atoms
        let mut noun = Cell::new(&mut stack, D(42), D(123)).as_noun();

        // Verify it's a cell on the stack
        assert!(noun.is_cell(), "Should be a cell");
        assert!(noun.is_stack_allocated(), "Should be stack-allocated before evacuation");

        // Install PMA arena and evacuate
        let _guard = pma.install();
        unsafe { noun.copy_to_pma(&stack, &mut pma) };

        // Verify PMA allocation was made (CellMemory size)
        let cell_words = word_size_of::<CellMemory>();
        assert_eq!(
            pma.alloc_offset(),
            cell_words,
            "Cell should allocate {} words",
            cell_words
        );

        // Verify the noun is now in offset form
        assert!(!noun.is_stack_allocated(), "Should be in offset form after evacuation");
        assert!(noun.is_cell(), "Should still be a cell");

        // Read head and tail
        let cell = noun.as_cell().expect("Should be a cell");
        let head = cell.head();
        let tail = cell.tail();

        // Verify head and tail are correct direct atoms
        assert!(head.is_direct(), "Head should be direct");
        assert!(tail.is_direct(), "Tail should be direct");
        assert_eq!(
            head.as_direct().expect("head is direct").data(),
            42,
            "Head should be 42"
        );
        assert_eq!(
            tail.as_direct().expect("tail is direct").data(),
            123,
            "Tail should be 123"
        );

        // Verify assert_in_pma passes
        noun.assert_in_pma(&pma);
    }

    /// Verifies nested cell structures are fully evacuated with all sub-cells in offset form.
    ///
    /// This test exercises:
    /// - Creating nested cells [[1 2] [3 4]]
    /// - Evacuating the entire structure
    /// - Verifying all cells are in offset form
    /// - Verifying all values are readable
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_evacuate_nested_cells() {
        let mut stack = NockStack::new(1 << 10, 0);
        let mut pma = test_pma(1000);

        // Create nested cells: [[1 2] [3 4]]
        let left = Cell::new(&mut stack, D(1), D(2)).as_noun();
        let right = Cell::new(&mut stack, D(3), D(4)).as_noun();
        let mut noun = Cell::new(&mut stack, left, right).as_noun();

        // Verify structure before evacuation
        assert!(noun.is_cell(), "Root should be a cell");
        assert!(noun.is_stack_allocated(), "Root should be stack-allocated");

        // Install PMA arena and evacuate
        let _guard = pma.install();
        unsafe { noun.copy_to_pma(&stack, &mut pma) };

        // Should allocate 3 cells worth of space
        let cell_words = word_size_of::<CellMemory>();
        assert_eq!(
            pma.alloc_offset(),
            cell_words * 3,
            "Should allocate 3 cells"
        );

        // Verify root is in offset form
        assert!(!noun.is_stack_allocated(), "Root should be in offset form");

        // Navigate and verify structure
        let root = noun.as_cell().expect("root is cell");
        let left_cell = root.head().as_cell().expect("left is cell");
        let right_cell = root.tail().as_cell().expect("right is cell");

        // Verify left cell [1 2]
        assert!(!root.head().is_stack_allocated(), "Left should be in offset form");
        assert_eq!(left_cell.head().as_direct().expect("1").data(), 1);
        assert_eq!(left_cell.tail().as_direct().expect("2").data(), 2);

        // Verify right cell [3 4]
        assert!(!root.tail().is_stack_allocated(), "Right should be in offset form");
        assert_eq!(right_cell.head().as_direct().expect("3").data(), 3);
        assert_eq!(right_cell.tail().as_direct().expect("4").data(), 4);

        // Verify assert_in_pma passes for entire structure
        noun.assert_in_pma(&pma);
    }

    /// Verifies cells containing indirect atoms have both the cell and atoms correctly evacuated.
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_evacuate_cell_with_indirect_atoms() {
        let mut stack = NockStack::new(1 << 10, 0);
        let mut pma = test_pma(1000);

        // Create indirect atoms
        let data1: [u64; 2] = [0xAAAAAAAA_BBBBBBBB, 0xCCCCCCCC_DDDDDDDD];
        let data2: [u64; 2] = [0x11111111_22222222, 0x33333333_44444444];
        let indirect1 = unsafe { IndirectAtom::new_raw(&mut stack, 2, data1.as_ptr()) };
        let indirect2 = unsafe { IndirectAtom::new_raw(&mut stack, 2, data2.as_ptr()) };

        // Create cell with indirect atoms
        let mut noun = Cell::new(&mut stack, indirect1.as_noun(), indirect2.as_noun()).as_noun();

        assert!(noun.is_stack_allocated(), "Should be stack-allocated");

        // Install PMA arena and evacuate
        let _guard = pma.install();
        unsafe { noun.copy_to_pma(&stack, &mut pma) };

        // Should allocate: 1 cell + 2 indirect atoms (4 words each)
        let cell_words = word_size_of::<CellMemory>();
        let indirect_words = 4; // metadata + size + 2 data words
        assert_eq!(
            pma.alloc_offset(),
            cell_words + indirect_words * 2,
            "Should allocate cell + 2 indirect atoms"
        );

        // Verify structure
        assert!(!noun.is_stack_allocated(), "Root should be in offset form");

        let cell = noun.as_cell().expect("is cell");
        let head = cell.head();
        let tail = cell.tail();

        // Verify head is indirect atom with correct data
        assert!(head.is_indirect(), "Head should be indirect");
        assert!(!head.is_stack_allocated(), "Head should be in offset form");
        let head_indirect = head.as_indirect().expect("head indirect");
        let head_data = unsafe { std::slice::from_raw_parts(head_indirect.data_pointer(), 2) };
        assert_eq!(head_data[0], data1[0]);
        assert_eq!(head_data[1], data1[1]);

        // Verify tail is indirect atom with correct data
        assert!(tail.is_indirect(), "Tail should be indirect");
        assert!(!tail.is_stack_allocated(), "Tail should be in offset form");
        let tail_indirect = tail.as_indirect().expect("tail indirect");
        let tail_data = unsafe { std::slice::from_raw_parts(tail_indirect.data_pointer(), 2) };
        assert_eq!(tail_data[0], data2[0]);
        assert_eq!(tail_data[1], data2[1]);

        noun.assert_in_pma(&pma);
    }

    /// Verifies structural sharing is preserved: [x x] evacuates x only once.
    ///
    /// When the same noun is referenced multiple times, the forwarding pointer
    /// mechanism ensures it's only copied once, and both references point to
    /// the same PMA location.
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_evacuate_shared_structure() {
        let mut stack = NockStack::new(1 << 10, 0);
        let mut pma = test_pma(1000);

        // Create a shared subcell
        let shared = Cell::new(&mut stack, D(1), D(2)).as_noun();

        // Create [shared shared] - both head and tail point to same cell
        let mut noun = Cell::new(&mut stack, shared, shared).as_noun();

        // Install PMA arena and evacuate
        let _guard = pma.install();
        unsafe { noun.copy_to_pma(&stack, &mut pma) };

        // Should allocate only 2 cells: the root and the shared subcell (not 3!)
        let cell_words = word_size_of::<CellMemory>();
        assert_eq!(
            pma.alloc_offset(),
            cell_words * 2,
            "Should allocate only 2 cells due to sharing"
        );

        // Verify both head and tail point to the same PMA location
        let root = noun.as_cell().expect("is cell");
        let head_raw = unsafe { root.head().as_raw() };
        let tail_raw = unsafe { root.tail().as_raw() };
        assert_eq!(
            head_raw, tail_raw,
            "Head and tail should point to same location (sharing preserved)"
        );

        // Verify the shared cell is correct
        let shared_cell = root.head().as_cell().expect("shared is cell");
        assert_eq!(shared_cell.head().as_direct().expect("1").data(), 1);
        assert_eq!(shared_cell.tail().as_direct().expect("2").data(), 2);

        noun.assert_in_pma(&pma);
    }

    /// Verifies evacuating an already-evacuated noun is a no-op that allocates nothing.
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_evacuate_already_evacuated() {
        let mut stack = NockStack::new(1 << 10, 0);
        let mut pma = test_pma(1000);

        // Create and evacuate a cell
        let mut noun = Cell::new(&mut stack, D(1), D(2)).as_noun();
        let _guard = pma.install();
        unsafe { noun.copy_to_pma(&stack, &mut pma) };

        let offset_after_first = pma.alloc_offset();
        assert!(offset_after_first > 0, "Should have allocated something");

        // Evacuate again - should be a no-op
        unsafe { noun.copy_to_pma(&stack, &mut pma) };

        assert_eq!(
            pma.alloc_offset(),
            offset_after_first,
            "Second evacuation should not allocate anything"
        );

        noun.assert_in_pma(&pma);
    }

    /// Verifies deeply nested structures are fully evacuated and traversable after evacuation.
    ///
    /// This test exercises the worklist algorithm's ability to handle deep trees
    /// without stack overflow (since we use iteration, not recursion).
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_evacuate_deep_tree() {
        let mut stack = NockStack::new(1 << 14, 0); // Larger stack for deep nesting
        let mut pma = test_pma(10000);

        // Create a deeply nested structure: [1 [2 [3 [4 ... [999 1000]]]]]
        const DEPTH: u64 = 500;

        // Build from the inside out
        let mut noun = D(DEPTH);
        for i in (1..DEPTH).rev() {
            noun = Cell::new(&mut stack, D(i), noun).as_noun();
        }

        // Verify it's deeply nested and stack-allocated
        assert!(noun.is_cell(), "Root should be a cell");
        assert!(noun.is_stack_allocated(), "Should be stack-allocated");

        // Install PMA arena (needed for Cell::tail() even on stack-allocated nouns)
        let _guard = pma.install();

        // Count the depth before evacuation
        let mut depth_before = 0u64;
        let mut current = noun;
        while current.is_cell() {
            depth_before += 1;
            current = current.as_cell().unwrap().tail();
        }
        assert_eq!(depth_before, DEPTH - 1, "Should have correct depth before evacuation");

        // Evacuate
        unsafe { noun.copy_to_pma(&stack, &mut pma) };

        // Should allocate (DEPTH - 1) cells
        let cell_words = word_size_of::<CellMemory>();
        assert_eq!(
            pma.alloc_offset(),
            cell_words * (DEPTH as usize - 1),
            "Should allocate {} cells",
            DEPTH - 1
        );

        // Verify root is in offset form
        assert!(!noun.is_stack_allocated(), "Root should be in offset form");

        // Traverse the entire structure and verify values
        let mut current = noun;
        for expected in 1..DEPTH {
            assert!(current.is_cell(), "Should be cell at depth {}", expected);
            let cell = current.as_cell().expect("is cell");

            // Verify head value
            let head = cell.head();
            assert!(head.is_direct(), "Head at depth {} should be direct", expected);
            assert_eq!(
                head.as_direct().expect("direct").data(),
                expected,
                "Head at depth {} should be {}",
                expected,
                expected
            );

            // Verify this cell is in offset form
            assert!(
                !current.is_stack_allocated(),
                "Cell at depth {} should be in offset form",
                expected
            );

            current = cell.tail();
        }

        // Final element should be direct atom DEPTH
        assert!(current.is_direct(), "Leaf should be direct atom");
        assert_eq!(
            current.as_direct().expect("direct").data(),
            DEPTH,
            "Leaf should be {}",
            DEPTH
        );

        // Verify assert_in_pma passes for entire structure
        noun.assert_in_pma(&pma);
    }

    /// Verifies deeply nested structures with variable-sized indirect atoms are fully evacuated.
    ///
    /// Similar to test_evacuate_deep_tree, but each value is an IndirectAtom with
    /// data size varying from 2 to 10 words. This tests the evacuation of mixed
    /// cell/indirect-atom structures with variable allocation sizes.
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_evacuate_deep_tree_indirect_atoms() {
        let mut stack = NockStack::new(1 << 16, 0); // Larger stack for indirect atoms
        let mut pma = test_pma(100000); // Larger PMA for indirect atoms

        const DEPTH: usize = 200;

        // Helper to create an indirect atom with `word_count` words of data
        // Data pattern: first word is the index, remaining words are index + word_position
        let make_indirect = |stack: &mut NockStack, index: usize, word_count: usize| -> Noun {
            let mut data = vec![0u64; word_count];
            for (i, word) in data.iter_mut().enumerate() {
                *word = (index as u64) << 32 | (i as u64);
            }
            unsafe {
                IndirectAtom::new_raw(stack, word_count, data.as_ptr()).as_noun()
            }
        };

        // Helper to compute word count for index (varies 2-10)
        let word_count_for_index = |index: usize| -> usize { (index % 9) + 2 };

        // Build from inside out: [indirect_1 [indirect_2 [indirect_3 ... indirect_DEPTH]]]
        let mut noun = make_indirect(&mut stack, DEPTH, word_count_for_index(DEPTH));
        for i in (1..DEPTH).rev() {
            let head = make_indirect(&mut stack, i, word_count_for_index(i));
            noun = Cell::new(&mut stack, head, noun).as_noun();
        }

        // Verify structure before evacuation
        assert!(noun.is_cell(), "Root should be a cell");
        assert!(noun.is_stack_allocated(), "Should be stack-allocated");

        // Install PMA arena
        let _guard = pma.install();

        // Count expected allocations:
        // - (DEPTH - 1) cells
        // - DEPTH indirect atoms, each with (word_count + 2) words (metadata + size + data)
        let cell_words = word_size_of::<CellMemory>();
        let mut expected_indirect_words = 0usize;
        for i in 1..=DEPTH {
            expected_indirect_words += word_count_for_index(i) + 2; // +2 for metadata and size
        }
        let expected_total = (cell_words * (DEPTH - 1)) + expected_indirect_words;

        // Evacuate
        unsafe { noun.copy_to_pma(&stack, &mut pma) };

        // Verify allocation size
        assert_eq!(
            pma.alloc_offset(),
            expected_total,
            "Should allocate {} words total ({} cells + {} indirect atom words)",
            expected_total,
            DEPTH - 1,
            expected_indirect_words
        );

        // Verify root is in offset form
        assert!(!noun.is_stack_allocated(), "Root should be in offset form");

        // Traverse and verify all values
        let mut current = noun;
        for expected_index in 1..DEPTH {
            assert!(current.is_cell(), "Should be cell at depth {}", expected_index);
            let cell = current.as_cell().expect("is cell");

            // Verify head is an indirect atom with correct data
            let head = cell.head();
            assert!(head.is_indirect(), "Head at depth {} should be indirect", expected_index);
            assert!(
                !head.is_stack_allocated(),
                "Head at depth {} should be in offset form",
                expected_index
            );

            let head_indirect = head.as_indirect().expect("indirect");
            let expected_word_count = word_count_for_index(expected_index);
            assert_eq!(
                head_indirect.size(),
                expected_word_count,
                "Indirect atom at depth {} should have {} words",
                expected_index,
                expected_word_count
            );

            // Verify data pattern
            let data_ptr = head_indirect.data_pointer();
            for word_idx in 0..expected_word_count {
                let expected_value = (expected_index as u64) << 32 | (word_idx as u64);
                let actual_value = unsafe { *data_ptr.add(word_idx) };
                assert_eq!(
                    actual_value, expected_value,
                    "Data mismatch at depth {}, word {}",
                    expected_index, word_idx
                );
            }

            current = cell.tail();
        }

        // Final element should be indirect atom for index DEPTH
        assert!(current.is_indirect(), "Leaf should be indirect atom");
        assert!(!current.is_stack_allocated(), "Leaf should be in offset form");

        let leaf_indirect = current.as_indirect().expect("indirect");
        let expected_leaf_words = word_count_for_index(DEPTH);
        assert_eq!(
            leaf_indirect.size(),
            expected_leaf_words,
            "Leaf indirect atom should have {} words",
            expected_leaf_words
        );

        // Verify leaf data pattern
        let leaf_data_ptr = leaf_indirect.data_pointer();
        for word_idx in 0..expected_leaf_words {
            let expected_value = (DEPTH as u64) << 32 | (word_idx as u64);
            let actual_value = unsafe { *leaf_data_ptr.add(word_idx) };
            assert_eq!(
                actual_value, expected_value,
                "Leaf data mismatch at word {}",
                word_idx
            );
        }

        // Verify assert_in_pma passes for entire structure
        noun.assert_in_pma(&pma);
    }

    /// Verifies NounAllocator::equals works through the Pma interface.
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_pma_noun_allocator_equals() {
        let mut stack = NockStack::new(1 << 10, 0);
        stack.install_arena(); // Required for Cell::new to work
        let mut pma = test_pma(1000);

        let mut noun1 = Cell::new(&mut stack, D(1), D(2)).as_noun();
        let mut noun2 = Cell::new(&mut stack, D(1), D(2)).as_noun();
        let mut noun3 = Cell::new(&mut stack, D(1), D(3)).as_noun();

        // Test through NounAllocator trait
        assert!(
            unsafe { pma.equals(&mut noun1 as *mut Noun, &mut noun2 as *mut Noun) },
            "NounAllocator::equals should return true for equal nouns"
        );
        assert!(
            !unsafe { pma.equals(&mut noun1 as *mut Noun, &mut noun3 as *mut Noun) },
            "NounAllocator::equals should return false for unequal nouns"
        );
    }

    /// Verifies that a HAMT can be evacuated to PMA and lookups still work.
    ///
    /// This test exercises:
    /// - Creating a HAMT with multiple entries (direct atoms as keys/values)
    /// - Evacuating the entire HAMT structure to PMA
    /// - Verifying all entries are still retrievable via lookup
    /// - Verifying all internal pointers are in offset form (not stack-allocated)
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_evacuate_hamt_round_trip() {
        let mut stack = NockStack::new(1 << 16, 0);
        let mut pma = test_pma(10000);
        let _guard = pma.install();

        // Create a HAMT with several entries
        let mut hamt: Hamt<Noun> = Hamt::new(&mut stack);

        // Insert 10 key-value pairs
        for i in 0u64..10 {
            let mut key = D(i);
            let value = D(i * 100);
            hamt = hamt.insert(&mut stack, &mut key, value);
        }

        // Verify lookups work before evacuation
        for i in 0u64..10 {
            let mut key = D(i);
            let result = hamt.lookup(&mut stack, &mut key);
            assert!(result.is_some(), "Lookup for key {} should succeed before evacuation", i);
            let value = result.unwrap();
            assert!(value.is_direct(), "Value should be direct atom");
            assert_eq!(
                value.as_direct().unwrap().data(),
                i * 100,
                "Value for key {} should be {}", i, i * 100
            );
        }

        // Evacuate the HAMT to PMA
        unsafe {
            hamt.copy_to_pma(&stack, &mut pma);
        }

        // Verify lookups still work after evacuation
        for i in 0u64..10 {
            let mut key = D(i);
            let result = hamt.lookup(&mut stack, &mut key);
            assert!(result.is_some(), "Lookup for key {} should succeed after evacuation", i);
            let value = result.unwrap();
            assert!(value.is_direct(), "Value should still be direct atom after evacuation");
            assert_eq!(
                value.as_direct().unwrap().data(),
                i * 100,
                "Value for key {} should still be {} after evacuation", i, i * 100
            );
        }

        // Verify internal structure is in PMA (offset form)
        // Iterate over the HAMT and check all nouns are not stack-allocated
        for entries in hamt.iter() {
            for (key, value) in entries {
                if !key.is_direct() {
                    assert!(
                        !key.is_stack_allocated(),
                        "HAMT key should be in offset form after evacuation"
                    );
                }
                if !value.is_direct() {
                    assert!(
                        !value.is_stack_allocated(),
                        "HAMT value should be in offset form after evacuation"
                    );
                }
            }
        }
    }

    /// Test that copy_to_pma correctly copies nouns to PMA and produces valid offset-form nouns.
    ///
    /// Note: copy_to_pma sets forwarding pointers in the source nouns, which corrupts
    /// them for normal use. This is by design for structural sharing. Therefore, we
    /// cannot compare source vs PMA copy directly. Instead, we verify the PMA copy
    /// contains the expected data.
    ///
    /// This test may look superfluous, but it helped debug test_evacuate_hamt_complex_nouns so
    /// that's why its in here.
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_copy_to_pma_preserves_data() {
        use crate::noun::{Cell, IndirectAtom};

        let mut stack = NockStack::new(1 << 16, 0);
        stack.install_arena();
        let mut pma = test_pma(10000);
        let _guard = pma.install();

        // Test with indirect atom
        let data: [u64; 2] = [0xDEADBEEF_CAFEBABE, 0x12345678_9ABCDEF0];
        let stack_indirect =
            unsafe { IndirectAtom::new_raw(&mut stack, 2, data.as_ptr()) }.as_noun();

        // Copy to PMA
        let mut pma_indirect = stack_indirect;
        unsafe { pma_indirect.copy_to_pma(&stack, &mut pma) };

        // Verify the PMA copy is in offset form
        assert!(!pma_indirect.is_stack_allocated(), "PMA copy should be in offset form");

        // Verify the PMA copy contains correct data
        let pma_ia = pma_indirect.as_indirect().unwrap();
        let pma_size = pma_ia.size_with_arena(pma.arena());
        assert_eq!(pma_size, 2, "PMA indirect atom should have size 2");

        let pma_bytes = pma_ia.as_ne_bytes_with_arena(pma.arena());
        assert_eq!(pma_bytes.len(), 16, "PMA indirect should have 16 bytes of data");

        // Verify actual data values
        let pma_slice = pma_ia.as_slice_with_arena(pma.arena());
        assert_eq!(pma_slice[0], 0xDEADBEEF_CAFEBABE, "First word should match");
        assert_eq!(pma_slice[1], 0x12345678_9ABCDEF0, "Second word should match");

        // Test with cell containing direct atoms
        let stack_cell = Cell::new(&mut stack, D(42), D(99)).as_noun();
        let mut pma_cell = stack_cell;
        unsafe { pma_cell.copy_to_pma(&stack, &mut pma) };

        assert!(!pma_cell.is_stack_allocated(), "PMA cell should be in offset form");
        let cell = pma_cell.as_cell().unwrap();
        assert_eq!(
            cell.head().as_direct().unwrap().data(),
            42,
            "Cell head should be 42"
        );
        assert_eq!(
            cell.tail().as_direct().unwrap().data(),
            99,
            "Cell tail should be 99"
        );

        // Test with nested structure
        let inner = Cell::new(&mut stack, D(1), D(2)).as_noun();
        let stack_nested = Cell::new(&mut stack, inner, D(3)).as_noun();
        let mut pma_nested = stack_nested;
        unsafe { pma_nested.copy_to_pma(&stack, &mut pma) };

        assert!(!pma_nested.is_stack_allocated(), "PMA nested should be in offset form");
        let outer = pma_nested.as_cell().unwrap();
        assert_eq!(
            outer.tail().as_direct().unwrap().data(),
            3,
            "Outer tail should be 3"
        );
        let inner_cell = outer.head().as_cell().unwrap();
        assert_eq!(
            inner_cell.head().as_direct().unwrap().data(),
            1,
            "Inner head should be 1"
        );
        assert_eq!(
            inner_cell.tail().as_direct().unwrap().data(),
            2,
            "Inner tail should be 2"
        );
    }

    /// Test HAMT evacuation with complex noun types: Cells and IndirectAtoms.
    ///
    /// This test exercises:
    /// - HAMT with indirect atoms as keys (large numbers)
    /// - HAMT with cells as values (nested structures)
    /// - Deep cell nesting to test recursive evacuation
    /// - Structural equality verification using a reference copy on a separate stack
    ///
    /// Note: copy_to_pma sets forwarding pointers in source nouns, corrupting them.
    /// To verify values, we create a second NockStack with fresh copies of the same
    /// data and compare those against the PMA copy using noun_equality.
    #[test]
    #[cfg_attr(miri, ignore = "memfd_create unsupported in Miri")]
    fn test_evacuate_hamt_complex_nouns() {
        use crate::ext::noun_equality;
        use crate::noun::{Cell, IndirectAtom};

        let mut stack = NockStack::new(1 << 16, 0);
        stack.install_arena();
        let mut pma = test_pma(100000);

        // Create a second stack with reference copies of keys/values for comparison
        // This stack won't be corrupted by forwarding pointers
        let mut ref_stack = NockStack::new(1 << 16, 0);

        // Install PMA arena - this must be the active arena when accessing PMA nouns
        let _guard = pma.install();

        let mut hamt: Hamt<Noun> = Hamt::new(&mut stack);

        // Store reference keys/values on the separate stack
        let mut ref_keys: Vec<Noun> = Vec::new();
        let mut ref_values: Vec<Noun> = Vec::new();

        // Insert entries with indirect atom keys and cell values
        for i in 0u64..5 {
            let key_data: [u64; 2] = [0xDEADBEEF_CAFEBABE + i, 0x12345678_9ABCDEF0 + i];

            // Create on main stack for HAMT
            let key_atom =
                unsafe { IndirectAtom::new_raw(&mut stack, 2, key_data.as_ptr()) }.as_noun();
            let inner = Cell::new(&mut stack, D(i + 100), D(i + 200)).as_noun();
            let value = Cell::new(&mut stack, D(i), inner).as_noun();

            // Create identical copies on reference stack
            let ref_key =
                unsafe { IndirectAtom::new_raw(&mut ref_stack, 2, key_data.as_ptr()) }.as_noun();
            let ref_inner = Cell::new(&mut ref_stack, D(i + 100), D(i + 200)).as_noun();
            let ref_value = Cell::new(&mut ref_stack, D(i), ref_inner).as_noun();
            ref_keys.push(ref_key);
            ref_values.push(ref_value);

            let mut key_copy = key_atom;
            hamt = hamt.insert(&mut stack, &mut key_copy, value);
        }

        // Insert entries with cell keys and indirect atom values
        for i in 5u64..10 {
            let val_data: [u64; 2] = [i * 1000, i * 2000];

            // Create on main stack for HAMT
            let key = Cell::new(&mut stack, D(i), D(i + 1)).as_noun();
            let value =
                unsafe { IndirectAtom::new_raw(&mut stack, 2, val_data.as_ptr()) }.as_noun();

            // Create identical copies on reference stack
            let ref_key = Cell::new(&mut ref_stack, D(i), D(i + 1)).as_noun();
            let ref_value =
                unsafe { IndirectAtom::new_raw(&mut ref_stack, 2, val_data.as_ptr()) }.as_noun();
            ref_keys.push(ref_key);
            ref_values.push(ref_value);

            let mut key_copy = key;
            hamt = hamt.insert(&mut stack, &mut key_copy, value);
        }

        // Insert entries with deeply nested cells
        for i in 10u64..12 {
            // Create on main stack for HAMT
            let ab = Cell::new(&mut stack, D(i), D(i + 1)).as_noun();
            let abc = Cell::new(&mut stack, ab, D(i + 2)).as_noun();
            let key = Cell::new(&mut stack, abc, D(i + 3)).as_noun();
            let zw = Cell::new(&mut stack, D(i + 10), D(i + 11)).as_noun();
            let yzw = Cell::new(&mut stack, D(i + 9), zw).as_noun();
            let value = Cell::new(&mut stack, D(i + 8), yzw).as_noun();

            // Create identical copies on reference stack
            let ref_ab = Cell::new(&mut ref_stack, D(i), D(i + 1)).as_noun();
            let ref_abc = Cell::new(&mut ref_stack, ref_ab, D(i + 2)).as_noun();
            let ref_key = Cell::new(&mut ref_stack, ref_abc, D(i + 3)).as_noun();
            let ref_zw = Cell::new(&mut ref_stack, D(i + 10), D(i + 11)).as_noun();
            let ref_yzw = Cell::new(&mut ref_stack, D(i + 9), ref_zw).as_noun();
            let ref_value = Cell::new(&mut ref_stack, D(i + 8), ref_yzw).as_noun();
            ref_keys.push(ref_key);
            ref_values.push(ref_value);

            let mut key_copy = key;
            hamt = hamt.insert(&mut stack, &mut key_copy, value);
        }

        // Count entries before evacuation
        let count_before: usize = hamt.iter().map(|entries| entries.len()).sum();
        assert_eq!(count_before, 12, "Should have 12 entries before evacuation");

        // Evacuate the HAMT to PMA
        unsafe {
            hamt.copy_to_pma(&stack, &mut pma);
        }

        // Count entries after evacuation
        let count_after: usize = hamt.iter().map(|entries| entries.len()).sum();
        assert_eq!(
            count_after, count_before,
            "Entry count should be preserved after evacuation"
        );

        // Re-install PMA arena (Cell::new on ref_stack may have changed thread-local arena)
        drop(_guard);
        let _guard = pma.install();

        // Verify all values match by comparing PMA nouns to reference stack nouns
        let mut found_count = 0;
        for entries in hamt.iter() {
            for (pma_key, pma_value) in entries {
                // Find matching reference key and verify value matches
                let mut found = false;
                for (idx, ref_key) in ref_keys.iter().enumerate() {
                    if noun_equality(pma_key, ref_key) {
                        assert!(
                            noun_equality(pma_value, &ref_values[idx]),
                            "Value for key {} should match reference after evacuation",
                            idx
                        );
                        found = true;
                        found_count += 1;
                        break;
                    }
                }
                assert!(found, "Every PMA key should match a reference key");
            }
        }
        assert_eq!(
            found_count,
            ref_keys.len(),
            "Should find all {} entries in HAMT after evacuation",
            ref_keys.len()
        );

        // Verify all nouns in the HAMT are in offset form
        for entries in hamt.iter() {
            for (key, value) in entries {
                verify_noun_not_stack_allocated(*key, "HAMT key");
                verify_noun_not_stack_allocated(*value, "HAMT value");
            }
        }

        // Verify the HAMT structure itself is in PMA
        hamt.assert_in_pma(&pma);
    }

    /// Helper to recursively verify a noun is not stack-allocated
    fn verify_noun_not_stack_allocated(noun: Noun, context: &str) {
        if noun.is_direct() {
            return;
        }

        assert!(
            !noun.is_stack_allocated(),
            "{} should be in offset form after evacuation",
            context
        );

        if let Ok(cell) = noun.as_cell() {
            verify_noun_not_stack_allocated(cell.head(), context);
            verify_noun_not_stack_allocated(cell.tail(), context);
        }
    }
}
