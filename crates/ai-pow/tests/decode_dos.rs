//! M1 (DoS audit): `MatmulProof::decode` and the internal
//! `decode_path_list` must NOT allocate memory disproportionate to
//! the input length on a crafted blob. The pre-fix
//! `Vec::with_capacity(untrusted_count)` pattern would let a
//! ~200-byte blob trigger a ~100 MiB allocation by declaring the
//! maximum policy-cap count and then truncating — a classic
//! deserialization bomb.
//!
//! This test installs a counting global allocator (scoped to this
//! test binary) and asserts the allocation amplification of a
//! crafted blob is bounded.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use ai_pow::proof::MatmulProof;

struct CountingAlloc;
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let cur =
                ALLOCATED.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
            PEAK.fetch_max(cur, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        ALLOCATED.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }
}

#[global_allocator]
static A: CountingAlloc = CountingAlloc;

/// Build a tiny malicious blob: a valid header + empty `found`
/// opening + a `spot` length-prefix declaring `declared_n` entries
/// — and no actual spot bytes after. The pre-fix `decode` would
/// `Vec::with_capacity(declared_n)` of `TileOpening`s up-front.
fn malicious_spot_blob(declared_n: u32) -> Vec<u8> {
    let mut b = Vec::new();
    // 6 × 32-byte commitment fields (zeros).
    b.extend_from_slice(&[0u8; 32 * 6]);
    // `found` TileOpening — all variable-length fields empty.
    b.extend_from_slice(&0u32.to_le_bytes()); // i
    b.extend_from_slice(&0u32.to_le_bytes()); // j
    b.extend_from_slice(&0u32.to_le_bytes()); // m_path: len 0
    b.extend_from_slice(&0u32.to_le_bytes()); // a_rows: len 0
    b.extend_from_slice(&0u32.to_le_bytes()); // b_cols: len 0
    b.extend_from_slice(&0u32.to_le_bytes()); // a_row_paths: count 0
    b.extend_from_slice(&0u32.to_le_bytes()); // b_col_paths: count 0
    b.extend_from_slice(&declared_n.to_le_bytes()); // spot count
    b
}

/// Same idea, but the bomb is the path-list count *inside* the
/// `found` opening (`decode_path_list`'s `with_capacity`).
fn malicious_path_list_blob(declared_n: u32) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(&[0u8; 32 * 6]);
    b.extend_from_slice(&0u32.to_le_bytes()); // i
    b.extend_from_slice(&0u32.to_le_bytes()); // j
    b.extend_from_slice(&0u32.to_le_bytes()); // m_path: len 0
    b.extend_from_slice(&0u32.to_le_bytes()); // a_rows: len 0
    b.extend_from_slice(&0u32.to_le_bytes()); // b_cols: len 0
    b.extend_from_slice(&declared_n.to_le_bytes()); // a_row_paths: count = bomb
    b
}

/// Measure the peak allocation `f()` triggers, in bytes.
fn measure_alloc_peak<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = ALLOCATED.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let r = f();
    let peak = PEAK.load(Ordering::Relaxed);
    (r, peak.saturating_sub(before))
}

/// Bomb threshold: with the bug, decode allocates ~100 MiB (spot)
/// or ~25 MiB (path-list) for a ~200-byte blob. The fix makes
/// allocation proportional to input — a few KiB at most for these
/// blobs. 4 MiB sits comfortably in the gap and tolerates parallel
/// test-harness noise.
const AMPLIFICATION_LIMIT: usize = 4 << 20;

#[test]
fn decode_does_not_amplify_attacker_declared_counts() {
    // (a) The `spot` count bomb.
    //
    // `MAX_SPOT = 1 << 20` (private const, mirrored here as the
    // worst-case policy-allowed value).
    let spot_blob = malicious_spot_blob(1 << 20);
    assert!(
        spot_blob.len() < 256,
        "blob is tiny ({} bytes), the bug amplifies it to ~100 MiB",
        spot_blob.len()
    );
    let (res, peak) = measure_alloc_peak(|| MatmulProof::decode(&spot_blob));
    assert!(res.is_err(), "malformed blob must be rejected");
    assert!(
        peak < AMPLIFICATION_LIMIT,
        "decode of a {}-byte blob declaring 2^20 spot entries allocated {} bytes — \
         must be proportional to input, not to the untrusted count \
         (pre-fix bomb was ~100 MiB)",
        spot_blob.len(),
        peak,
    );

    // (b) The `decode_path_list` count bomb (inside the `found`
    // opening; `MAX_STRIP_COUNT = 1 << 20`).
    let path_blob = malicious_path_list_blob(1 << 20);
    assert!(path_blob.len() < 256);
    let (res, peak) = measure_alloc_peak(|| MatmulProof::decode(&path_blob));
    assert!(res.is_err());
    assert!(
        peak < AMPLIFICATION_LIMIT,
        "decode_path_list of a {}-byte blob declaring 2^20 paths allocated {} bytes \
         (pre-fix bomb was ~25 MiB)",
        path_blob.len(),
        peak,
    );

    // (c) Sanity: an out-of-range count is rejected *before* any
    // allocation (the policy-cap check).
    let oversize = malicious_spot_blob(u32::MAX);
    let (res, peak) = measure_alloc_peak(|| MatmulProof::decode(&oversize));
    assert!(matches!(res, Err(_)));
    assert!(
        peak < AMPLIFICATION_LIMIT,
        "u32::MAX-count blob allocated {peak} bytes",
    );
}
