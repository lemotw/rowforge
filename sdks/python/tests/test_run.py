"""End-to-end tests for the protocol loop.

Runs handler scripts as subprocesses and asserts their stdout matches the
expected protocol envelopes byte-for-byte (modulo JSON key ordering).
"""

import json
import subprocess
import sys
import textwrap
from pathlib import Path

import pytest

SDK_ROOT = Path(__file__).resolve().parent.parent


def _run_handler(script: str, stdin_lines: list[dict], extra_env: dict | None = None) -> tuple[list[dict], str, int]:
    """Run a handler script with the SDK on PYTHONPATH; return (stdout_envelopes, stderr, exit_code)."""
    env = {"PYTHONPATH": str(SDK_ROOT), **(extra_env or {})}
    # Inherit PATH so python3 still works.
    import os
    env["PATH"] = os.environ.get("PATH", "")
    stdin_blob = "".join(json.dumps(m) + "\n" for m in stdin_lines)
    proc = subprocess.run(
        [sys.executable, "-c", script],
        input=stdin_blob,
        capture_output=True,
        text=True,
        env=env,
        timeout=10,
    )
    envelopes = []
    for line in proc.stdout.splitlines():
        if line.strip():
            envelopes.append(json.loads(line))
    return envelopes, proc.stderr, proc.returncode


HAPPY_SCRIPT = textwrap.dedent("""
    from rowforge_handler import run
    def handle(row, ctx):
        return {"upper": row["name"].upper()}
    run(handle, handler_version="9.9.9")
""")


def test_happy_path_emits_ready_then_results():
    envs, stderr, code = _run_handler(
        HAPPY_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": ["name"]},
            {"type": "row", "seq": 0, "data": {"name": "alice"}, "meta": {"dry_run": False, "row_index": 0}},
            {"type": "row", "seq": 1, "data": {"name": "bob"}, "meta": {"dry_run": False, "row_index": 1}},
            {"type": "shutdown"},
        ],
    )
    assert code == 0, f"stderr: {stderr}"
    assert envs[0] == {"type": "ready", "handler_version": "9.9.9"}
    assert envs[1] == {"type": "result", "seq": 0, "data": {"upper": "ALICE"}}
    assert envs[2] == {"type": "result", "seq": 1, "data": {"upper": "BOB"}}


HANDLER_ERROR_SCRIPT = textwrap.dedent("""
    from rowforge_handler import run, HandlerError
    def handle(row, ctx):
        if not row.get("name"):
            raise HandlerError("EMPTY_NAME", "name is empty")
        return {"upper": row["name"].upper()}
    run(handle)
""")


def test_handler_error_emits_error_envelope():
    envs, stderr, code = _run_handler(
        HANDLER_ERROR_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": ["name"]},
            {"type": "row", "seq": 5, "data": {"name": ""}, "meta": {}},
            {"type": "shutdown"},
        ],
    )
    assert code == 0
    assert envs[1] == {"type": "error", "seq": 5, "code": "EMPTY_NAME", "message": "name is empty"}


UNCAUGHT_SCRIPT = textwrap.dedent("""
    from rowforge_handler import run
    def handle(row, ctx):
        return 1 / 0  # ZeroDivisionError
    run(handle)
""")


def test_uncaught_exception_becomes_error_with_code_uncaught():
    envs, stderr, code = _run_handler(
        UNCAUGHT_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": []},
            {"type": "row", "seq": 0, "data": {}, "meta": {}},
            {"type": "shutdown"},
        ],
    )
    assert code == 0
    assert envs[1]["type"] == "error"
    assert envs[1]["seq"] == 0
    assert envs[1]["code"] == "UNCAUGHT"
    assert "ZeroDivisionError" in envs[1]["message"]
    # Traceback was logged to stderr (audit trail).
    assert "ZeroDivisionError" in stderr


CONTEXT_SCRIPT = textwrap.dedent("""
    from rowforge_handler import run
    def handle(row, ctx):
        return {
            "dry_run": ctx.dry_run,
            "row_index": ctx.row_index,
            "cfg_x": ctx.config.get("x"),
        }
    run(handle, handler_version="0.7.0")
""")


def test_context_propagates_meta_and_config():
    envs, stderr, code = _run_handler(
        CONTEXT_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {"x": 42}, "columns": []},
            {"type": "row", "seq": 0, "data": {}, "meta": {"dry_run": True, "row_index": 7}},
            {"type": "shutdown"},
        ],
    )
    assert code == 0, f"stderr: {stderr}"
    assert envs[0] == {"type": "ready", "handler_version": "0.7.0"}
    assert envs[1]["data"] == {"dry_run": True, "row_index": 7, "cfg_x": 42}


BAD_RETURN_SCRIPT = textwrap.dedent("""
    from rowforge_handler import run
    def handle(row, ctx):
        return "not a dict"
    run(handle)
""")


def test_non_dict_return_becomes_bad_return_error():
    envs, stderr, code = _run_handler(
        BAD_RETURN_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": []},
            {"type": "row", "seq": 0, "data": {}, "meta": {}},
            {"type": "shutdown"},
        ],
    )
    assert code == 0
    assert envs[1]["code"] == "BAD_RETURN"


def test_eof_without_shutdown_exits_cleanly():
    envs, stderr, code = _run_handler(
        HAPPY_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": ["name"]},
            {"type": "row", "seq": 0, "data": {"name": "x"}, "meta": {}},
            # No shutdown — stdin closes after this row.
        ],
    )
    assert code == 0, f"stderr: {stderr}"
    assert len(envs) == 2  # ready + 1 result


def test_unknown_envelope_is_ignored():
    envs, stderr, code = _run_handler(
        HAPPY_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": ["name"]},
            {"type": "futureproof", "weird": "field"},
            {"type": "row", "seq": 0, "data": {"name": "x"}, "meta": {}},
            {"type": "shutdown"},
        ],
    )
    assert code == 0, f"stderr: {stderr}"
    # Only ready + 1 result; the unknown envelope didn't break anything.
    assert len(envs) == 2


HANDLER_ERROR_WITH_DATA_SCRIPT = textwrap.dedent("""
    from rowforge_handler import run, HandlerError
    def handle(row, ctx):
        raise HandlerError("DEMO_FAIL", "always", data={"billid": row.get("billid", "")})
    run(handle)
""")


def test_handler_error_with_data_emits_data_field():
    # New: HandlerError(code, msg, data=...) attaches the payload to the
    # wire envelope so rowforge can render schema.failed_output columns.
    envs, stderr, code = _run_handler(
        HANDLER_ERROR_WITH_DATA_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": ["billid"]},
            {"type": "row", "seq": 0, "data": {"billid": "B42"}, "meta": {}},
            {"type": "shutdown"},
        ],
    )
    assert code == 0, f"stderr: {stderr}"
    assert envs[1] == {
        "type": "error",
        "seq": 0,
        "code": "DEMO_FAIL",
        "message": "always",
        "data": {"billid": "B42"},
    }


def test_handler_error_without_data_omits_data_field():
    # Backward-compat: old HandlerError(code, msg) callers stay
    # byte-identical (no `data` key on the wire).
    envs, _, code = _run_handler(
        HANDLER_ERROR_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": ["name"]},
            {"type": "row", "seq": 1, "data": {"name": ""}, "meta": {}},
            {"type": "shutdown"},
        ],
    )
    assert code == 0
    assert "data" not in envs[1]
