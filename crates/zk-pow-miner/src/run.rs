//! Stage 0 placeholder. Real run loop lands in Stage 3.

#[derive(Debug, thiserror::Error)]
pub enum MinerError {
    #[error("stage-0 stub; real run() lands in stage 3")]
    Stub,
}

pub struct MinerConfig;

pub async fn run(_cfg: MinerConfig, _shutdown: tokio_util::sync::CancellationToken) -> Result<(), MinerError> {
    Err(MinerError::Stub)
}
