# golang-uppercase

Minimal Go handler example. Uppercases the `name` field of each input row.

Sister handler to `examples/handlers/python3-uppercase` — same logic, two
SDKs, useful for verifying the protocol parity across language SDKs.

## Build + run

```bash
# Build the handler binary (rowforge auto-builds via entry.build; this is
# only for local testing in isolation):
cd examples/handlers/golang-uppercase
go build -o handler handler.go

# Run via rowforge CLI:
cd ../../..
printf 'name\nalice\nbob\ncarol\n' > /tmp/names.csv
rowforge exec start --csv /tmp/names.csv --name uppercase-demo
EXEC=$(rowforge exec list | head -2 | tail -1 | awk '{print $1}')
rowforge exec run $EXEC --handler examples/handlers/golang-uppercase

# Inspect outcomes:
cat ~/.rowforge/executions/$EXEC/attempts/*/outcomes.jsonl
```

## Export

```bash
rowforge exec export $EXEC --output-dir /tmp/exp
cat /tmp/exp/success.csv
```

Expected output:

```csv
seqid,name_upper
0,ALICE
1,BOB
2,CAROL
```

## JSONL input

The same handler accepts JSONL too — `exec start` auto-detects by extension:

```bash
printf '{"name":"alice"}\n{"name":"bob"}\n' > /tmp/names.jsonl
rowforge exec start --csv /tmp/names.jsonl --name uppercase-jsonl-demo
# ... continue as above
```
