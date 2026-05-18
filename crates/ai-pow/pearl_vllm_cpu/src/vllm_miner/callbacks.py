from dataclasses import dataclass

import torch
from miner_utils import get_logger
from pearl_gateway.comm.dataclasses import CommitmentHash, MiningJob, OpenedBlockInfo
from pearl_gemm import HostSignalStatus, extract_indices, get_host_signal_header

from .config import config
from .mining_state import (
    get_pinned_pool,
)

_LOGGER = get_logger("vllm.pearl_miner")


@dataclass
class StatusCheckCallback:
    """Note: all tensors are ptrs to GPU memory, except host_signal_header_pinned"""

    host_signal_header_pinned: torch.Tensor
    commitment_hash_A_tensor: torch.Tensor
    commitment_hash_B_tensor: torch.Tensor
    A: torch.Tensor
    B: torch.Tensor
    mining_job: MiningJob

    def __call__(self, handle_submit_block):
        header = get_host_signal_header(self.host_signal_header_pinned)

        if header.status == HostSignalStatus.kSignalTriggered:
            _LOGGER.info(f"Block found! {header=}")

            idxs = extract_indices(header)

            commitment_hash = CommitmentHash(
                noise_seed_A=self.commitment_hash_A_tensor.cpu().numpy().tobytes(),
                noise_seed_B=self.commitment_hash_B_tensor.cpu().numpy().tobytes(),
            )

            opened_block_info = OpenedBlockInfo(
                A_row_indices=idxs.A_row_indices,
                B_column_indices=idxs.B_column_indices,
                # .cpu() creates a copy and no need for clone()
                A=self.A.cpu().detach(),
                # GPU holds B transposed
                B_t=self.B.cpu().detach(),
                commitment_hash=commitment_hash,
                noise_rank=config.settings.noise_rank,
            )

            handle_submit_block(opened_block_info, self.mining_job)

        get_pinned_pool().release(self.host_signal_header_pinned)
        self.host_signal_header_pinned = None
        del self.commitment_hash_A_tensor
        del self.commitment_hash_B_tensor
        del self.A
        del self.B
        del header
