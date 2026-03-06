#![feature(slice_pattern)]

//! # Crown
//!
//! The Crown library provides a set of modules and utilities for working with
//! the Sword runtime. It includes functionality for handling jammed nouns, kernels (as jammed nouns),
//! and various types and utilities that make nockvm easier to use.
//!
//! ## Modules
//!
//! - `kernel`: Sword runtime interface.
//! - `noun`: Extensions and utilities for working with Urbit nouns.
//! - `utils`: Errors, misc functions and extensions.
//!
pub mod drivers;
pub(crate) mod event_log;
pub mod kernel;
pub mod nockapp;
pub mod noun;
pub mod observability;
pub mod utils;

use std::path::PathBuf;

pub use bytes::*;
pub use drivers::*;
pub use nockapp::*;
pub use nockvm::noun::Noun;
pub use noun::{AtomExt, IndirectAtomExt, JammedNoun, NounExt};
pub use utils::bytes::{ToBytes, ToBytesExt};
pub use utils::error::{CrownError, Result};

/// Returns the default directory where kernel data is stored.
///
/// # Arguments
///
/// * `dir` - A string slice that holds the kernel identifier.
///
/// # Example
///
/// ```
///
/// use std::path::PathBuf;
/// use nockapp::default_data_dir;
/// let dir = default_data_dir("nockapp");
/// assert_eq!(dir, PathBuf::from("./.data.nockapp"));
/// ```
pub fn default_data_dir(dir_name: &str) -> PathBuf {
    PathBuf::from(format!("./.data.{}", dir_name))
}

pub fn system_data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("NOCKAPP_HOME") {
        if !dir.trim().is_empty() {
            let path = PathBuf::from(&dir);
            if path.is_absolute() {
                return path;
            }
            if let Ok(current) = std::env::current_dir() {
                return current.join(path);
            }
            return PathBuf::from(dir);
        }
    }

    let home_dir = dirs::home_dir().expect("Failed to get home directory");
    home_dir.join(".nockapp")
}

/// Default size for the Nock stack (1 GB)
pub const DEFAULT_NOCK_STACK_SIZE: usize = 1 << 27;

#[cfg(test)]
pub mod test_support {
    use nockvm::mem::NockStack;

    pub struct TestArena {
        stack: NockStack,
    }

    impl TestArena {
        pub fn with_words(words: usize) -> Self {
            let stack = NockStack::new(words, 0);
            Self { stack }
        }
    }

    impl Default for TestArena {
        fn default() -> Self {
            // A modest stack is enough because tests mostly need TLS to be populated.
            Self::with_words(1 << 16)
        }
    }

    impl std::ops::Deref for TestArena {
        type Target = NockStack;

        fn deref(&self) -> &Self::Target {
            &self.stack
        }
    }

    impl std::ops::DerefMut for TestArena {
        fn deref_mut(&mut self) -> &mut Self::Target {
            &mut self.stack
        }
    }
}
