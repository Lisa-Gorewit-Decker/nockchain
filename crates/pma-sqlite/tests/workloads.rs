use std::sync::Arc;
use std::time::{Duration, Instant};

use nockvm::ext::AtomExt;
use nockvm::mem::NockStack;
use nockvm::noun::{Atom, Cell, Noun};
use nockvm::pma::{Pma, PmaCopy};
use nockvm::serialization;
use pma_sqlite::{SqlitePma, SqlitePmaConfig};
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
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
    let jams = build_jams(spec, &mut rng);

    let temp_dir = TempDir::new().expect("temp dir");
    let sqlite_path = temp_dir.path().join("sqlite.db");
    let mut sqlite = SqlitePma::open({
        let mut config = SqlitePmaConfig::new(&sqlite_path);
        config.cache_capacity = spec.count / 2;
        config.stack_words_hint = stack_words / 4;
        config
    })
    .expect("sqlite open");

    let start = Instant::now();
    let mut ids = Vec::with_capacity(jams.len());
    for jam in &jams {
        let id = sqlite.insert_jam(jam).expect("sqlite insert");
        ids.push(id);
    }
    for id in ids.iter().step_by(2) {
        sqlite
            .with_cached(*id, |cached| {
                let root = cached.root();
                let _ = serialization::jam(cached.stack_mut(), root);
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
    let jams = build_jams(spec, &mut rng);

    let temp_dir = TempDir::new().expect("temp dir");
    let sqlite_path = temp_dir.path().join("sqlite.db");
    let pma_path = temp_dir.path().join("pma.dat");

    let sqlite_timings = run_sqlite_workload(&jams, &sqlite_path, spec);
    let pma_timings = run_pma_workload(&jams, &pma_path, stack_words);

    CompareTimings {
        sqlite: sqlite_timings,
        pma: pma_timings,
    }
}

fn run_sqlite_workload(
    jams: &[Vec<u8>],
    sqlite_path: &std::path::Path,
    spec: WorkloadSpec,
) -> WorkloadTimings {
    let mut sqlite = SqlitePma::open({
        let mut config = SqlitePmaConfig::new(sqlite_path);
        config.cache_capacity = spec.count / 2;
        config.stack_words_hint = estimate_stack_words(spec) / 4;
        config
    })
    .expect("sqlite open");

    let start = Instant::now();
    let mut ids = Vec::with_capacity(jams.len());
    for jam in jams {
        let id = sqlite.insert_jam(jam).expect("sqlite insert");
        ids.push(id);
    }
    let write = start.elapsed();

    let start = Instant::now();
    for id in ids {
        sqlite
            .with_cached(id, |cached| {
                let root = cached.root();
                let _ = serialization::jam(cached.stack_mut(), root);
            })
            .expect("sqlite get");
    }
    let read = start.elapsed();

    WorkloadTimings { write, read }
}

fn run_pma_workload(
    jams: &[Vec<u8>],
    pma_path: &std::path::Path,
    stack_words: usize,
) -> WorkloadTimings {
    let mut pma = Pma::new(stack_words.saturating_mul(2), pma_path.to_path_buf()).expect("pma new");

    let start = Instant::now();
    let mut roots = Vec::with_capacity(jams.len());
    let (mut stack, _) = NockStack::new_(stack_words, 0).expect("stack init");
    for jam in jams {
        let mut root = cue_jam(&mut stack, jam);
        unsafe {
            root.copy_to_pma(&stack, &mut pma);
        }
        roots.push(root);
        unsafe {
            stack.reset(0);
        }
    }
    let write = start.elapsed();

    let (mut jam_stack, _) = NockStack::new_(stack_words, 0).expect("stack init");
    jam_stack.install_pma_arena(Arc::clone(pma.arena()));

    let start = Instant::now();
    for root in roots {
        let _ = serialization::jam(&mut jam_stack, root);
        unsafe {
            jam_stack.reset(0);
        }
    }
    let read = start.elapsed();

    WorkloadTimings { write, read }
}

fn build_jams(spec: WorkloadSpec, rng: &mut StdRng) -> Vec<Vec<u8>> {
    let stack_words = estimate_stack_words(spec);
    let (mut stack, _) = NockStack::new_(stack_words, 0).expect("stack init");
    let mut jams = Vec::with_capacity(spec.count);
    for _ in 0..spec.count {
        let noun = build_tree(&mut stack, spec.depth, rng);
        let jammed = serialization::jam(&mut stack, noun);
        let space = stack.noun_space();
        jams.push(jammed.in_space(&space).to_ne_bytes());
        unsafe {
            stack.reset(0);
        }
    }
    jams
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

fn cue_jam(stack: &mut NockStack, jam: &[u8]) -> Noun {
    let atom = <Atom as AtomExt>::from_bytes(stack, jam);
    serialization::cue(stack, atom).expect("cue")
}

fn estimate_stack_words(spec: WorkloadSpec) -> usize {
    let nodes_per_tree = (1usize << (spec.depth + 1)).saturating_sub(1);
    let total_nodes = spec.count.saturating_mul(nodes_per_tree);
    let estimate = total_nodes.saturating_mul(4).saturating_add(1024 * 16);
    estimate.max(256 * 1024)
}
