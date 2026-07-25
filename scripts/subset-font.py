#!/usr/bin/env python3
"""Regenerate the CAPTCHA glyph font embedded in the binary.

The renderer only ever draws the 54 characters in `BASIC_CHAR`
(src/services/captcha/generator.rs), so shipping a full font face wastes
~400 KB in every binary and container image. This script cuts Liberation
Sans Bold down to exactly those glyphs.

Subsetting produces a "Modified Version" under the SIL Open Font License,
which forbids Modified Versions from carrying a Reserved Font Name. The
face is therefore renamed away from "Liberation" (and from Arimo/Tinos/
Cousine, reserved by the same copyright holders). Copyright, license and
license-URL records are preserved verbatim, as the OFL requires.

IMPORTANT: the subset locks the character set. If BASIC_CHAR gains a
character, rerun this script or that glyph renders as .notdef. The
`test_every_basic_char_has_a_glyph` test in generator.rs guards against
forgetting.

Usage:
    pip install fonttools
    # Source: https://github.com/liberationfonts/liberation-fonts/releases
    # (or a distro package, e.g. /usr/share/fonts/truetype/liberation/)
    python3 scripts/subset-font.py path/to/LiberationSans-Bold.ttf
"""

from __future__ import annotations

import pathlib
import sys

from fontTools import subset
from fontTools.ttLib import TTFont

# Must mirror BASIC_CHAR in src/services/captcha/generator.rs.
BASIC_CHAR = (
    "23456789"
    "ABCDEFGHJKMNPQRSTUVWXYZ"
    "abcdefghjkmnpqrstuvwxyz"
)

# Renaming is mandatory: "Liberation" is a Reserved Font Name under the OFL.
FAMILY = "CaptchAPI Glyphs"
SUBFAMILY = "Bold"
FULL_NAME = f"{FAMILY} {SUBFAMILY}"
POSTSCRIPT_NAME = "CaptchAPIGlyphs-Bold"
VERSION = "Version 2.1.5"

# The OFL FAQ permits acknowledging the original in the description record
# (name ID 10); the Reserved Font Name must stay out of the naming records
# proper (IDs 1, 3, 4, 5, 6, 16, 17).
DESCRIPTION = (
    "Subset of Liberation Sans Bold 2.1.5 containing only the glyphs CaptchAPI "
    "renders. Renamed as required by the SIL Open Font License. Not affiliated "
    "with or endorsed by the Liberation Fonts project or its copyright holders."
)

OUT_PATH = pathlib.Path(__file__).resolve().parent.parent / "assets" / "fonts" / "CaptchAPIGlyphs-Bold.ttf"

# name IDs that must lose the Reserved Font Name. IDs 0 (copyright),
# 13 (license) and 14 (license URL) are deliberately left untouched.
NAME_OVERRIDES = {
    1: FAMILY,
    2: SUBFAMILY,
    3: f"{POSTSCRIPT_NAME};2.1.5",
    4: FULL_NAME,
    5: VERSION,
    6: POSTSCRIPT_NAME,
    10: DESCRIPTION,
    16: FAMILY,
    17: SUBFAMILY,
}

# fontTools keeps only IDs 0-6 by default, which would strip the OFL notice
# out of the face. Keep copyright, description, license and license URL.
KEEP_NAME_IDS = [0, 1, 2, 3, 4, 5, 6, 10, 13, 14, 16, 17]

# Layout, hinting and metadata tables the rasteriser never reads.
DROP_TABLES = ["DSIG", "LTSH", "VDMX", "PCLT", "hdmx", "kern", "GSUB", "GDEF", "JSTF", "gasp", "FFTM"]


def main() -> int:
    if len(sys.argv) != 2:
        print(__doc__, file=sys.stderr)
        return 2

    source = pathlib.Path(sys.argv[1])
    if not source.is_file():
        print(f"error: no such font: {source}", file=sys.stderr)
        return 1

    font = TTFont(source)

    options = subset.Options()
    options.hinting = False
    options.desubroutinize = True
    options.drop_tables += DROP_TABLES
    options.notdef_outline = True
    options.recalc_bounds = True
    options.name_IDs = KEEP_NAME_IDS
    options.name_legacy = True

    subsetter = subset.Subsetter(options=options)
    subsetter.populate(text=BASIC_CHAR)
    subsetter.subset(font)

    name_table = font["name"]
    for name_id, value in NAME_OVERRIDES.items():
        name_table.setName(value, name_id, 3, 1, 0x409)  # Windows / Unicode BMP / en-US
        name_table.setName(value, name_id, 1, 0, 0)      # Macintosh / Roman / English
    # Trademark: the Liberation trademark does not carry over to this subset.
    name_table.removeNames(nameID=7)

    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    font.save(OUT_PATH)

    before = source.stat().st_size
    after = OUT_PATH.stat().st_size
    print(f"{source.name}: {before:,} bytes")
    print(f"{OUT_PATH.name}: {after:,} bytes ({len(BASIC_CHAR)} glyphs, -{before - after:,})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
