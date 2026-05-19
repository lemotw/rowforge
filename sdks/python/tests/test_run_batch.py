"""End-to-end tests for the batch-mode protocol loop.

Runs handler scripts as subprocesses and asserts their stdout matches the
expected batch_result envelopes.
"""

import json
import os
import subprocess
import sys
import textwrap
from pathlib import Path

SDK_ROOT = Path(__file__).resolve().parent.parent


def _run_batch_handler(
    script: str,
    stdin_lines: list[dict],
    extra_env: dict | None = None,
) -> tuple[list[dict], str, int]:
    """Run a batch handler script with the SDK on PYTHONPATH; return (stdout_envelopes, stderr, exit_code)."""
    env = {"PYTHONPATH": str(SDK_ROOT), **(extra_env or {})}
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


HAPPY_BATCH_SCRIPT = textwrap.dedent("""
    from rowforge_handler import run_batch
    def handle(rows, ctx):
        return [{"upper": r["data"]["name"].upper()} for r in rows]
    run_batch(handle, handler_version="9.9.9")
""")


def test_batch_happy_path():
    envs, stderr, code = _run_batch_handler(
        HAPPY_BATCH_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": ["name"]},
            {
                "type": "batch",
                "rows": [
                    {"seq": 0, "data": {"name": "alice"}, "meta": {}},
                    {"seq": 1, "data": {"name": "bob"}, "meta": {}},
                    {"seq": 2, "data": {"name": "carol"}, "meta": {}},
                ],
            },
            {"type": "shutdown"},
        ],
    )
    assert code == 0, f"stderr: {stderr}"
    assert envs[0] == {"type": "ready", "handler_version": "9.9.9"}
    assert envs[1] == {
        "type": "batch_result",
        "results": [
            {"kind": "result", "data": {"upper": "ALICE"}},
            {"kind": "result", "data": {"upper": "BOB"}},
            {"kind": "result", "data": {"upper": "CAROL"}},
        ],
    }


MIXED_BATCH_SCRIPT = textwrap.dedent("""
    from rowforge_handler import run_batch, HandlerError
    def handle(rows, ctx):
        out = []
        for r in rows:
            name = r["data"].get("name")
            if not name:
                out.append(HandlerError("EMPTY_NAME", "name is empty"))
            else:
                out.append({"upper": name.upper()})
        return out
    run_batch(handle)
""")


def test_batch_mixed_results():
    envs, stderr, code = _run_batch_handler(
        MIXED_BATCH_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": ["name"]},
            {
                "type": "batch",
                "rows": [
                    {"seq": 0, "data": {"name": "alice"}, "meta": {}},
                    {"seq": 1, "data": {"name": ""}, "meta": {}},
                    {"seq": 2, "data": {"name": "carol"}, "meta": {}},
                ],
            },
            {"type": "shutdown"},
        ],
    )
    assert code == 0, f"stderr: {stderr}"
    assert envs[1]["type"] == "batch_result"
    results = envs[1]["results"]
    assert len(results) == 3
    assert results[0] == {"kind": "result", "data": {"upper": "ALICE"}}
    assert results[1] == {"kind": "error", "code": "EMPTY_NAME", "message": "name is empty"}
    assert results[2] == {"kind": "result", "data": {"upper": "CAROL"}}


WRONG_LENGTH_SCRIPT = textwrap.dedent("""
    from rowforge_handler import run_batch
    def handle(rows, ctx):
        # Return only 2 results for whatever batch size.
        return [{"ok": True}, {"ok": True}]
    run_batch(handle)
""")


def test_batch_wrong_length():
    envs, stderr, code = _run_batch_handler(
        WRONG_LENGTH_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": []},
            {
                "type": "batch",
                "rows": [
                    {"seq": 0, "data": {}, "meta": {}},
                    {"seq": 1, "data": {}, "meta": {}},
                    {"seq": 2, "data": {}, "meta": {}},
                ],
            },
            {"type": "shutdown"},
        ],
    )
    assert code == 2, f"stderr: {stderr}"
    # No batch_result envelope should be emitted.
    batch_results = [e for e in envs if e.get("type") == "batch_result"]
    assert batch_results == []
    assert "2 results for 3 rows" in stderr


NON_SEQUENCE_SCRIPT = textwrap.dedent("""
    from rowforge_handler import run_batch
    def handle(rows, ctx):
        return None
    run_batch(handle)
""")


def test_batch_returns_non_sequence():
    envs, stderr, code = _run_batch_handler(
        NON_SEQUENCE_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": []},
            {
                "type": "batch",
                "rows": [
                    {"seq": 0, "data": {}, "meta": {}},
                ],
            },
            {"type": "shutdown"},
        ],
    )
    assert code == 2, f"stderr: {stderr}"
    batch_results = [e for e in envs if e.get("type") == "batch_result"]
    assert batch_results == []


PANIC_SCRIPT = textwrap.dedent("""
    from rowforge_handler import run_batch
    def handle(rows, ctx):
        raise RuntimeError("boom")
    run_batch(handle)
""")


def test_batch_panic_exits_2():
    envs, stderr, code = _run_batch_handler(
        PANIC_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": []},
            {
                "type": "batch",
                "rows": [
                    {"seq": 0, "data": {}, "meta": {}},
                ],
            },
            {"type": "shutdown"},
        ],
    )
    assert code == 2
    assert "RuntimeError" in stderr
    assert "boom" in stderr
    batch_results = [e for e in envs if e.get("type") == "batch_result"]
    assert batch_results == []


def test_batch_shutdown_exits_clean():
    envs, stderr, code = _run_batch_handler(
        HAPPY_BATCH_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": []},
            {"type": "shutdown"},
        ],
    )
    assert code == 0, f"stderr: {stderr}"
    # Only ready was emitted.
    assert envs == [{"type": "ready", "handler_version": "9.9.9"}]


def test_batch_unknown_envelope_ignored():
    envs, stderr, code = _run_batch_handler(
        HAPPY_BATCH_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": ["name"]},
            {"type": "futureproof", "weird": "field"},
            {
                "type": "batch",
                "rows": [
                    {"seq": 0, "data": {"name": "x"}, "meta": {}},
                ],
            },
            {"type": "shutdown"},
        ],
    )
    assert code == 0, f"stderr: {stderr}"
    # ready + 1 batch_result; unknown envelope didn't break the loop.
    assert len(envs) == 2
    assert envs[1]["type"] == "batch_result"
    assert envs[1]["results"] == [{"kind": "result", "data": {"upper": "X"}}]


BATCH_FAILURE_WITH_DATA_SCRIPT = textwrap.dedent("""
    from rowforge_handler import run_batch, HandlerError
    def handle(rows, ctx):
        out = []
        for r in rows:
            billid = r["data"].get("billid", "")
            out.append(HandlerError("EMPTY_FAIL_FIELDS", "demo", data={"billid": billid}))
        return out
    run_batch(handle)
""")


def test_batch_handler_error_with_data_emits_data_field():
    # HandlerError(..., data=...) inside a batch must surface a `data` key
    # on the per-entry error envelope so rowforge can render
    # schema.failed_output columns in failed.csv.
    envs, stderr, code = _run_batch_handler(
        BATCH_FAILURE_WITH_DATA_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": ["billid"]},
            {"type": "batch", "rows": [
                {"seq": 0, "data": {"billid": "B1"}, "meta": {}},
                {"seq": 1, "data": {"billid": "B2"}, "meta": {}},
            ]},
            {"type": "shutdown"},
        ],
    )
    assert code == 0, f"stderr: {stderr}"
    results = envs[1]["results"]
    assert len(results) == 2
    for entry, want in zip(results, ["B1", "B2"]):
        assert entry["kind"] == "error"
        assert entry["code"] == "EMPTY_FAIL_FIELDS"
        assert entry["data"] == {"billid": want}


BATCH_FAILURE_WITHOUT_DATA_SCRIPT = textwrap.dedent("""
    from rowforge_handler import run_batch, HandlerError
    def handle(rows, ctx):
        return [HandlerError("BAD_INPUT", "x") for _ in rows]
    run_batch(handle)
""")


def test_batch_handler_error_without_data_omits_data_field():
    # Old-shape HandlerError(code, msg) stays byte-identical (no `data` key).
    envs, _, code = _run_batch_handler(
        BATCH_FAILURE_WITHOUT_DATA_SCRIPT,
        [
            {"type": "init", "run_id": "r1", "config": {}, "columns": []},
            {"type": "batch", "rows": [{"seq": 0, "data": {}, "meta": {}}]},
            {"type": "shutdown"},
        ],
    )
    assert code == 0
    entry = envs[1]["results"][0]
    assert entry["kind"] == "error"
    assert "data" not in entry
