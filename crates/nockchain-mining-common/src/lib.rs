//! Shared substrate for the external miner binaries.
//!
//! Both the ZK-PoW (dumb-puzzle) and AI-PoW miners run in their own
//! process and talk to a `nockchain` node over the node's private
//! `NockAppService` gRPC. This crate carries everything those
//! binaries share:
//!
//! - [`MiningWire`] — the kernel-side noun-ABI labels for poking
//!   the node (`SOURCE = "miner"`, `VERSION = 1`).
//! - [`MiningKeyConfig`] / [`MiningPkhConfig`] — CLI-parseable
//!   mining reward configuration.
//! - `MiningCandidate` (Stage 3) — decoded form of a `%mine`
//!   effect emitted by the node's kernel.
//! - `NodeClient` (Stage 3) — high-level wrapper over
//!   `PrivateNockAppGrpcClient`.

pub mod key_config;
pub mod wire;

pub use key_config::{MiningKeyConfig, MiningPkhConfig};
pub use wire::MiningWire;
