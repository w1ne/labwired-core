# Exact toolchain source-package metadata — qualification pending

[PR184](https://github.com/CrispStrobe/labwired-core/pull/184)'s first
[hosted capture](https://github.com/CrispStrobe/labwired-core/actions/runs/37817595613)
records three installed header-owner packages and nineteen notice references.
Their package file listings supply only the three existing copyright files;
`COPYING3`, `COPYING.RUNTIME` and the referenced `copyright-gcc` are not supplied
by those selected listings. The separate capture audit passed; notice-chain
closure did not. Do not repeat the original firmware build to read this evidence.

This standalone hosted job verifies that exact prior run/artifact/hash and
retains version-specific APT binary control records, corresponding source-package
records and checksum-bound source descriptors. It does not install a compiler,
download a source archive, read an implementation body, build firmware or run a
guest. The script lives outside the original-build collector's trigger directory
so this metadata-only change does not start another original firmware build.

The official Ubuntu package pages identify source records for
[gcc-arm-none-eabi](https://packages.ubuntu.com/noble/gcc-arm-none-eabi) and
[libnewlib-dev](https://packages.ubuntu.com/noble/libnewlib-dev); these are discovery
links, not acceptance evidence. The actual collector must bind all three exact
versions from the qualified artifact rather than selecting a current/latest
package or substituting unrelated host GCC documentation.

Acceptance requires the first actual hosted artifact, a separate raw control/
descriptor/hash/identity audit and all enabled final-head checks. Preserve missing
versions, ambiguous records, checksum mismatches and other first failures; do not
waive them or relabel a different package as the captured one.

APT trust and index provenance remain hosted observations. A matching descriptor
checksum is not independent signature verification or source-to-binary proof.
Archive member names/hashes are retained for a later bounded acquisition/review;
no archive members are fetched here. Exact COPYING/exception correspondence,
generated/assembly/retained-inline provenance and distribution obligations remain
open. No firmware admission, app pin, capture, ACK, performance or store claim
changes. This is evidence preparation for R0/R1, not legal clearance or an
isolated implementation process.
