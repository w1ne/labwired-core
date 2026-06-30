# IO-Link Device Stack Isolation

LabWired needs more than one IO-Link device instance for a multi-port master.
The existing `core/third_party/iolinki` device stack currently exposes singleton
entry points such as `iolink_init()` and `iolink_process()`.

Decision:

- Phase 2 supports one stack-backed device instance only.
- Multi-port product modeling must wait for one of:
  - a reentrant `iolink_device_ctx_t` API in `third_party/iolinki`, or
  - a separate process/worker per device instance.

GPL note:

`core/third_party/iolinki` carries GPL headers. Linking it directly into
`labwired-core` may affect distribution. Until licensing is resolved, stack
backed device simulation must remain behind `iolink-native` and must not be
claimed as browser-safe or default-distribution-safe.
