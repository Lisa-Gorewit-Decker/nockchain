class _D:
    def __init__(self,*a,**k): self.__dict__.update(k)
    def __getattr__(self,_n): return None
class CommitmentHash(_D): pass
class MiningJob(_D): pass
class OpenedBlockInfo(_D): pass
