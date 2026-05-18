"""CPU inference-fork mining state — **inert**.

Upstream `mining_state` spins an `AsyncLoopManager` (gateway
socket, block-submission threads) + a CUDA host-pinned pool.
vLLM v1 runs the model in an ``EngineCore`` process, so
``register.py``'s ``_is_vllm_worker()`` branch calls
``init_async_manager`` / ``init_pinned_pool`` during engine
init — none of which is relevant to (or runnable on) the
CPU **inference / activation-extraction** fork.

This rewrite makes that state inert: a no-op manager whose
``_conf`` still serves the few settings the *inference* path
reads (``quantization_fast_math``, ``skip_block_submission``,
``no_vllm_plugin``, ``pinned_pool_size``), and a no-op pinned
pool. No gateway, no threads, no CUDA. The faithful mining /
noisy-GEMM / commitment path lives in Rust ``ai_pow`` /
the B1-audited reference — out of scope for this fork
(``no_mining=True``). See PEARL_VLLM_CPU_FORK_DESIGN.md.
"""

from __future__ import annotations

from miner_base.settings import MinerSettings
from miner_utils import get_logger

_LOGGER = get_logger("vllm.pearl_miner")


class _InertAsyncManager:
    """No-op stand-in for ``AsyncLoopManager`` on the CPU fork.

    ``_conf`` is the compat ``MinerSettings`` (carries
    ``quantization_fast_math``/``skip_block_submission``/
    ``no_vllm_plugin``/``pinned_pool_size`` read by the
    quant/register paths). All lifecycle methods are no-ops.
    """

    def __init__(self) -> None:
        self._conf = MinerSettings()
        self._pool = None

    def start(self) -> None:  # no-op
        pass

    def stop(self) -> None:  # no-op
        pass

    def wait_until_done_submitting_blocks(self) -> None:  # no-op
        pass

    def schedule_status_check(self, *_a, **_k) -> None:  # no-op
        pass


_async_manager: _InertAsyncManager | None = None
_pinned_pool: object | None = None


def get_async_manager() -> _InertAsyncManager:
    # Lazily inert: never "not initialized" on the CPU fork.
    global _async_manager
    if _async_manager is None:
        _async_manager = _InertAsyncManager()
    return _async_manager


def init_async_manager(miner_settings: MinerSettings | None = None) -> None:
    global _async_manager
    if _async_manager is None:
        _async_manager = _InertAsyncManager()
        _LOGGER.info("CPU fork: inert async manager (no mining gateway/threads)")


def get_pinned_pool() -> object | None:
    return _pinned_pool


def init_pinned_pool(pool_size: int = 0) -> None:
    # No CUDA host-pinned pool on the CPU fork.
    _LOGGER.info("CPU fork: pinned pool disabled (mining-only)")


def delete_state() -> None:
    global _async_manager, _pinned_pool
    _async_manager = None
    _pinned_pool = None
