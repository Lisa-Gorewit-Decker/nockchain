use std::time::{Duration, Instant};

use nockvm::mem::NockStack;
use nockvm::noun::{Atom, Cell, Noun, NounSpace};
use nockvm::pma::{Pma, PmaCopy};
use pma_sqlite::archive::NounNode;
use pma_sqlite::{ArchivedNoun, SqlitePma, SqlitePmaConfig};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use rkyv::Archive;
use tempfile::TempDir;

#[derive(Clone, Copy, Debug)]
struct WorkloadSpec {
    count: usize,
    depth: usize,
}

#[derive(Debug)]
struct WorkloadTimings {
    write: Duration,
    read: Duration,
}

#[test]
fn compare_sqlite_and_pma_timings_small() {
    let spec = WorkloadSpec {
        count: 64,
        depth: 6,
    };
    let timings = run_compare(spec);
    println!(
        "small: sqlite write {:?} read {:?}; pma write {:?} read {:?}",
        timings.sqlite.write, timings.sqlite.read, timings.pma.write, timings.pma.read
    );
}

#[test]
fn compare_sqlite_and_pma_timings_medium() {
    let spec = WorkloadSpec {
        count: 128,
        depth: 7,
    };
    let timings = run_compare(spec);
    println!(
        "medium: sqlite write {:?} read {:?}; pma write {:?} read {:?}",
        timings.sqlite.write, timings.sqlite.read, timings.pma.write, timings.pma.read
    );
}

#[test]
fn compare_sqlite_and_pma_timings_mixed() {
    let spec = WorkloadSpec {
        count: 64,
        depth: 6,
    };
    let mut rng = StdRng::seed_from_u64(1337);
    let stack_words = estimate_stack_words(spec);
    let seeds = build_seeds(spec, &mut rng);

    let temp_dir = TempDir::new().expect("temp dir");
    let sqlite_path = temp_dir.path().join("sqlite.db");
    let mut sqlite = SqlitePma::open({
        let mut config = SqlitePmaConfig::new(&sqlite_path);
        config.cache_capacity = spec.count / 2;
        config
    })
    .expect("sqlite open");
    sqlite.reserve_archive_nodes(estimate_nodes_per_tree(spec.depth));

    let start = Instant::now();
    let (mut stack, _) = NockStack::new_(stack_words, 0).expect("stack init");
    let mut ids = Vec::with_capacity(seeds.len());
    for seed in &seeds {
        let noun = build_tree_from_seed(&mut stack, spec.depth, *seed);
        let id = sqlite.insert_noun(&mut stack, noun).expect("sqlite insert");
        ids.push(id);
        unsafe {
            stack.reset(0);
        }
    }
    for id in ids.iter().step_by(2) {
        sqlite
            .with_cached(*id, |cached| {
                let _ = touch_archived_noun(cached.root());
            })
            .expect("sqlite get");
    }
    let sqlite_mixed = start.elapsed();

    println!("mixed sqlite workload time {:?}", sqlite_mixed);
}

struct CompareTimings {
    sqlite: WorkloadTimings,
    pma: WorkloadTimings,
}

fn run_compare(spec: WorkloadSpec) -> CompareTimings {
    let mut rng = StdRng::seed_from_u64(42);
    let stack_words = estimate_stack_words(spec);
    let seeds = build_seeds(spec, &mut rng);

    let temp_dir = TempDir::new().expect("temp dir");
    let sqlite_path = temp_dir.path().join("sqlite.db");
    let pma_path = temp_dir.path().join("pma.dat");

    let sqlite_timings = run_sqlite_workload(&seeds, &sqlite_path, spec);
    let pma_timings = run_pma_workload(&seeds, &pma_path, stack_words, spec.depth);

    CompareTimings {
        sqlite: sqlite_timings,
        pma: pma_timings,
    }
}

fn run_sqlite_workload(
    seeds: &[u64],
    sqlite_path: &std::path::Path,
    spec: WorkloadSpec,
) -> WorkloadTimings {
    let mut sqlite = SqlitePma::open({
        let mut config = SqlitePmaConfig::new(sqlite_path);
        config.cache_capacity = spec.count / 2;
        config
    })
    .expect("sqlite open");
    sqlite.reserve_archive_nodes(estimate_nodes_per_tree(spec.depth));

    let start = Instant::now();
    let (mut stack, _) = NockStack::new_(estimate_stack_words(spec), 0).expect("stack init");
    let mut ids = Vec::with_capacity(seeds.len());
    for seed in seeds {
        let noun = build_tree_from_seed(&mut stack, spec.depth, *seed);
        let id = sqlite.insert_noun(&mut stack, noun).expect("sqlite insert");
        ids.push(id);
        unsafe {
            stack.reset(0);
        }
    }
    let write = start.elapsed();

    let start = Instant::now();
    for id in ids {
        sqlite
            .with_cached(id, |cached| {
                let _ = touch_archived_noun(cached.root());
            })
            .expect("sqlite get");
    }
    let read = start.elapsed();

    WorkloadTimings { write, read }
}

fn run_pma_workload(
    seeds: &[u64],
    pma_path: &std::path::Path,
    stack_words: usize,
    depth: usize,
) -> WorkloadTimings {
    let mut pma = Pma::new(stack_words.saturating_mul(2), pma_path.to_path_buf()).expect("pma new");

    let start = Instant::now();
    let mut roots = Vec::with_capacity(seeds.len());
    let (mut stack, _) = NockStack::new_(stack_words, 0).expect("stack init");
    for seed in seeds {
        let mut root = build_tree_from_seed(&mut stack, depth, *seed);
        unsafe {
            root.copy_to_pma(&stack, &mut pma);
        }
        roots.push(root);
        unsafe {
            stack.reset(0);
        }
    }
    let write = start.elapsed();

    let space = NounSpace::pma_only(&pma);

    let start = Instant::now();
    for root in roots {
        let _ = touch_noun(&space, root);
    }
    let read = start.elapsed();

    WorkloadTimings { write, read }
}

fn build_tree(stack: &mut NockStack, depth: usize, rng: &mut StdRng) -> Noun {
    if depth == 0 {
        let value: u64 = rng.random();
        Atom::new(stack, value).as_noun()
    } else {
        let head = build_tree(stack, depth - 1, rng);
        let tail = build_tree(stack, depth - 1, rng);
        Cell::new(stack, head, tail).as_noun()
    }
}

fn build_tree_from_seed(stack: &mut NockStack, depth: usize, seed: u64) -> Noun {
    let mut rng = StdRng::seed_from_u64(seed);
    build_tree(stack, depth, &mut rng)
}

fn build_seeds(spec: WorkloadSpec, rng: &mut StdRng) -> Vec<u64> {
    (0..spec.count).map(|_| rng.random()).collect()
}

fn touch_noun(space: &NounSpace, root: Noun) -> u64 {
    let mut acc = 0u64;
    let mut pending = Vec::new();
    pending.push(root);
    while let Some(noun) = pending.pop() {
        let handle = space.handle(noun);
        if let Some(atom) = handle.atom() {
            let bytes = atom.as_ne_bytes();
            acc = acc.wrapping_add(bytes.len() as u64);
            if let Some(first) = bytes.first() {
                acc ^= *first as u64;
            }
        } else if let Some(cell) = handle.cell() {
            pending.push(cell.tail().noun());
            pending.push(cell.head().noun());
        }
    }
    acc
}

fn touch_archived_noun(root: &ArchivedNoun) -> u64 {
    type ArchivedNounNode = <NounNode as Archive>::Archived;

    let mut acc = 0u64;
    let nodes: &[ArchivedNounNode] = root.nodes.as_slice();
    let atom_bytes = root.atom_bytes.as_slice();
    let mut pending = Vec::new();
    pending.push(root.root.to_native() as usize);
    while let Some(idx) = pending.pop() {
        let node = &nodes[idx];
        match node {
            ArchivedNounNode::DirectAtom(value) => {
                let bytes = value.to_native().to_ne_bytes();
                acc = acc.wrapping_add(bytes.len() as u64);
                if let Some(first) = bytes.first() {
                    acc ^= *first as u64;
                }
            }
            ArchivedNounNode::IndirectAtom { offset, len } => {
                let start = offset.to_native() as usize;
                let end = start.saturating_add(len.to_native() as usize);
                let slice = &atom_bytes[start..end];
                acc = acc.wrapping_add(slice.len() as u64);
                if let Some(first) = slice.first() {
                    acc ^= *first as u64;
                }
            }
            ArchivedNounNode::Cell { head, tail } => {
                pending.push(tail.to_native() as usize);
                pending.push(head.to_native() as usize);
            }
        }
    }
    acc
}

fn estimate_stack_words(spec: WorkloadSpec) -> usize {
    let nodes_per_tree = estimate_nodes_per_tree(spec.depth);
    let total_nodes = spec.count.saturating_mul(nodes_per_tree);
    let estimate = total_nodes.saturating_mul(4).saturating_add(1024 * 16);
    estimate.max(256 * 1024)
}

fn estimate_nodes_per_tree(depth: usize) -> usize {
    let shift = depth.saturating_add(1);
    if shift >= usize::BITS as usize {
        return usize::MAX;
    }
    (1usize << shift).saturating_sub(1)
}
