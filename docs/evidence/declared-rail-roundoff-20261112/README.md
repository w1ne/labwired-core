# Declared rail numerical-zero and atomic publication proof

The frozen source passed 55 focused cases with no failures or ignored tests:
5 publication/lock cases, 14 external-device rail cases, 12 routed/World cases,
13 MCU domain cases, 5 owner-timing cases and 6 legacy shared-supply cases.

The raw samples remain unchanged. A bounded receiver admits only a finite
negative value within one relative f64 epsilon of its own declared maximum
voltage. Availability still uses the original operating range. A receiver with
no declared maximum receives no new allowance.

Late-rejection tests cover supervisor deadline overflow, reset-event queue
exhaustion after a pending BOD request is consumed in the detached preview,
backward time, missing required fresh rails, duplicate targets, device
generation exhaustion and poisoned receiver locks. They compare complete
supervisor state and verify peer device power remains unchanged. The callback
test reads both newly published states and queues a future BOD request from the
device callback without deadlock or losing the request.

The source manifest and compressed raw logs bind these claims. The first
revision's failed assertion is retained separately: that test incorrectly
expected the raw-voltage accessor to hide an admitted out-of-range value.
The corrected assertion checks raw bits and unavailable state separately.

This is generic receiver evidence. It does not establish native firmware
startup, reverse-supply hardware behavior, motor operation, or flight readiness.
The exact MCU model/capture acknowledgment remains a separate gate.
