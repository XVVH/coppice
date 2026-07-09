# ADR 0003 — MCP proxy: hand-rolled JSON-RPC passthrough, not an SDK server

**Status:** accepted — 2026-07-08 (milestone 2)

## Decision

`asf proxy` implements the MCP wire (newline-delimited JSON-RPC 2.0 over
stdio) directly, ~80 lines in `crates/asf-cli/src/mcp.rs`, instead of
building on the official `rmcp` SDK. Exactly two methods are ever
interpreted: `tools/call` (the broker pipeline) and `tools/list` (response
filtered to the session grant). Everything else — handshake, notifications,
resources, sampling, methods that don't exist yet — passes through
byte-faithfully in both directions.

## Why

1. **A proxy's correctness property is transparency.** The brief's strategic
   claim is "works with all agents, true by mechanism" (§9). A typed SDK
   server must understand a message to relay it; unknown methods and future
   protocol revisions become breakage instead of passthrough. Raw relay
   inverts that: we are compatible with protocol features we have never
   heard of, which is the right default for a gateway that fronts arbitrary
   agent↔server pairs.
2. **The interception surface is tiny and load-bearing.** Broker enforcement
   needs `tools/call` (and, later, `tools/list` filtering). Implementing two
   methods does not justify an SDK dependency on the enforcement path — the
   security-critical code should be small enough to read in one sitting.
3. **rmcp remains the right choice for milestone 3+** if/when the daemon
   grows first-party MCP *server* features of its own (elicitation for JIT
   consent is in the MCP spec and on our roadmap). This ADR is about the
   relay path, not a ban.

## Consequences / limits (Stage 2)

- The proxy relays whole lines; it does not validate JSON-RPC beyond what
  interception requires. Malformed downstream traffic passes through —
  by design (the agent client is the party that should reject it).
- Concurrent in-flight `tools/call`s are supported (responses are routed by
  id via the intercepted-id map), but heavy bidirectional streaming has no
  backpressure story yet — acceptable at one-agent dogfooding scale.
- `tools/list` responses are filtered to the session grant: an action is
  advertised iff it is registered AND every statically checkable dimension
  (action.allow, reversibility.max, external_reach) either passes or is
  escalatable. Escalatable actions stay visible on purpose — attempting
  them is how JIT elicitation starts; flat-out unreachable ones are noise
  and an injection target. Dynamic dimensions (paths, budgets, time) never
  filter. Enforcement never depends on the advertisement: hidden actions
  are still independently denied at call time (smoke-tested both ways).
  Not yet done: emitting `notifications/tools/list_changed` when a
  re-manifest changes the grant mid-session (milestone 3, with
  re-manifest-aware sessions generally).

## C2 topology (recorded here because it is structural)

The approval surface is a Unix socket owned by the daemon
(`<home>/approvals.sock`, driven by `asf approve` in a separate terminal).
The MCP stream has no approval method; the smoke test drives an invented
`asf/approve` method through the proxy and asserts it falls through to the
downstream and errors. An approval "carried by the agent" is not forbidden —
it is unrepresentable.
