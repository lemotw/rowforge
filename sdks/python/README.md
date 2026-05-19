# rowforge-handler (Python SDK)

```python
from rowforge_handler import run, HandlerError

def handle(row, ctx):
    if "@" not in row["email"]:
        raise HandlerError("INVALID_FORMAT", "missing @")
    return {"domain": row["email"].split("@")[1]}

if __name__ == "__main__":
    run(handle, handler_version="0.1.0")
```

That's it. The SDK owns the stdio loop, message dispatch, error packaging, and shutdown handling. See the spec at `docs/superpowers/specs/2026-05-10-rowforge-design.md` §6.4 for the full contract.
