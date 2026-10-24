# Separate dependency-only compiler probes — qualification pending

The original build uses shared literal `-MF DEPFILE` outputs, as observed in
[run37787234826](https://github.com/CrispStrobe/labwired-core/actions/runs/37787234826).
The successful [run37787957105](https://github.com/CrispStrobe/labwired-core/actions/runs/37787957105)
captured four surviving original rules and 246 unique dependencies, but not the
screen translation unit. These original rules cannot recover overwritten units.

This follow-up parses the observed generated C/C++ compile-command grammar and
its target flags without executing a shell. It replaces compilation/output
arguments with a **separate `-M` dependency-only invocation**, including system
headers. The exact diagnostic invocation, source/recipe/flags/compiler hashes,
raw diagnostic rules and generated compiler identity records are retained.
Assembly commands are recorded as excluded, not silently counted as covered.
Unexpected syntax, duplicate targets, ambiguous flags or executable/output
options fail explicitly. Original ELF identity must match before and after.

Acceptance requires an actual hosted artifact, separate audit of raw rules and
their invocation/source/configuration bindings, explicit coverage/exclusions,
bounded output and all enabled exact-head checks. Initial failures stay recorded.
No local worker/compiler, firmware guest, original image changes or app adoption
are required. The compiler does preprocessing only; this is not evidence of
the original dependency invocations or retained header/inline bytes. Component
origins/notices and source-to-binary correspondence still need separate review.
No vendor-phrase absence grants clearance and no historical-image admission,
pin, hardware capture/ACK, RTx claim or benchmark floor changes here.
