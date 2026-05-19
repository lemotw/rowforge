# raw-protocol-python

Reference handler showing the raw JSON-Lines wire protocol with no SDK.

For 99% of cases use `examples/handlers/python3-uppercase/` instead — it does the same thing in 10 lines using `rowforge-handler`.

This example exists for:
1. People porting rowforge to a language without an official SDK (use this as the protocol blueprint)
2. Debugging — sometimes you want to see exactly what's on the wire

The wire protocol itself is normative — see `docs/superpowers/specs/2026-05-10-rowforge-design.md` §6.3.
