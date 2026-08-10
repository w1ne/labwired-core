# Brand assets

Copies of the LabWired logo, checked in so this repository's README renders without
depending on an external host staying up.

**These are copies, not the source of truth.** The canonical press kit — full logo and
mark set, light/dark/mono variants, PNG exports, icon sizes, and the usage rules — lives
at **[labwired.com/brand.html](https://labwired.com/brand.html)**. If a file here
disagrees with the kit, the kit wins; re-download rather than edit in place.

| File | Use |
| --- | --- |
| `labwired-logo.svg` | Full logo, light backgrounds (`#0056b3` / `#14151a`) |
| `labwired-logo-dark.svg` | Full logo, dark backgrounds (`#4d9fff` / `#ffffff`) |
| `labwired-mark.svg` | Mark only, light backgrounds |
| `labwired-logo{,-dark}.png` | 2× PNG renders of the above, used by the README header |
| `social.svg` → `social-preview.png` | GitHub social preview card, 1280×640 |

`social-preview.png` is **generated**, not hand-drawn. Regenerate after editing `social.svg`:

```sh
rsvg-convert -w 1280 -h 640 docs/assets/brand/social.svg -o docs/assets/brand/social-preview.png
```

It is uploaded in **Settings → General → Social preview**; GitHub has no API for that
field, so a change here does not take effect until someone re-uploads it. The terminal
block inside it quotes the real quickstart output — if the quickstart changes, this card
is lying until it is regenerated and re-uploaded.

The wordmark is outlined paths, not live `<text>`, so it renders identically without the
brand typeface installed. Do not re-typeset it.

LabWired is a trademark. The MIT license on this repository covers the code, not the
name or the logo — see [TRADEMARKS](https://labwired.com/brand.html) for what use is
permitted.
