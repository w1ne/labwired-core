# Pinned GCC original-source notice candidates — qualification pending

[PR185](https://github.com/CrispStrobe/labwired-core/pull/185#issuecomment-6066200720)
binds the exact installed header-owner packages to source descriptors.
[PR186's small packaging capture](https://github.com/CrispStrobe/labwired-core/pull/186#issuecomment-6066618120)
matches all three installed copyright captures, but supplies none of the named
`COPYING3`, `COPYING.RUNTIME` or `copyright-gcc` references. Packaging notices
alone do not close the GCC source/exception chain.

This distinct hosted workflow uses the qualified metadata artifact's exact
104,971,782-byte GCC original archive and SHA256. It verifies the whole compressed
download before parsing, then streams BZip2/TAR reads with a 2 GiB expanded-read
bound, member/path/count limits and bounded selected notice extraction. The full
expanded source tree is not buffered. Only exact `COPYING`, `COPYING3`,
`COPYING3.LIB` and `COPYING.RUNTIME` basename candidates are captured. Named
links/nonregular entries are recorded unresolved and never followed.

The raw archive stays outside the uploaded evidence directory. No implementation
member is separately extracted, inspected as implementation or executed. Whole
archive acquisition and streaming necessarily access implementation payload
bytes: this is not isolated/clean-room work or avoidance of source-byte access.
No compiler, firmware rebuild, guest or app artifact is involved. Neither earlier
collector is retriggered by this separate workflow/directory.

Acceptance requires the first actual hosted artifact, separate raw notice/input
graph/hash audit and every enabled final-head check. Raw source archive and full
member inventory are not uploaded; their hashes, scanning and extraction remain
hosted observations, not independent archive-body replay. The reader's count at
TAR stop is not a claim about every trailing decompressed byte.

Named-file presence alone does not establish exact header correspondence,
exception applicability, source-to-binary proof or completed obligations.
Preserve missing names, aliases, original failures and unknowns. Historical image
admission, app pins, captures, ACKs and performance gates stay unchanged. No
legal/store clearance, native-runtime execution or new RTx result is claimed.
