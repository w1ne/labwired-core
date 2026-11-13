# Declared rail observation boundaries

Named MCU power domains and external device supply rails retain their raw solved
voltage. Their receivers admit a finite negative sample only down to
-f64::EPSILON times that exact rail's declared maximum voltage. This is a
numerical-zero admission rule. It does not clamp a signal, alter the circuit,
relax an operating limit, or qualify a negative rail as powered.

The next representable value below that floor, a substantive negative voltage,
and any nonfinite sample still fail. A larger maximum from another rail cannot
supply the allowance. Legacy supply domains with no explicit maximum keep their
original finite, nonnegative observation contract. The effective_voltage_v
accessor remains a raw observation; availability is reported separately.

## MCU and device rail publication

The router parses every present device sample and every required MCU sample
before preparing either receiver. Device preparation retains all affected
receiver locks and calculates the next complete device states. MCU preparation
holds the live registry and control locks, then advances a detached copy of the
small rail registry, supervisor timers, pending BOD commands and reset events.
It does not copy the CPU, peripheral models or analog circuit.

Only a successful preparation publishes either family. The private publication
callback installs the device states without allocation, fallible work or
notifications; their locks remain held through MCU publication. Existing MCU
handle identities remain unchanged. Device and MCU callbacks run after both
families publish and all receiver locks are released. A callback can therefore
observe both new states and queue the next control request without deadlock or
having that request overwritten.

Unknown/duplicate rails, missing required routed samples, invalid values,
backward MCU time, deadline overflow, reset-event queue exhaustion, device
generation exhaustion and poisoned receiver locks all refuse before either
rail family changes. The direct MCU/device apply APIs use the same preparation
paths. Rejected samples preserve raw observations, timestamps, supervisor
state, queued control requests and reset events. Omitted device rails retain
their previous observation. Existing MCU owner-boundary completeness and timing
rules remain unchanged.

This contract covers the MCU and external-device rail batches. It is not a
transaction over unrelated regulator, motor, pin, ADC or other route effects.

## Evidence scope

Focused tests cover the exact numerical floor, both signed zeros, the next lower
representable value, receiver-specific maxima, NaN/infinities, legacy behavior,
real off-state SPI clocks, full-batch rollback and callback visibility. Late
failure tests consume a detached BOD request before deadline/reset-enqueue
refusal and verify the live request remains pending. The native49 refusal is
retained separately; this patch alone does not establish native firmware startup
or hardware reverse-supply behavior.
