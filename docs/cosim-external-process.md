# Bounded persistent external-process co-simulation

`ExternalProcessCosimAdapter` keeps one native Unix child for the existing JSONL
request/response protocol. A step sends `time_ns`, `dt_ns` and scalar `inputs`;
its one response contains scalar `outputs`. Valid bool, integer, finite float
and text signals retain their existing types. Nonfinite request numbers are
rejected instead of silently becoming JSON null.

`spawn` uses a two-second exchange deadline and 65,536-byte request and response
limits, including the newline. `spawn_with_options` accepts explicit timeout
and frame limits through `ExternalProcessCosimOptions`; timeout must be positive
and no more than 60 seconds, and frames are 128 through 1,048,576 bytes. The native
manifest adapter uses the documented defaults. These are wall-clock IPC safety
bounds, separate from model `step_ns` and simulated time.

Native Unix pipes are nonblocking. One monotonic deadline covers encoding,
partial writes, interrupted polling, partial reads and response decoding. Caller
strings are borrowed for size/finite preflight before building a JSON tree; a
capped writer bounds escaped JSON allocation. Reads use a fixed 4 KiB buffer and
a capped response vector (capacity growth is bounded by the selected frame cap's
next power of two). JSON nesting is limited by serde_json's default recursion
limit. There are no worker threads, channels, background queues or retries.
Unsolicited trailing response bytes and non-scalar outputs are protocol errors.
The adapter also probes nonblocking stdout immediately after a complete frame
and immediately before writing each request. Any byte already queued at those
checks is a terminal protocol error, including an extra frame beyond the 4 KiB
read boundary or arriving between steps. A valid complete one-shot response
followed by EOF remains valid; a later step fails normally. These checks do not
claim to identify the provenance of arbitrary bytes a malicious peer sends after
the pre-request check. The JSONL protocol has no request correlation field.

Any deadline, I/O, request validation or protocol failure is terminal. The
adapter kills its own process group, closes its pipes and polls to reap its own
leader for at most 250 ms. Drop uses the same cleanup, also killing descendants
that inherited stdio. No foreign process group is touched. A child stuck in an
uninterruptible kernel wait can outlive that bounded reap interval; cleanup never
turns that host failure into an unbounded `wait()`. Later steps fail immediately
without another request, child restart or replacement battery state.

This implementation deliberately supports native Unix only. Browser/WASM and
Windows return an explicit unsupported error at spawn; the browser continues to
compile without Unix dependencies. Windows previously exposed the unbounded
synchronous process path; it now fails explicitly until a bounded Windows pipe
transport is implemented. No bounded Windows behavior is claimed.

Tests: `cargo test -p labwired-core --test cosim_external_process_bounds
--test cosim_contract --test cosim_canonical_basket`. The canonical basket test
uses the generated native manifest and the existing Python power wrapper,
not a Linux guest or new private battery/sensor implementation.
