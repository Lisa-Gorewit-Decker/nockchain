"""CPU-fork compat stub (mining-only; never executed on the
inference/activation path — `no_mining=True`)."""
class _Stub:
    def __init__(self, *a, **k): pass
    def __getattr__(self, _n):
        def _f(*_a, **_k):
            raise NotImplementedError(
                "miner_base mining stub hit on the CPU inference fork "
                "(should be unreachable with no_mining=True)")
        return _f
def __getattr__(_name): return _Stub
