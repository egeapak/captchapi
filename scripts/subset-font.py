#!/usr/bin/env python3
"""Regenerate the CAPTCHA glyph font embedded in the binary.

The renderer only ever draws the 54 characters in `BASIC_CHAR`
(src/services/captcha/generator.rs), so shipping a full font face wastes
tens of KB in every binary and container image. This script cuts Roboto
Bold down to exactly those glyphs.

Roboto is licensed under the SIL Open Font License 1.1 and declares no
Reserved Font Name, so a subset may keep the family name. The copyright,
license and license-URL records are preserved in the face, and the
modification is disclosed in the description record.

IMPORTANT: the subset locks the character set. If BASIC_CHAR gains a
character, rerun this script or that glyph renders as .notdef. The
`test_every_basic_char_has_a_glyph` test in generator.rs guards against
forgetting.

Usage:
    pip install fonttools
    # Source: https://github.com/googlefonts/roboto-classic
    # or the TTF Google Fonts serves for `family=Roboto:700`.
    python3 scripts/subset-font.py path/to/Roboto-Bold.ttf
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

OUT_PATH = (
    pathlib.Path(__file__).resolve().parent.parent
    / "assets"
    / "fonts"
    / "Roboto-Bold-subset.ttf"
)

COPYRIGHT = (
    "Copyright 2011 The Roboto Project Authors "
    "(https://github.com/googlefonts/roboto-classic)"
)
LICENSE = "This Font Software is licensed under the SIL Open Font License, Version 1.1."
LICENSE_URL = "https://openfontlicense.org"
DESCRIPTION = (
    f"Roboto Bold subset to the {len(BASIC_CHAR)} characters CaptchAPI renders. "
    "Modified by the CaptchAPI project; hinting and unused layout tables removed. "
    "Roboto declares no Reserved Font Name, so the family name is retained."
)

# Records to (re)write. Google's CDN build omits the license records, so they
# are restored explicitly rather than assumed present.
NAME_OVERRIDES = {
    0: COPYRIGHT,
    10: DESCRIPTION,
    13: LICENSE,
    14: LICENSE_URL,
}

# fontTools keeps only IDs 0-6 by default, which would drop the notice records.
KEEP_NAME_IDS = [0, 1, 2, 3, 4, 5, 6, 10, 13, 14, 16, 17]

# Layout, hinting and metadata tables the rasteriser never reads.
DROP_TABLES = [
    "DSIG", "LTSH", "VDMX", "PCLT", "hdmx", "kern",
    "GSUB", "GDEF", "GPOS", "JSTF", "gasp", "FFTM",
]


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
    # The Apache-era trademark record does not apply to this subset.
    name_table.removeNames(nameID=7)

    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    font.save(OUT_PATH)

    before = source.stat().st_size
    after = OUT_PATH.stat().st_size
    print(f"{source.name}: {before:,} bytes")
    print(f"{OUT_PATH.name}: {after:,} bytes ({len(BASIC_CHAR)} glyphs)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
