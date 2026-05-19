"""Internal: JSON-Lines stdio protocol loop.

Maps spec §6.3 envelopes to a callback-style API. See package README for usage.
"""

from __future__ import annotations

import json
import sys
import traceback
from dataclasses import dataclass, field
from typing import Any, Callable, Mapping


class HandlerError(Exception):
    """Raise from your handle function to emit a protocol-level error envelope.

    Args:
        code: short uppercase error code, e.g. "INVALID_FORMAT".
        message: human-readable detail.
        data: optional dict of handler-supplied domain columns. Keys
            declared under ``manifest.schema.failed_output`` materialize as
            their own columns in failed.csv. ``None`` (the default) is
            byte-equivalent to omitting the field on the wire.
    """

    def __init__(self, code: str, message: str, data: Mapping[str, Any] | None = None):
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message
        self.data = data


@dataclass(frozen=True)
class Context:
    """Per-row context passed as the second argument to `handle(row, ctx)`."""

    dry_run: bool
    row_index: int
    config: Mapping[str, Any]
    handler_version: str
    run_id: str = ""


HandleFn = Callable[[Mapping[str, Any], Context], Mapping[str, Any]]


def _emit(obj: Mapping[str, Any]) -> None:
    """Write one JSON envelope to stdout and flush.

    Flush is mandatory — Python buffers stdout when piped, which deadlocks
    rowforge waiting for our reply.
    """
    sys.stdout.write(json.dumps(obj, separators=(",", ":")))
    sys.stdout.write("\n")
    sys.stdout.flush()


def _read_line() -> str | None:
    line = sys.stdin.readline()
    if not line:
        return None
    return line.rstrip("\n")


def run(
    handle: HandleFn,
    *,
    handler_version: str = "0.0.0",
) -> None:
    """Run the protocol loop until rowforge sends shutdown (or stdin closes).

    Returns normally; the caller usually has nothing else to do, so calling
    `sys.exit(0)` after is optional. We exit non-zero only if the protocol
    itself breaks (malformed JSON, missing init, etc.).
    """
    # 1. Init.
    init_line = _read_line()
    if init_line is None:
        # No init at all — rowforge crashed or test harness sent EOF.
        return
    try:
        init_msg = json.loads(init_line)
    except json.JSONDecodeError as e:
        print(f"rowforge-handler: malformed init: {e}", file=sys.stderr)
        sys.exit(2)
    if init_msg.get("type") != "init":
        print(f"rowforge-handler: expected init, got {init_msg.get('type')!r}", file=sys.stderr)
        sys.exit(2)

    run_id = init_msg.get("run_id", "")
    config = init_msg.get("config", {}) or {}

    # 2. Ready.
    _emit({"type": "ready", "handler_version": handler_version})

    # 3. Row loop.
    while True:
        line = _read_line()
        if line is None:
            return  # EOF without explicit shutdown — fine
        if not line.strip():
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError as e:
            print(f"rowforge-handler: malformed message: {e}", file=sys.stderr)
            sys.exit(2)

        msg_type = msg.get("type")
        if msg_type == "shutdown":
            return

        if msg_type != "row":
            # Unknown envelope. Tolerate (forward compat).
            continue

        seq = msg.get("seq")
        data = msg.get("data", {}) or {}
        meta = msg.get("meta", {}) or {}
        ctx = Context(
            dry_run=bool(meta.get("dry_run", False)),
            row_index=int(meta.get("row_index", seq if seq is not None else 0)),
            config=config,
            handler_version=handler_version,
            run_id=run_id,
        )

        try:
            result = handle(data, ctx)
        except HandlerError as e:
            # Backward-compat: only emit `data` when the handler explicitly
            # supplied it. Old handlers raising HandlerError(code, msg) stay
            # byte-identical on the wire.
            err = {"type": "error", "seq": seq, "code": e.code, "message": e.message}
            if e.data is not None:
                err["data"] = dict(e.data)
            _emit(err)
            continue
        except Exception as e:  # noqa: BLE001 — spec mandates catch-all
            tb = traceback.format_exc()
            print(tb, file=sys.stderr)  # log full trace
            _emit({
                "type": "error",
                "seq": seq,
                "code": "UNCAUGHT",
                "message": f"{type(e).__name__}: {e}",
            })
            continue

        if not isinstance(result, Mapping):
            _emit({
                "type": "error",
                "seq": seq,
                "code": "BAD_RETURN",
                "message": f"handle() must return a dict, got {type(result).__name__}",
            })
            continue

        _emit({"type": "result", "seq": seq, "data": dict(result)})
