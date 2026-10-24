# Optional matching Linux driver preparation

`scripts/prepare-rv1103-modules.py` builds the unchanged upstream Linux6.18.56
RK3x I2C driver against the exact pinned Alpine ARMv7 headers and symbol versions,
and extracts matching stock `i2c-dev`. This is an optional host preparation step.
The base Linux fixture, its manifest and the strict guest loader remain unchanged.
A later deliberate basket profile must consume these modules; the base loader
does not accept this module manifest as a boot manifest.

```sh
python3 scripts/prepare-rv1103-modules.py \
  --cache /tmp/rv1103-apk-cache \
  --output /tmp/rv1103-linux-modules
```

The output must be a fresh directory with no symlink parents. `--offline` forbids
downloads and rechecks the hash-addressed cache. The source cache name is
`<source-sha256>.c`; `--source /path/to/i2c-rk3x.c` instead rechecks an existing
source. Defaults are native `cc` and `arm-none-eabi-`; `--hostcc` and
`--cross-compile` select installed tools whose paths, versions and executable
hashes are recorded. Python uses only its standard library. GNU make, native C
compiler and ARM GCC/binutils are prerequisites; no guest or Rust build runs here.

The utility bounds compressed downloads, the complete gzip expansion, physical
TAR records including hidden PAX/GNU metadata, and semantic archive payloads.
Every archive name is checked for traversal and canonical duplicates. It extracts
only the header tree and package identity, with lexically safe relative header
links and no symlink parents, link chains, hardlinks or special files. The genuine
absolute `lib/modules/6.18.56-0-lts/build` package link is outside the whitelist and
is never extracted. Safe dangling DTC links inside the header tree are preserved.
Link targets are checked one component at a time before handling a later `..`;
lexically cancelling a symlink component cannot conceal an extraction escape.

The original tree is staged read-only and hashed before/after both builds. Each
independent disposable copy rebuilds `fixdep`, `genksyms`, `modpost` and
`recordmcount` from the included matching sources using the native compiler.
Extracted ARM ELF host utilities receive no execute permission. Rebuilt helpers
must be actual native host ELF before make can use them. The generated headers,
config and `Module.symvers` also receive before/after checks in each build copy.
Make runs with `-j1`, bounded process-group timeouts, bounded logs and disk checks.
Tool identification uses the same owned process-group runner with a ten-second
timeout and a 64KiB version-output limit. Owned descendants are stopped even when
a version wrapper exits normally; version bytes, hashes and commands are recorded.
Failures leave the requested output absent and include a bounded diagnostic tail;
only the utility's own disposable directories and child processes are cleaned up.

The stock config is preserved byte-for-byte. The dev config is preserved separately;
the only allowed difference is the exact observed line:

```text
stock:   CONFIG_CC_VERSION_TEXT="cc (Alpine 15.2.0) 15.2.0"
headers: CONFIG_CC_VERSION_TEXT="gcc (Alpine 15.2.0) 15.2.0"
```

Both original configs enable modversions, basic modversions, GCC plugins and
`RANDSTRUCT_NONE`; `MODULE_SIG_FORCE` is unset. The build explicitly uses
`CONFIG_GCC_PLUGINS=` because matching native plugins are unavailable. The verified
local build uses ARM GCC13.2.1 rather than stock Alpine GCC15.2.0, and native
GCC13.3.0. GCC13 ignores GCC15 `counted_by` attributes. These compiler/plugin
hardening differences are recorded, with the original config unchanged. Matching
vermagic and CRCs do not prove equivalent compiler hardening or guest acceptance.

The receipt verifies actual ELF32 little-endian ARM EABI5 relocatable bytes, empty
dependencies, module names, licenses and every imported CRC from ELF `__versions`
against matching original `Module.symvers`. RK3x must have exactly 48 imports.
Both modules have exact vermagic, including its final space:

```text
'6.18.56-0-lts SMP mod_unload modversions ARMv7 p2v8 '
```

Stock `i2c-dev` declares `GPL` and contains its original module signature. External
RK3x declares `GPL v2` and is unsigned. Actual guest loading must use ordinary
strict `insmod` for both modules, with no force/version/CRC/symbol bypass or
signature-enforcement changes. The unsigned out-of-tree module may taint the
kernel. Actual loader exits, taint, adapter registration and transactions are
not preparation evidence: this preparation receipt remains `NOT_RUN`. The later
[accepted native joint basket](BASKET.md) records ordinary strict insertion and
actual shared I2C3 transactions separately, without changing the preparation
receipt or bypassing these ABI guards.

The output contains the two `.ko` files, `modules.cpio`, both build logs, and
`module-manifest.json`. The sorted deterministic newc payload contains only fixed
directory entries and `lib/modules/6.18.56-0-lts/{i2c-dev,i2c-rk3x}.ko`; it has no
headers, source, PID1 or userspace helper. Fixed build identity and debug/file/macro
prefix maps are used for two independent builds. A verified receipt requires
byte-identical modules and native helpers, and explicitly records
`guest_status: NOT_RUN`. Build logs and receipt paths themselves need not have
identical bytes because they retain the actual temporary command paths.

| Pinned input | Bytes | SHA256 |
|---|---:|---|
| linux-lts-dev6.18.56-r0 APK | 18028271 | `a2fd3f3a675e5e1c68b4dbea544fa7fe3e1ebbc0403bf1f8de6130ed6cdb1b80` |
| unchanged upstream i2c-rk3x.c | 37582 | `cee9b45dfee3fb4a7168585603c88df5e3422493425ac1bd35046d509d910423` |
| original dev .config | 269494 | `4f33c7f3043a3d3e49794bd31aa4f7dce39951f2919e64ea11dfc1fbed0bcd17` |
| original Module.symvers | 1382573 | `1db6b87e8f410f2b865e5fb37700c614d13f96852edd7dcfb8f952a2b96c03cb` |
| stock i2c-dev.ko.gz | 8580 | `d8287b5b3234089627e4c250b00df77da366b2405767187b01b0d6f54057918d` |
| stock i2c-dev.ko | 21719 | `6381d4a9b257a836ce7f14a80ad1386ae5471e82a0b23715c848ba5362c159aa` |

The actual two-directory local preparation on 12Oct2026 produced RK3x module
300952 bytes, SHA256
`1b3404627ffae4a66d0d126bc14c0fee76127609455c52d44137d7db82257efa`,
and cpio323580 bytes, SHA256
`e6cf5e66657e4d3b252fbd1f9f0c83193784c456f412c135c1c2b81c61ac6750`.
These identify that verified host build, not arbitrary compatible modules or
the earlier ad-hoc experiment. Downloaded inputs, generated headers and binaries
stay outside the repository for fresh preparation. The exact reviewed historical
module receipt/payload is also retained in [basket/reviewed-modules/](basket/reviewed-modules/README.md),
with its historical build provenance preserved. This host preparation receipt
still records guest insertion `NOT_RUN`; later [native joint acceptance](BASKET.md)
records successful guest insertion and shared-circuit evidence separately.
Physical hardware remains unavailable.

Run the focused preparation guards (including the unchanged base tests):

```sh
python3 -m unittest discover -s scripts/tests -p 'test_prepare_rv1103*.py' -v
```
