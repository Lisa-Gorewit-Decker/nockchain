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
