# IO-Link Device Stack Isolation

LabWired needs more than one IO-Link device instance for a multi-port master,
and it must do that with the same C stacks used by real firmware paths.
`core/third_party/iolinki` now exposes the reentrant `iolink_device_ctx_t` API,
and `third_party/iolinki-master` exposes caller-owned master port/controller
contexts.

Decision:

- Native LabWired IO-Link tests and host-side simulation use real
  `iolinki-master` plus real `iolinki` device contexts.
- Multiple stack-backed devices are allowed in one process by allocating one
  `iolink_device_ctx_t` per device and passing transport state through PHY
  `user` pointers.
- Do not add protocol shims, prefixed duplicate C builds, or simulator-only
  IO-Link behavior to make multi-port tests pass.

GPL note:

`core/third_party/iolinki` carries GPL headers. Linking it directly into
`labwired-core` may affect distribution. Until licensing is resolved, stack
backed device simulation remains behind `iolink-native` and must not be
claimed as browser-safe or default-distribution-safe.
