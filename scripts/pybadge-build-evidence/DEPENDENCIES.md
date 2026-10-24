# Compiled header/source dependencies — qualification pending

The first hosted attempt, [run37785853900](https://github.com/CrispStrobe/labwired-core/actions/runs/37785853900)
at source `dc495bbf74244d654dfeb9a27791bcec92d55318`, failed because the
collector found no `*.o.d` rules after the original build and map capture passed.
It remains a collector failure, not dependency qualification or firmware failure.
The diagnostic follow-up retains bounded generated CMake dependency metadata plus a build
file-name/size inventory before requiring GCC rules. Inspect that actual discovery
before adding support for another format; no synthetic dependency list substitutes
for evidence. No object, ELF, HEX or source bodies are copied by discovery.

Actual [run37787234826](https://github.com/CrispStrobe/labwired-core/actions/runs/37787234826)
at `43f3c9fc3a5ff8c02fb67f673212f9b6754a02aa` also failed the `.o.d` requirement,
but retained 23 generated metadata files and a 273-file name/size inventory.
The original generated compiler commands use the shared literal `-MF DEPFILE`;
the inventory contains four such files. The collector now captures those raw
surviving GCC rules with their generated-recipe binding and explicitly marks
compiled coverage incomplete. They cannot recover earlier overwritten rules.
Missing screen dependencies are an explicit field, not inferred from another
translation unit. Full coverage needs a separately named dependency-only compiler
probe with exact source/flags/toolchain binding, not silent historical-build edits.

The original-map qualification in PR177 establishes a bounded object-level
inventory, not the provenance of headers or inlined code. This follow-up captures
the clean build's original GCC `.o.d` rules and inventories their dependency
paths/hashes, separating build-source from toolchain files. Raw source bodies
are not copied into the artifact; raw dependency rules are retained unchanged
for independent parsing. Presence of a vendor-use phrase is a review
hint only; its absence never grants permission or classifies a file as clean.

The scope is deliberately conservative: **only surviving observed compiler rules**,
including code the linker may discard. Overwritten shared rules, missing rules and
assembly coverage remain explicit gaps. It does not establish which header bytes
became retained instructions, complete binary correspondence, or licence
clearance. Review against the actual map and immutable component origins before
choosing a permissive replacement boundary. Source access must remain recorded;
this is not an isolated implementation or whole-firmware clean-room claim.

Require an actual hosted artifact and all enabled exact-head checks. Missing or
unsupported GCC rule syntax fails explicitly; no fallback invents a dependency
list. No compiler/linker/guest execution is needed by this collector, and no app
pin, historical-image classification, hardware ACK or benchmark floor changes.
