#!/usr/bin/env python3
"""Write a deterministic newc cpio archive (Linux initramfs).

usage: mkinitramfs.py OUT.cpio SPEC...

SPEC is one of:
  dir:PATH                     directory, mode 0755
  file:PATH=HOSTFILE[:MODE]    regular file copied from HOSTFILE (octal MODE)
  link:PATH=TARGET             symlink
  chr:PATH=MAJOR,MINOR[:MODE]  character device

All entries get uid/gid 0, mtime 0 and sequential inode numbers in the
order given, so the same inputs always give the same bytes.
"""

import sys

S_IFDIR, S_IFREG, S_IFLNK, S_IFCHR = 0o040000, 0o100000, 0o120000, 0o020000


def entry(out, ino, name, mode, data=b"", rdev=(0, 0)):
    name_b = name.encode() + b"\0"
    fields = [
        ino, mode, 0, 0, 1, 0, len(data), 0, 0, rdev[0], rdev[1], len(name_b), 0,
    ]
    hdr = b"070701" + b"".join(b"%08X" % f for f in fields)
    out.write(hdr + name_b)
    out.write(b"\0" * (-(len(hdr) + len(name_b)) % 4))
    out.write(data)
    out.write(b"\0" * (-len(data) % 4))


def main():
    if len(sys.argv) < 3:
        sys.exit(__doc__)
    with open(sys.argv[1], "wb") as out:
        ino = 1
        for spec in sys.argv[2:]:
            kind, rest = spec.split(":", 1)
            if kind == "dir":
                entry(out, ino, rest.lstrip("/"), S_IFDIR | 0o755)
            elif kind == "file":
                path, src = rest.split("=", 1)
                mode = 0o644
                if ":" in src:
                    src, m = src.rsplit(":", 1)
                    mode = int(m, 8)
                with open(src, "rb") as f:
                    entry(out, ino, path.lstrip("/"), S_IFREG | mode, f.read())
            elif kind == "link":
                path, target = rest.split("=", 1)
                entry(out, ino, path.lstrip("/"), S_IFLNK | 0o777, target.encode())
            elif kind == "chr":
                path, dev = rest.split("=", 1)
                mode = 0o600
                if ":" in dev:
                    dev, m = dev.split(":", 1)
                    mode = int(m, 8)
                major, minor = (int(x) for x in dev.split(","))
                entry(out, ino, path.lstrip("/"), S_IFCHR | mode, rdev=(major, minor))
            else:
                sys.exit(f"bad spec {spec}")
            ino += 1
        entry(out, 0, "TRAILER!!!", 0)


if __name__ == "__main__":
    main()
