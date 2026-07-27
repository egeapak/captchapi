#!/usr/bin/env python3
"""Score solver arms against a `challenge_set` manifest.

    python3 score-arms.py <manifest.json> <answers dir>

Every `*.json` in the answers directory is one arm, named `<label>-<model>` and
holding `{"<file>": "<answer>"}`. Complements `score-ab.py`, which handles the
paired two-condition sets `deformation_ab_set` writes; this one handles a plain
challenge set where the arms differ rather than the images.

Grading is case-sensitive because `POST /sessions/{id}/validate` is. A
"right letters, wrong case" answer is a failed solve for an attacker, so it is a
failed solve here — but it is counted separately as well, because that column is
what tells you whether case sensitivity is still buying anything.

If arm labels contain `tooled` and `vision`, the arms are also compared per
image and per model, which is the comparison that matters when the question is
whether image-processing tooling beats looking.
"""
import json
import sys
from collections import Counter
from math import comb
from pathlib import Path

manifest_path, answers_dir = Path(sys.argv[1]), Path(sys.argv[2])
man = {row["file"]: row for row in json.load(open(manifest_path))}
arms = {p.stem: json.load(open(p)) for p in sorted(Path(answers_dir).glob("*.json"))}
if not arms:
    sys.exit(f"no arm files in {answers_dir}")

rows = []
for arm, answers in arms.items():
    for file, guess in answers.items():
        row = man.get(file)
        if row is None:
            continue
        truth = row["solution"]
        rows.append(
            {
                "arm": arm,
                "model": arm.split("-")[-1],
                "kind": "tooled" if "tooled" in arm else "vision",
                "file": file,
                "difficulty": row["difficulty"],
                "length": row["length"],
                "solved": guess == truth,
                "case_only": guess != truth and guess.lower() == truth.lower(),
                "chars": sum(1 for a, b in zip(truth, guess) if a == b),
                "total": len(truth),
            }
        )

print(f"\n{len(rows)} attempts across {len(arms)} arms, "
      f"{len(man)} images at difficulty "
      f"{sorted({r['difficulty'] for r in man.values()})}\n")


def summarise(label, subset, width=30):
    if not subset:
        return
    solved = sum(1 for r in subset if r["solved"])
    chars = sum(r["chars"] for r in subset)
    total = sum(r["total"] for r in subset)
    case_only = sum(1 for r in subset if r["case_only"])
    print(
        f"{label:<{width}}{solved:>3}/{len(subset):<3} solved   "
        f"chars {chars:>3}/{total} ({100 * chars / total:>3.0f}%)   "
        f"case-only misses {case_only}"
    )


for arm in arms:
    summarise(arm, [r for r in rows if r["arm"] == arm])
print()
for kind in ("vision", "tooled"):
    summarise(f"all {kind}", [r for r in rows if r["kind"] == kind])
print()
for length in sorted({r["length"] for r in rows}):
    for kind in ("vision", "tooled"):
        summarise(
            f"  length {length}, {kind}",
            [r for r in rows if r["length"] == length and r["kind"] == kind],
        )

# Per-image, per-model: did tooling flip this image for this model? Keyed on the
# model as well as the image, so a tooled-Opus success is compared against
# vision-Opus on the same picture rather than against a different model.
paired = {}
for row in rows:
    paired.setdefault((row["file"], row["model"]), {})[row["kind"]] = row["solved"]

flips = Counter()
for verdicts in paired.values():
    if len(verdicts) < 2:
        continue
    vision, tooled = verdicts["vision"], verdicts["tooled"]
    flips[
        "both"
        if vision and tooled
        else "neither"
        if not (vision or tooled)
        else "tooling won"
        if tooled
        else "tooling lost"
    ] += 1

if sum(flips.values()):
    def mcnemar(a, b):
        n = a + b
        if n == 0:
            return 1.0
        k = max(a, b)
        return min(1.0, 2 * sum(comb(n, i) for i in range(k, n + 1)) / 2**n)

    print(f"\nper-image, per-model: {sum(flips.values())} paired comparisons")
    print(f"  solved with tools only     {flips['tooling won']:>3}")
    print(f"  solved by looking only     {flips['tooling lost']:>3}")
    print(f"  solved both ways           {flips['both']:>3}")
    print(f"  solved neither way         {flips['neither']:>3}")
    print(
        f"\n  p = {mcnemar(flips['tooling won'], flips['tooling lost']):.3f} "
        "on the discordant pairs"
    )
