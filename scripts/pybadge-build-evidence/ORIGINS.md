# Dependency origins — qualification pending

The qualified separate dependency probes found 542 files. Read-only inventory
review independently matched all 67 application source/header records to the
captured original request, plus the one captured generated forced header. The
other source groups contain 182 codal-core, 178 ASF4, 44 codal-samd, 25
samd-peripherals, eight board files and one root toolchain header. The 36 compiler
headers remain a separate toolchain review. These are source-location groups,
not inferred component licences or retained-image byte counts.

This collector requires the six exact reviewed component commits and manifest
URLs, then compares every remaining source dependency against its exact regular
Git blob. The longest pinned repository prefix selects nested submodule owners.
Missing, modified, symlink, ambiguous or unbound source fails explicitly. Original
request and captured generated-header bytes are checked separately. Toolchain
records remain explicitly unreviewed; no directory name or vendor-use phrase
grants permission. The result binds to the diagnostic report/request/manifest
hashes and preserves source URLs, commits and Git blob identities without copying
implementation bodies into the artifact.

Acceptance requires actual hosted capture and all enabled exact-head checks,
then a separate artifact/graph/blob-binding audit. Preserve the first failure if
any dependency differs from its claimed pin. Component notices, generated-header
derivation, toolchain obligations, assembly and retained-inline attribution still
need review. Hash-only source-byte access is part of this integration's access
record: it is not an isolated implementation or whole-firmware clean-room claim.
No original firmware execution/disassembly, app pin, admission, hardware ACK,
performance floor or store-clearance claim is added here.
