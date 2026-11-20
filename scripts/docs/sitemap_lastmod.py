"""Remove fabricated build-day sitemap timestamps from generated MkDocs docs.

Until a source-file revision map exists, omitting <lastmod> is more accurate
than declaring every docs page changed during each deployment. This hook
runs in both mkdocs build and mkdocs gh-deploy, after sitemap generation.
"""

from pathlib import Path
import os
import re

_LASTMOD_LINE = re.compile(r"^[ \t]*<lastmod>[^<]*</lastmod>\r?\n", re.MULTILINE)


def on_post_build(config, **kwargs):
    """Keep URL discovery but remove build-time-only freshness claims."""
    sitemap = Path(config["site_dir"]) / "sitemap.xml"
    # A missing generated sitemap is a real publication error. Do not hide it.
    original = sitemap.read_text(encoding="utf-8")
    cleaned, count = _LASTMOD_LINE.subn("", original)
    if count == 0:
        return
    tmp = sitemap.with_name(f"{sitemap.name}.{os.getpid()}.tmp")
    try:
        tmp.write_text(cleaned, encoding="utf-8")
        os.replace(tmp, sitemap)
    finally:
        tmp.unlink(missing_ok=True)
