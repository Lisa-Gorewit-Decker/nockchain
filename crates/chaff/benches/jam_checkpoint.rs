use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use bincode::config::{self, Configuration};
use bincode::Decode;
use blake3::Hash;
use bytes::Bytes;
use chaff::Chaff;
use criterion::{black_box, criterion_group, criterion_main, BatchSize, Criterion};
use nockvm::ext::{JammedNoun, NounExt};
use nockvm::mem::NockStack;
use nockvm::noun::Noun;
use nockvm_macros::tas;

const FALLBACK_CHECKPOINT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/test-jams/0.chkjam");

const JAM_MAGIC_BYTES: u64 = tas!(b"CHKJAM");
const SNAPSHOT_VERSION_1: u32 = 1;
const SNAPSHOT_VERSION_2: u32 = 2;
const DEFAULT_STACK_WORDS: usize = 8 << 10 << 10;
const TOP_SLOTS: usize = 0;
const STACK_WORDS_ENV: &str = "NOCKAPP_BENCH_STACK_WORDS";

fn stack_words() -> usize {
    std::env::var(STACK_WORDS_ENV)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_STACK_WORDS)
}

fn fresh_stack() -> NockStack {
    NockStack::new(stack_words(), TOP_SLOTS)
}

trait Jammer {
    fn cue(stack: &mut NockStack, bytes: Bytes) -> Noun;
    fn jam(stack: &mut NockStack, noun: Noun) -> Bytes;
}

struct NockvmJammer;

impl Jammer for NockvmJammer {
    fn cue(stack: &mut NockStack, bytes: Bytes) -> Noun {
        Noun::cue_bytes(stack, &bytes).expect("cue should succeed for checkpoint")
    }

    fn jam(stack: &mut NockStack, noun: Noun) -> Bytes {
        noun.jam_self(stack).0
    }
}

struct ChaffJammer;

impl Jammer for ChaffJammer {
    fn cue(stack: &mut NockStack, bytes: Bytes) -> Noun {
        Chaff::cue_into(stack, bytes).expect("cue should succeed for checkpoint")
    }

    fn jam(stack: &mut NockStack, noun: Noun) -> Bytes {
        Chaff::jam(noun, &stack.noun_space())
    }
}

#[derive(Decode)]
struct CheckpointEnvelope {
    magic_bytes: u64,
    version: u32,
    payload: Vec<u8>,
}

#[derive(Decode)]
struct JammedCheckpointV1 {
    magic_bytes: u64,
    version: u32,
    #[bincode(with_serde)]
    _ker_hash: Hash,
    #[bincode(with_serde)]
    _checksum: Hash,
    _event_num: u64,
    jam: JammedNoun,
}

#[derive(Decode)]
struct JammedCheckpointV2 {
    #[bincode(with_serde)]
    _ker_hash: Hash,
    #[bincode(with_serde)]
    _checksum: Hash,
    _event_num: u64,
    _cold_jam: JammedNoun,
    state_jam: JammedNoun,
}

fn checkpoint_bytes() -> &'static [u8] {
    static SAMPLE: OnceLock<Vec<u8>> = OnceLock::new();
    SAMPLE
        .get_or_init(|| {
            let path = resolve_checkpoint_path();
            std::fs::read(&path).unwrap_or_else(|err| {
                panic!(
                    "failed to read checkpoint {path:?}: {err}. Set NOCKAPP_BENCH_CHECKPOINT to a .chkjam file or NOCKAPP_BENCH_CHECKPOINT_DIR to a directory containing one"
                )
            })
        })
        .as_slice()
}

/// Returns the jammed state bytes from the checkpoint, supporting both V1 and V2 formats
fn extract_jammed_state(bytes: &[u8]) -> Bytes {
    let config = config::standard();

    // Try to decode as envelope format (V2)
    if let Ok((envelope, _)) =
        bincode::decode_from_slice::<CheckpointEnvelope, Configuration>(bytes, config)
    {
        if envelope.magic_bytes == JAM_MAGIC_BYTES && envelope.version == SNAPSHOT_VERSION_2 {
            let (checkpoint, _) = bincode::decode_from_slice::<JammedCheckpointV2, Configuration>(
                &envelope.payload, config,
            )
            .expect("V2 checkpoint payload should decode");
            return checkpoint.state_jam.0;
        }
    }

    // Try to decode as V1 (non-envelope format)
    if let Ok((checkpoint, _)) =
        bincode::decode_from_slice::<JammedCheckpointV1, Configuration>(bytes, config)
    {
        if checkpoint.magic_bytes == JAM_MAGIC_BYTES && checkpoint.version == SNAPSHOT_VERSION_1 {
            return checkpoint.jam.0;
        }
    }

    panic!("Failed to decode checkpoint as either V1 or V2 format");
}

fn jammed_state_bytes() -> &'static Bytes {
    static JAMMED: OnceLock<Bytes> = OnceLock::new();
    JAMMED.get_or_init(|| extract_jammed_state(checkpoint_bytes()))
}

fn resolve_checkpoint_path() -> PathBuf {
    if let Ok(file_path) = std::env::var("NOCKAPP_BENCH_CHECKPOINT") {
        let path = PathBuf::from(&file_path);
        if path.is_file() {
            return path;
        }
        panic!(
            "NOCKAPP_BENCH_CHECKPOINT={file_path:?} does not point to a file. Provide a full path to a .chkjam file"
        );
    }

    if let Ok(dir) = std::env::var("NOCKAPP_BENCH_CHECKPOINT_DIR") {
        let dir_path = Path::new(&dir);
        if !dir_path.is_dir() {
            panic!(
                "NOCKAPP_BENCH_CHECKPOINT_DIR={dir:?} is not a directory. Provide a directory containing .chkjam files"
            );
        }
        let mut entries = std::fs::read_dir(dir_path)
            .unwrap_or_else(|err| panic!("failed to read directory {dir:?}: {err}"))
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.extension() == Some(OsStr::new("chkjam")))
            .collect::<Vec<_>>();
        entries.sort();
        if let Some(first) = entries.into_iter().next() {
            return first;
        }
        panic!(
            "NOCKAPP_BENCH_CHECKPOINT_DIR={dir:?} did not contain any files ending with .chkjam"
        );
    }

    let fallback = Path::new(FALLBACK_CHECKPOINT);
    if fallback.is_file() {
        return fallback.to_path_buf();
    }

    panic!(
        "Set NOCKAPP_BENCH_CHECKPOINT to the path of a .chkjam file, provide NOCKAPP_BENCH_CHECKPOINT_DIR, or ensure the fallback checkpoint exists at {fallback:?}"
    );
}

fn run_checkpoint_jam_bench<J>(c: &mut Criterion, name: &str)
where
    J: Jammer,
{
    let jammed_bytes = jammed_state_bytes().clone();
    c.bench_function(name, |b| {
        b.iter_batched(
            || {
                let mut stack = fresh_stack();
                let noun = J::cue(&mut stack, jammed_bytes.clone());
                (stack, noun)
            },
            |(mut stack, noun)| {
                let jammed = J::jam(&mut stack, noun);
                black_box(jammed);
            },
            BatchSize::SmallInput,
        );
    });
}

fn run_checkpoint_cue_bench<J>(c: &mut Criterion, name: &str)
where
    J: Jammer,
{
    let jammed_bytes = jammed_state_bytes().clone();
    c.bench_function(name, |b| {
        b.iter(|| {
            let mut stack = fresh_stack();
            let noun = J::cue(&mut stack, jammed_bytes.clone());
            black_box(noun);
        });
    });
}

fn jam_checkpoint_nockvm(c: &mut Criterion) {
    run_checkpoint_jam_bench::<NockvmJammer>(c, "jam_hoonc_state_nockvm");
}

fn jam_checkpoint_chaff(c: &mut Criterion) {
    run_checkpoint_jam_bench::<ChaffJammer>(c, "jam_hoonc_state_chaff");
}

fn cue_checkpoint_nockvm(c: &mut Criterion) {
    run_checkpoint_cue_bench::<NockvmJammer>(c, "cue_hoonc_state_nockvm");
}

fn cue_checkpoint_chaff(c: &mut Criterion) {
    run_checkpoint_cue_bench::<ChaffJammer>(c, "cue_hoonc_state_chaff");
}

criterion_group!(
    benches, jam_checkpoint_nockvm, jam_checkpoint_chaff, cue_checkpoint_nockvm,
    cue_checkpoint_chaff
);
criterion_main!(benches);
