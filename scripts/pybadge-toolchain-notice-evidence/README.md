# Exact small source-packaging notices — qualification pending

[PR185's metadata capture](https://github.com/CrispStrobe/labwired-core/pull/185#issuecomment-6066200720)
binds three installed header-owner binaries to exact source descriptors. In
particular, the libstdc++ binary's combined version maps to source version `26`;
substituting that combined version as a source version would be incorrect.

This standalone hosted job admits the exact prior run/artifact/hash, then
downloads only three descriptor-pinned small packaging archives: GCC Debian
packaging (19,812 bytes), libstdc++ source packaging (4,864 bytes) and newlib Debian
packaging (13,736 bytes). The roughly 105 MB GCC original and 9 MB newlib original
archives are not acquired. No compiler, firmware build or guest runs.

Archive paths, duplicates, member types, counts, compressed/member/notice sizes
and whole-archive hashes are checked. XZ decoding has a 64 MiB decoder-memory
limit and a 4 MiB expanded-output bound before TAR metadata parsing; truncated
and trailing streams fail. Only regular notice members are read and
captured. Other members are listed, not extracted, inspected or executed. Raw
archives are not uploaded; their acquisition/hash and extraction correspondence
remain hosted observations. Separate audit must verify captured raw notice
hashes, input graph/pins and explicit missing names without pretending it has
independently replayed extraction from absent raw archives.

The artifact retains exact notice bytes, archive/member identities and any SHA
matches to the earlier installed package copyright captures. Matching bytes are
not source-to-binary proof or legal clearance. `COPYING3`, `COPYING.RUNTIME` and
`copyright-gcc` absence remains explicit per selected archive; another copyright
file or similar exception wording does not silently satisfy a missing name.

Acceptance requires the first actual hosted result, separate bounded artifact
audit and all enabled exact-head checks. Preserve original failures and unknowns.
This is a distinct directory/workflow: no earlier firmware-build or metadata job
is repeated just to collect these notices. Original reference classifications,
app pins, captures, ACKs and performance gates stay unchanged. Exact exception
correspondence, header/assembly/retained-inline provenance and obligations remain
open. Archive-byte access is recorded; no isolated implementation, whole-image
admission, store approval or new RTx result is claimed.
