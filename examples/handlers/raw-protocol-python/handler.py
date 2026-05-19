#!/usr/bin/env python3
"""Hand-rolled rowforge handler in Python — NO SDK, demonstrates the raw wire protocol.

Most users should use the SDK (`pip install rowforge-handler` or the in-tree
`sdks/python/`). This example exists as a reference: if you want to write a
handler in a language without an official SDK, this is roughly what your
implementation needs to do.

Wire protocol:
  stdin  ← {"type":"init",...}     (one line)
  stdout → {"type":"ready",...}    (one line)
  stdin  ← {"type":"row","seq":N,"data":{...},"meta":{...}}  (per row)
  stdout → {"type":"result","seq":N,"data":{...}}            (or error)
  stdin  ← {"type":"shutdown"}
"""

import json
import sys


def emit(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()


def main():
    init_line = sys.stdin.readline()
    if not init_line:
        sys.exit(0)
    emit({"type": "ready", "handler_version": "0.1.0"})

    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        msg = json.loads(line)
        if msg.get("type") == "shutdown":
            return
        if msg.get("type") != "row":
            continue
        seq = msg["seq"]
        name = msg.get("data", {}).get("name", "")
        if not name:
            emit({"type": "error", "seq": seq, "code": "EMPTY_NAME", "message": "input 'name' field is empty"})
            continue
        emit({"type": "result", "seq": seq, "data": {"name_upper": name.upper()}})


if __name__ == "__main__":
    main()
