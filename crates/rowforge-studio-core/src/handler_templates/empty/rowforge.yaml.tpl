name: {{name}}
version: 0.1.0
description: "Skeleton handler — fill in handler.go and update build/cmd"
language: go

entry:
  cmd: ["./handler"]
  # build is optional; uncomment when you have a handler binary:
  # build: ["go", "build", "-o", "handler", "handler.go"]
  startup_timeout_ms: 10000

runtime:
  mode: row

# Add input columns the handler requires (semicolons in CSV / JSONL keys):
required_input: ["{{primary_field}}"]
