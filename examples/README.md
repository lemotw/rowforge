# rowforge examples

Each handler is a self-contained directory: `rowforge.yaml` manifest + source.
`sample.csv` provides shared input rows (`name,email`).

## SDK-based examples (recommended)

| Folder | Language | What it does |
|--------|----------|--------------|
| `handlers/python3-uppercase/` | Python 3 | Uppercases the `name` field — uses `rowforge_handler` (Python SDK) |
| `handlers/golang-domain/` | Go ≥1.22 | Extracts the domain from `email` — uses `github.com/lemotw/rowforge/sdks/go` |

These are the canonical way to write a handler. Business logic is one function;
the SDK owns the stdio loop, dispatch, and shutdown.

## Raw-protocol reference

| Folder | Language | What it does |
|--------|----------|--------------|
| `handlers/raw-protocol-python/` | Python 3 | Same logic as `python3-uppercase`, hand-rolling the JSON-Lines protocol |

Kept as a reference for porting the SDK to a new language or for debugging
protocol-level issues. **Not** the recommended way to write a handler — use the
SDK examples above. Wire protocol details: see
`docs/superpowers/specs/2026-05-10-rowforge-design.md` §6.3.

## Install / build

```bash
# Python (python3-uppercase)
cd examples/handlers/python3-uppercase && pip install -r requirements.txt

# Go (golang-domain)
cd examples/handlers/golang-domain && go mod tidy && go build -o handler handler.go
```

The Go SDK isn't published yet; `golang-domain/go.mod` uses a `replace`
directive pointing at `sdks/go/` in this repo.

## Run a handler

```bash
# build rowforge once
cargo build --release

# run python3-uppercase against sample.csv
./target/release/rowforge run \
    --handler examples/handlers/python3-uppercase \
    --input examples/sample.csv \
    --output-dir examples/out/uppercase/
cat examples/out/uppercase/success.csv
```

Swap `--handler` for `examples/handlers/golang-domain` (after building it) to
run the Go example.
