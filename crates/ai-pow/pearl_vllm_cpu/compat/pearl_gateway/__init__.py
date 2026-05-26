def __getattr__(_n):
    class _S:
        def __init__(self,*a,**k): pass
        def __getattr__(self,_x): return None
    return _S
