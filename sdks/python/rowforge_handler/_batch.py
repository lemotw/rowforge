"""Batch consumption mode protocol loop.

User-facing API: see __init__.py docstring or doc.
"""

from __future__ import annotations

import json
import sys
import traceback
from typing import Any, Callable, Mapping, Sequence, Union

from ._protocol import Context, HandlerError, _emit, _read_line


BatchEntry = Union[Mapping[str, Any], HandlerError]
BatchHandleFn = Callable[[Sequence[Mapping[str, Any]], Context], Sequence[BatchEntry]]


def run_batch(
    handle: BatchHandleFn,
    *,
    handler_version: str = "0.0.0",
) -> None:
    """Run the batch-mode protocol loop until shutdown / EOF.

    The handle function receives a sequence of row dicts (each with 'seq',
    'data', 'meta' keys) and a Context. It must return a sequence of exactly
    len(rows) entries, each either a dict (success) or a HandlerError
    (failure). Positional: results[i] corresponds to rows[i].
    """
    # init
    init_line = _read_line()
    if init_line is None:
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

    _emit({"type": "ready", "handler_version": handler_version})

    while True:
        line = _read_line()
        if line is None:
            return
        if not line.strip():
            continue
        try:
            msg = json.loads(line)
        except json.JSONDecodeError as e:
            print(f"rowforge-handler: malformed envelope: {e}", file=sys.stderr)
            sys.exit(2)

        msg_type = msg.get("type")
        if msg_type == "shutdown":
            return
        if msg_type != "batch":
            continue  # forward-compat

        rows = msg.get("rows", []) or []
        ctx = Context(
            dry_run=False,
            row_index=0,
            config=config,
            handler_version=handler_version,
            run_id=run_id,
        )

        try:
            results = handle(rows, ctx)
        except Exception:
            traceback.print_exc(file=sys.stderr)
            sys.exit(2)

        # length validation
        try:
            results_len = len(results)
        except TypeError:
            print("rowforge-handler: handle did not return a sequence", file=sys.stderr)
            sys.exit(2)

        if results_len != len(rows):
            print(
                f"rowforge-handler: handle returned {results_len} results for {len(rows)} rows",
                file=sys.stderr,
            )
            sys.exit(2)

        entries = []
        for r in results:
            if isinstance(r, HandlerError):
                entry = {"kind": "error", "code": r.code, "message": r.message}
                if r.data is not None:
                    entry["data"] = dict(r.data)
                entries.append(entry)
            elif isinstance(r, Mapping):
                entries.append({"kind": "result", "data": dict(r)})
            else:
                entries.append({
                    "kind": "error",
                    "code": "BAD_RETURN",
                    "message": f"expected dict or HandlerError, got {type(r).__name__}",
                })

        _emit({"type": "batch_result", "results": entries})
