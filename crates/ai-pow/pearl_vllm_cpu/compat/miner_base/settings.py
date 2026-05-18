"""Behaviour-critical settings for the CPU inference fork.

`no_vllm_plugin=False` ⇒ the `"pearl"` quant config IS registered.
`no_mining=True`       ⇒ PearlKernel routes through the vanilla
                          (dequant-matmul) path — never the
                          un-ported noisy/commitment kernel.
Any unlisted attribute returns a benign default (False) so we
don't have to enumerate Pearl's mining internals we never run.
"""
class MinerSettings:
    no_vllm_plugin = False
    no_mining = True
    skip_block_submission = True
    enable_async_cuda_event_processing = False
    quantization_fast_math = False
    pinned_pool_size = 0
    tile_size_m = None
    tile_size_n = None
    tile_size_k = None
    def __init__(self, *a, **k): pass
    def __getattr__(self, _name): return False
