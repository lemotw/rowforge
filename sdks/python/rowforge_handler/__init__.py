"""rowforge handler SDK.

Public API: `run`, `run_batch`, `HandlerError`, `Context`.

Wire protocol implementation lives in `_protocol` and `_batch`.
"""

from ._batch import run_batch
from ._protocol import Context, HandlerError, run

__all__ = ["Context", "HandlerError", "run", "run_batch"]
__version__ = "0.1.0"
