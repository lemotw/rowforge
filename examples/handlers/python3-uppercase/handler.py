#!/usr/bin/env python3
"""Uppercase the 'name' field. Demonstrates a minimal Python handler using the SDK."""

from rowforge_handler import HandlerError, run


def handle(row, _ctx):
    name = row.get("name", "")
    if not name:
        raise HandlerError("EMPTY_NAME", "input 'name' field is empty")
    return {"name_upper": name.upper()}


if __name__ == "__main__":
    run(handle, handler_version="0.1.0")
