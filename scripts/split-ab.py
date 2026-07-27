#!/usr/bin/env python3
"""Crossover split of a paired A/B set into two blind arms.

Second stage of the deformation A/B pipeline. Generate the paired set with the
`deformation_ab_set` test, split it here, hand each arm to a solver, then score
with `score-ab.py`:

    CAPTCHA_SAMPLE_DIR=/tmp/ab-rotation CAPTCHA_AB_FIELD=rotation \\
      CAPTCHA_AB_LEVELS=3,5 \\
      cargo test --release --lib deformation_ab_set -- --ignored
    python3 scripts/split-ab.py /tmp/ab-rotation /tmp/blind-rotation
    # ... solve /tmp/blind-rotation-A and -B, one JSON file per arm ...
    python3 scripts/score-ab.py /tmp/ab-rotation/manifest.json <answers dir>

Usage:

    python3 split-ab.py <source dir> <dest prefix>

Each pair contributes its "off" image to one arm and its "on" image to the
other, alternating, so every arm sees each solution text exactly once and gets
half of each condition. That is what makes the comparison within-subject: the
per-string difficulty that dominates an unpaired grid cancels.

The arm directories get images and a lengths file only. No solution, no
condition, no manifest — a solver working in one of them has nothing to read
an answer off.
"""
import json
import random
import shutil
import sys
from pathlib import Path

source, prefix = Path(sys.argv[1]), sys.argv[2]
manifest = json.load(open(source / "manifest.json"))

by_pair = {}
for row in manifest:
    by_pair.setdefault(row["pair"], {})[row["condition"]] = row

# Seeded, so the split is reproducible and a re-run scores the same images.
rng = random.Random(20260727)

arms = {"A": [], "B": []}
for index, pair in enumerate(sorted(by_pair)):
    first, second = ("off", "on") if index % 2 == 0 else ("on", "off")
    arms["A"].append(by_pair[pair][first])
    arms["B"].append(by_pair[pair][second])

for name, rows in arms.items():
    rng.shuffle(rows)
    out = Path(f"{prefix}-{name}")
    if out.exists():
        shutil.rmtree(out)
    out.mkdir(parents=True)
    for row in rows:
        shutil.copy(source / row["file"], out / row["file"])
    json.dump(
        {row["file"]: row["length"] for row in rows},
        open(out / "lengths.json", "w"),
        indent=1,
        sort_keys=True,
    )
    texts = {row["solution"] for row in rows}
    conditions = [row["condition"] for row in rows]
    print(
        f"{out}: {len(rows)} images, {len(texts)} distinct texts, "
        f"{conditions.count('off')} off / {conditions.count('on')} on"
    )
    assert len(texts) == len(rows), "an arm must not show the same text twice"
