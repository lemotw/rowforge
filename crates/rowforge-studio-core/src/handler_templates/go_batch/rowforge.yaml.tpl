name: {{name}}
version: 0.1.0
description: "Batch-mode handler scaffolded by Studio"
language: go

entry:
  cmd: ["./{{name}}"]
  build: ["go", "build", "-o", "{{name}}", "handler.go"]
  startup_timeout_ms: 10000

runtime:
  mode: batch
  batch_size: 5
  idempotent: true

required_input: ["{{primary_field}}"]
