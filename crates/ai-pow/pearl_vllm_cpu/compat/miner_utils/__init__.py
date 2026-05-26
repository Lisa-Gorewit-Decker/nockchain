"""CPU-fork compat stub for `miner_utils` (logging only)."""
import logging
def get_logger(name: str = "vllm.pearl_miner") -> logging.Logger:
    lg = logging.getLogger(name)
    if not lg.handlers:
        lg.addHandler(logging.NullHandler())
    return lg
