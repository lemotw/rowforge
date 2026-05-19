# python3-uppercase handler

Minimal hand-rolled rowforge handler in Python. Demonstrates the wire protocol
without any SDK dependency.

**Behavior:** reads `name` field from each row, returns `name_upper` (the field
uppercased). Empty names produce `EMPTY_NAME` error.

## Run

From the repo root:

```bash
./target/release/rowforge run \
    --handler examples/handlers/python3-uppercase \
    --input examples/sample.csv \
    --output-dir examples/out/uppercase/
```

Then:

```bash
cat examples/out/uppercase/success.csv
```

Expected:

```csv
name,email,name_upper,_meta_row,_meta_dur_ms,_meta_handler_ver
Alice,alice@example.com,ALICE,0,...,0.1.0
Bob,bob@gmail.com,BOB,1,...,0.1.0
...
```

## Requirements

- `python3` on PATH (≥3.6).

No external pip packages needed — uses only stdlib `json` and `sys`.
