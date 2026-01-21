use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;
use nockapp::kernel::boot::NockStackSize;
use nockapp::kernel::form::write_pma_metadata;
use nockapp::noun::slab::NockJammer;
use nockapp::save::{SaveableCheckpoint, Saver};
use nockapp::utils::{
    NOCK_STACK_SIZE, NOCK_STACK_SIZE_HUGE, NOCK_STACK_SIZE_LARGE, NOCK_STACK_SIZE_MEDIUM,
    NOCK_STACK_SIZE_SMALL, NOCK_STACK_SIZE_TINY,
};
use nockvm::jets::cold::{Cold, Nounable};
use nockvm::mem::NockStack;
use nockvm::pma::{Pma, PmaCopy};

#[derive(Debug)]
struct CliError(String);

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CliError {}

type Result<T> = std::result::Result<T, CliError>;

#[derive(Debug, Parser)]
#[command(about = "Convert the latest checkpoint into a PMA persistence file.")]
struct Cli {
    #[arg(long, value_name = "DIR")]
    data_dir: PathBuf,
    #[arg(long, value_enum, default_value_t = NockStackSize::Normal)]
    stack_size: NockStackSize,
    #[arg(long)]
    pma_words: Option<usize>,
    #[arg(long)]
    force: bool,
}

fn stack_words_for_size(size: &NockStackSize) -> usize {
    match size {
        NockStackSize::Tiny => NOCK_STACK_SIZE_TINY,
        NockStackSize::Small => NOCK_STACK_SIZE_SMALL,
        NockStackSize::Normal => NOCK_STACK_SIZE,
        NockStackSize::Medium => NOCK_STACK_SIZE_MEDIUM,
        NockStackSize::Large => NOCK_STACK_SIZE_LARGE,
        NockStackSize::Huge => NOCK_STACK_SIZE_HUGE,
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    nockvm::check_endian();

    let cli = Cli::parse();
    let checkpoints_dir = cli.data_dir.join("checkpoints");
    let pma_dir = cli.data_dir.join("pma");
    std::fs::create_dir_all(&pma_dir)
        .map_err(|err| CliError(format!("create pma directory: {err}")))?;

    let pma_path = pma_dir.join("pma.mmap");
    let meta_path = pma_path.with_extension("meta");

    if (pma_path.exists() || meta_path.exists()) && !cli.force {
        return Err(CliError(format!(
            "PMA output already exists at {} (use --force to overwrite)",
            pma_path.display()
        )));
    }
    if cli.force {
        let _ = std::fs::remove_file(&pma_path);
        let _ = std::fs::remove_file(&meta_path);
    }

    let (_saver, checkpoint_opt) =
        Saver::<NockJammer>::try_load::<SaveableCheckpoint>(&checkpoints_dir, None)
            .await
            .map_err(|err| CliError(format!("load checkpoint: {err}")))?;
    let checkpoint = match checkpoint_opt {
        Some(checkpoint) => checkpoint,
        None => {
            return Err(CliError(format!(
                "no checkpoints found in {}",
                checkpoints_dir.display()
            )));
        }
    };

    let stack_words = stack_words_for_size(&cli.stack_size);
    let pma_words = cli.pma_words.unwrap_or(stack_words);
    let mut pma = Pma::new(pma_words, pma_path.clone())
        .map_err(|err| CliError(format!("create PMA: {err}")))?;
    let mut stack = NockStack::new(stack_words, 0);
    stack.install_pma_arena(Arc::clone(pma.arena()));

    let mut kernel_state = checkpoint.state.copy_to_stack(&mut stack);
    let cold_noun = checkpoint.cold.copy_to_stack(&mut stack);
    let space = stack.noun_space();
    let cold_vecs = Cold::from_noun(&mut stack, &cold_noun, &space)
        .map_err(|err| CliError(format!("decode cold state: {err}")))?;
    let mut cold = Cold::from_vecs(&mut stack, cold_vecs.0, cold_vecs.1, cold_vecs.2);

    unsafe {
        kernel_state.copy_to_pma(&stack, &mut pma);
        cold.copy_to_pma(&stack, &mut pma);
    }

    let kernel_state_raw = unsafe { kernel_state.as_raw() };
    let cold_offset = cold
        .pma_offset(&pma)
        .ok_or_else(|| CliError("cold state not in PMA after copy".to_string()))?;
    let pma_base = pma.arena().base_ptr() as u64;

    pma.persist_metadata();
    pma.sync_all()
        .map_err(|err| CliError(format!("sync PMA: {err}")))?;
    write_pma_metadata(
        &meta_path, checkpoint.ker_hash, checkpoint.event_num, kernel_state_raw, cold_offset,
        pma_base,
    )
    .map_err(|err| CliError(format!("write PMA metadata: {err}")))?;

    println!(
        "Wrote PMA at {} (event_num={}, pma_words={})",
        pma_path.display(),
        checkpoint.event_num,
        pma_words
    );
    Ok(())
}
