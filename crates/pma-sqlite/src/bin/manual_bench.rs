use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use std::{env, fs};

use nockvm::mem::NockStack;
use nockvm::noun::{Atom, Cell, Noun, NounSpace};
use nockvm::pma::{Pma, PmaCopy};
use pma_sqlite::archive::{TAG_CELL, TAG_DIRECT_ATOM, TAG_INDIRECT_ATOM};
use pma_sqlite::{ArchivedNoun, SqlitePma, SqlitePmaConfig, SqlitePmaStats};

#[derive(Clone, Copy, Debug)]
struct BenchArgs {
    count: usize,
    depth: usize,
    read_rounds: usize,
    write_multiplier: usize,
    mixed_ops: usize,
    mixed_read_pct: u8,
    seed: u64,
    cache_capacity: Option<usize>,
    stack_words: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
struct BenchConfig {
    count: usize,
    depth: usize,
    read_rounds: usize,
    write_multiplier: usize,
    mixed_ops: usize,
    mixed_read_pct: u8,
    seed: u64,
    cache_capacity: usize,
    stack_words: usize,
}

#[derive(Clone, Copy, Debug)]
enum Op {
    Write(u64),
    Read(usize),
}

#[derive(Debug)]
struct WorkloadPlan {
    name: &'static str,
    ops: Vec<Op>,
    max_items: usize,
}

#[derive(Debug)]
struct BackendResult {
    duration: std::time::Duration,
    checksum: u64,
    stats: Option<SqlitePmaStats>,
}

#[derive(Debug)]
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = parse_args()?;
    let plans = build_workloads(&args);
    let config = resolve_config(args, &plans)?;
    let base_dir = prepare_base_dir()?;

    println!("manual bench base_dir: {}", base_dir.display());
    println!(
        "config: count={} depth={} read_rounds={} write_multiplier={} mixed_ops={} mixed_read_pct={} cache_capacity={} stack_words={} seed={}",
        config.count,
        config.depth,
        config.read_rounds,
        config.write_multiplier,
        config.mixed_ops,
        config.mixed_read_pct,
        config.cache_capacity,
        config.stack_words,
        config.seed
    );

    for plan in &plans {
        let (writes, reads) = count_ops(plan);
        println!(
            "workload: {} (writes={}, reads={}, max_items={})",
            plan.name, writes, reads, plan.max_items
        );

        let sqlite_path = base_dir.join(format!("{}_sqlite.db", plan.name));
        let pma_path = base_dir.join(format!("{}_pma.dat", plan.name));

        let sqlite = run_sqlite(plan, &config, &sqlite_path)?;
        let pma = run_pma(plan, &config, &pma_path)?;

        println!("  sqlite: {:?}", sqlite.duration);
        if let Some(stats) = sqlite.stats {
            println!(
                "  sqlite stats: hits={} misses={} inserts={}",
                stats.cache_hits, stats.cache_misses, stats.inserts
            );
        }
        println!("  pma: {:?}", pma.duration);
        println!(
            "  checksums: sqlite={} pma={}",
            sqlite.checksum, pma.checksum
        );
    }

    Ok(())
}

fn parse_args() -> Result<BenchArgs, Box<dyn std::error::Error>> {
    let mut args = BenchArgs {
        count: 512,
        depth: 6,
        read_rounds: 5,
        write_multiplier: 4,
        mixed_ops: 0,
        mixed_read_pct: 50,
        seed: 42,
        cache_capacity: None,
        stack_words: None,
    };

    let mut iter = env::args().skip(1);
    while let Some(arg) = iter.next() {
        if arg == "--help" || arg == "-h" {
            print_help();
            return Ok(args);
        }

        let (key, value) = if let Some((k, v)) = arg.split_once('=') {
            (k.to_string(), v.to_string())
        } else if arg.starts_with("--") {
            let value = iter
                .next()
                .ok_or_else(|| format!("missing value for {}", arg))?;
            (arg, value)
        } else {
            return Err(format!("unexpected argument: {}", arg).into());
        };

        match key.as_str() {
            "--count" => args.count = parse_usize(&value, "count")?,
            "--depth" => args.depth = parse_usize(&value, "depth")?,
            "--read-rounds" => args.read_rounds = parse_usize(&value, "read-rounds")?,
            "--write-multiplier" => {
                args.write_multiplier = parse_usize(&value, "write-multiplier")?
            }
            "--mixed-ops" => args.mixed_ops = parse_usize(&value, "mixed-ops")?,
            "--mixed-read-pct" => args.mixed_read_pct = parse_u8(&value, "mixed-read-pct")?,
            "--seed" => args.seed = parse_u64(&value, "seed")?,
            "--cache-capacity" => {
                args.cache_capacity = Some(parse_usize(&value, "cache-capacity")?)
            }
            "--stack-words" => args.stack_words = Some(parse_usize(&value, "stack-words")?),
            _ => return Err(format!("unknown argument: {}", key).into()),
        }
    }

    if args.mixed_read_pct > 100 {
        return Err("mixed-read-pct must be between 0 and 100".into());
    }

    Ok(args)
}

fn resolve_config(
    args: BenchArgs,
    plans: &[WorkloadPlan],
) -> Result<BenchConfig, Box<dyn std::error::Error>> {
    if args.count == 0 {
        return Err("count must be greater than 0".into());
    }
    if args.depth == 0 {
        return Err("depth must be greater than 0".into());
    }
    if args.read_rounds == 0 {
        return Err("read-rounds must be greater than 0".into());
    }
    if args.write_multiplier == 0 {
        return Err("write-multiplier must be greater than 0".into());
    }

    let max_items = plans
        .iter()
        .map(|plan| plan.max_items)
        .max()
        .unwrap_or(args.count);
    let stack_words = args
        .stack_words
        .unwrap_or_else(|| estimate_stack_words(max_items, args.depth));
    let cache_capacity = args
        .cache_capacity
        .unwrap_or_else(|| (args.count / 2).max(1));
    let mixed_ops = if args.mixed_ops == 0 {
        args.count.saturating_mul(4)
    } else {
        args.mixed_ops
    };

    Ok(BenchConfig {
        count: args.count,
        depth: args.depth,
        read_rounds: args.read_rounds,
        write_multiplier: args.write_multiplier,
        mixed_ops,
        mixed_read_pct: args.mixed_read_pct,
        seed: args.seed,
        cache_capacity,
        stack_words,
    })
}

fn build_workloads(args: &BenchArgs) -> Vec<WorkloadPlan> {
    vec![
        build_read_heavy(args),
        build_read_hot(args),
        build_write_heavy(args),
        build_mixed(args),
    ]
}

fn build_read_heavy(args: &BenchArgs) -> WorkloadPlan {
    let mut rng = SplitMix64::new(args.seed ^ 0xA5A5_A5A5_5A5A_5A5A);
    let mut ops = Vec::new();
    for _ in 0..args.count {
        ops.push(Op::Write(rng.next_u64()));
    }
    for _ in 0..args.read_rounds {
        for idx in 0..args.count {
            ops.push(Op::Read(idx));
        }
    }

    WorkloadPlan {
        name: "read-heavy",
        ops,
        max_items: args.count,
    }
}

fn build_read_hot(args: &BenchArgs) -> WorkloadPlan {
    let mut rng = SplitMix64::new(args.seed ^ 0xDEAD_BEEF_FEED_FACE);
    let mut ops = Vec::new();
    for _ in 0..args.count {
        ops.push(Op::Write(rng.next_u64()));
    }

    for idx in 0..args.count {
        ops.push(Op::Read(idx));
    }

    let cache_capacity = args
        .cache_capacity
        .unwrap_or_else(|| (args.count / 2).max(1))
        .max(1);
    let hot_items = cache_capacity.min(args.count).max(1);
    let start = args.count.saturating_sub(hot_items);

    for _ in 0..args.read_rounds {
        for idx in start..args.count {
            ops.push(Op::Read(idx));
        }
    }

    WorkloadPlan {
        name: "read-hot",
        ops,
        max_items: args.count,
    }
}

fn build_write_heavy(args: &BenchArgs) -> WorkloadPlan {
    let mut rng = SplitMix64::new(args.seed ^ 0x5A5A_5A5A_A5A5_A5A5);
    let write_count = args.count.saturating_mul(args.write_multiplier);
    let mut ops = Vec::new();
    for _ in 0..write_count {
        ops.push(Op::Write(rng.next_u64()));
    }
    let read_count = args.count.min(write_count).max(1);
    let start = write_count.saturating_sub(read_count);
    for idx in start..write_count {
        ops.push(Op::Read(idx));
    }

    WorkloadPlan {
        name: "write-heavy",
        ops,
        max_items: write_count,
    }
}

fn build_mixed(args: &BenchArgs) -> WorkloadPlan {
    let mixed_ops = if args.mixed_ops == 0 {
        args.count.saturating_mul(4)
    } else {
        args.mixed_ops
    };
    let mut rng = SplitMix64::new(args.seed ^ 0xC3C3_C3C3_3C3C_3C3C);
    let mut ops = Vec::with_capacity(mixed_ops);
    let mut items = 0usize;
    let mut max_items = 0usize;

    for _ in 0..mixed_ops {
        let pick = (rng.next_u64() % 100) as u8;
        if pick < args.mixed_read_pct && items > 0 {
            let idx = (rng.next_u64() as usize) % items;
            ops.push(Op::Read(idx));
        } else {
            ops.push(Op::Write(rng.next_u64()));
            items = items.saturating_add(1);
            max_items = max_items.max(items);
        }
    }

    WorkloadPlan {
        name: "mixed",
        ops,
        max_items,
    }
}

fn run_sqlite(
    plan: &WorkloadPlan,
    bench: &BenchConfig,
    path: &Path,
) -> Result<BackendResult, Box<dyn std::error::Error>> {
    let mut sqlite = SqlitePma::open({
        let mut config = SqlitePmaConfig::new(path);
        config.cache_capacity = bench.cache_capacity.max(1);
        config
    })?;
    sqlite.reserve_archive_nodes(estimate_nodes_per_tree(bench.depth));

    let (mut stack, _) = NockStack::new_(bench.stack_words, 0)?;
    let mut ids: Vec<i64> = Vec::new();
    let mut checksum = 0u64;

    let start = Instant::now();
    sqlite.begin_transaction()?;
    let result = (|| {
        for op in &plan.ops {
            match *op {
                Op::Write(seed) => {
                    let noun = build_tree_from_seed(&mut stack, bench.depth, seed);
                    let id = sqlite.insert_noun(&mut stack, noun)?;
                    ids.push(id);
                    unsafe {
                        stack.reset(0);
                    }
                }
                Op::Read(index) => {
                    let id = *ids
                        .get(index)
                        .ok_or_else(|| format!("read index {} out of range", index))?;
                    sqlite.with_cached(id, |cached| {
                        let value = touch_archived_noun(cached.root());
                        checksum = checksum.wrapping_add(value);
                    })?;
                }
            }
        }
        let duration = start.elapsed();
        let stats = sqlite.stats();
        let checksum = black_box(checksum);

        Ok(BackendResult {
            duration,
            checksum,
            stats: Some(stats),
        })
    })();

    match result {
        Ok(result) => {
            sqlite.commit_transaction()?;
            Ok(result)
        }
        Err(err) => {
            if let Err(rollback_err) = sqlite.rollback_transaction() {
                return Err(format!("{}; rollback failed: {}", err, rollback_err).into());
            }
            Err(err)
        }
    }
}

fn run_pma(
    plan: &WorkloadPlan,
    bench: &BenchConfig,
    path: &Path,
) -> Result<BackendResult, Box<dyn std::error::Error>> {
    let pma_words = bench.stack_words.saturating_mul(2);
    let mut pma = Pma::new(pma_words, path.to_path_buf())
        .map_err(|err| format!("pma init failed: {}", err))?;
    let (mut stack, _) = NockStack::new_(bench.stack_words, 0)
        .map_err(|err| format!("stack init failed: {}", err))?;
    let space = NounSpace::pma_only(&pma);

    let mut roots: Vec<Noun> = Vec::new();
    let mut checksum = 0u64;

    let start = Instant::now();
    for op in &plan.ops {
        match *op {
            Op::Write(seed) => {
                let mut noun = build_tree_from_seed(&mut stack, bench.depth, seed);
                unsafe {
                    noun.copy_to_pma(&stack, &mut pma);
                }
                roots.push(noun);
                unsafe {
                    stack.reset(0);
                }
            }
            Op::Read(index) => {
                let root = *roots
                    .get(index)
                    .ok_or_else(|| format!("read index {} out of range", index))?;
                let value = touch_noun(&space, root);
                checksum = checksum.wrapping_add(value);
            }
        }
    }
    let duration = start.elapsed();
    let checksum = black_box(checksum);

    Ok(BackendResult {
        duration,
        checksum,
        stats: None,
    })
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
    let mut acc = 0u64;
    let tags = root.tags.as_slice();
    let direct_atoms = root.direct_atoms.as_slice();
    let indirect_offsets = root.indirect_offsets.as_slice();
    let indirect_lens = root.indirect_lens.as_slice();
    let cell_heads = root.cell_heads.as_slice();
    let cell_tails = root.cell_tails.as_slice();
    let atom_bytes = root.atom_bytes.as_slice();
    let mut pending = Vec::new();
    pending.push(root.root.to_native() as usize);
    while let Some(idx) = pending.pop() {
        match tags[idx] {
            TAG_DIRECT_ATOM => {
                let bytes = direct_atoms[idx].to_native().to_ne_bytes();
                acc = acc.wrapping_add(bytes.len() as u64);
                if let Some(first) = bytes.first() {
                    acc ^= *first as u64;
                }
            }
            TAG_INDIRECT_ATOM => {
                let start = indirect_offsets[idx].to_native() as usize;
                let end = start.saturating_add(indirect_lens[idx].to_native() as usize);
                let slice = &atom_bytes[start..end];
                acc = acc.wrapping_add(slice.len() as u64);
                if let Some(first) = slice.first() {
                    acc ^= *first as u64;
                }
            }
            TAG_CELL => {
                pending.push(cell_tails[idx].to_native() as usize);
                pending.push(cell_heads[idx].to_native() as usize);
            }
            _ => {}
        }
    }
    acc
}

fn build_tree_from_seed(stack: &mut NockStack, depth: usize, seed: u64) -> Noun {
    let mut rng = SplitMix64::new(seed);
    build_tree(stack, depth, &mut rng)
}

fn build_tree(stack: &mut NockStack, depth: usize, rng: &mut SplitMix64) -> Noun {
    if depth == 0 {
        let value = rng.next_u64();
        Atom::new(stack, value).as_noun()
    } else {
        let head = build_tree(stack, depth - 1, rng);
        let tail = build_tree(stack, depth - 1, rng);
        Cell::new(stack, head, tail).as_noun()
    }
}

fn estimate_stack_words(count: usize, depth: usize) -> usize {
    let nodes_per_tree = estimate_nodes_per_tree(depth);
    let total_nodes = count.saturating_mul(nodes_per_tree);
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

fn count_ops(plan: &WorkloadPlan) -> (usize, usize) {
    let mut writes = 0usize;
    let mut reads = 0usize;
    for op in &plan.ops {
        match op {
            Op::Write(_) => writes += 1,
            Op::Read(_) => reads += 1,
        }
    }
    (writes, reads)
}

fn prepare_base_dir() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let dir = env::temp_dir().join(format!(
        "pma_sqlite_bench_{}_{}",
        std::process::id(),
        now.as_millis()
    ));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn parse_usize(value: &str, name: &str) -> Result<usize, Box<dyn std::error::Error>> {
    value
        .parse::<usize>()
        .map_err(|err| format!("invalid {}: {}", name, err).into())
}

fn parse_u64(value: &str, name: &str) -> Result<u64, Box<dyn std::error::Error>> {
    value
        .parse::<u64>()
        .map_err(|err| format!("invalid {}: {}", name, err).into())
}

fn parse_u8(value: &str, name: &str) -> Result<u8, Box<dyn std::error::Error>> {
    value
        .parse::<u8>()
        .map_err(|err| format!("invalid {}: {}", name, err).into())
}

fn print_help() {
    println!(
        "manual_bench options:\n  \
  --count <n>\n  \
  --depth <n>\n  \
  --read-rounds <n>\n  \
  --write-multiplier <n>\n  \
  --mixed-ops <n>\n  \
  --mixed-read-pct <0-100>\n  \
  --seed <n>\n  \
  --cache-capacity <n>\n  \
  --stack-words <n>\n"
    );
}
