#!/usr/bin/env python3
"""Score a paired deformation A/B.

Third stage of the deformation A/B pipeline — see `split-ab.py` for the whole
sequence.

    python3 score-ab.py <manifest.json> <answers dir>

Every file in the answers directory is one solver arm, named
`<experiment>-<arm>-<model>.json` and holding {"<file>": "<answer>"}.

Reports three things, in increasing order of how much they should be believed:

  totals      solves and characters per condition — the headline, and the
              weakest evidence, since it still mixes per-string difficulty in
  by cell     the same split by difficulty, to see whether an effect lives in
              one band rather than being spread evenly
  flips       the within-text comparison the pairing exists for: for each
              solution text, was it solved with the deformation, without it,
              both, or neither. Symmetric flip counts mean the deformation is
              moving individual solves in both directions, which is noise.
"""
import json
import sys
from collections import Counter
from math import comb
from pathlib import Path

manifest_path, answers_dir = Path(sys.argv[1]), Path(sys.argv[2])
man = {row["file"]: row for row in json.load(open(manifest_path))}
field = man[next(iter(man))]["field"]

arms = {}
for path in sorted(answers_dir.glob("*.json")):
    arms[path.stem] = json.load(open(path))

# One row per (arm, file) attempt.
attempts = []
for arm, answers in arms.items():
    for file, guess in answers.items():
        row = man.get(file)
        if row is None:
            continue
        attempts.append(
            {
                "arm": arm,
                "model": arm.split("-")[-1],
                "condition": row["condition"],
                "difficulty": row["difficulty"],
                "pair": row["pair"],
                "solution": row["solution"],
                "solved": guess == row["solution"],
                "chars": sum(1 for a, b in zip(row["solution"], guess) if a == b),
                "total": len(row["solution"]),
            }
        )


def summarise(label, rows):
    if not rows:
        return
    solved = sum(1 for r in rows if r["solved"])
    chars = sum(r["chars"] for r in rows)
    total = sum(r["total"] for r in rows)
    print(
        f"{label:<34}{solved:>3}/{len(rows):<3} solved   "
        f"chars {chars:>3}/{total} ({100 * chars / total:>3.0f}%)"
    )


print(f"\npaired A/B for `{field}`, {len(attempts)} attempts "
      f"across {len(arms)} arms\n")

for condition in ("off", "on"):
    summarise(f"{field} {condition}", [r for r in attempts if r["condition"] == condition])
print()
for difficulty in sorted({r["difficulty"] for r in attempts}):
    for condition in ("off", "on"):
        summarise(
            f"  difficulty {difficulty}, {field} {condition}",
            [
                r
                for r in attempts
                if r["difficulty"] == difficulty and r["condition"] == condition
            ],
        )
print()
for model in sorted({r["model"] for r in attempts}):
    for condition in ("off", "on"):
        summarise(
            f"  {model}, {field} {condition}",
            [r for r in attempts if r["model"] == model and r["condition"] == condition],
        )

# The within-text comparison. Keyed on (pair, model) so a text is compared
# against itself as seen by the same model — comparing Opus-with against
# Sonnet-without would put the model difference back into the measurement the
# pairing exists to remove.
outcomes = {}
for row in attempts:
    outcomes.setdefault((row["pair"], row["model"]), {})[row["condition"]] = row["solved"]

flips = Counter()
for verdicts in outcomes.values():
    if len(verdicts) < 2:
        continue
    off, on = verdicts["off"], verdicts["on"]
    flips["both" if off and on else "neither" if not (off or on) else "lost" if off else "gained"] += 1

print(f"\nwithin-text flips, {sum(flips.values())} paired comparisons:")
print(f"  solved without {field}, lost with it   {flips['lost']:>3}")
print(f"  solved with {field}, missed without    {flips['gained']:>3}")
print(f"  solved both ways                       {flips['both']:>3}")
print(f"  solved neither way                     {flips['neither']:>3}")
net = flips["lost"] - flips["gained"]
print(
    f"\nnet: {field} cost the solver {net:+d} solves out of "
    f"{flips['lost'] + flips['gained']} that went either way."
)

# McNemar's exact test on the flip counts, per difficulty as well as overall.
# Only the discordant pairs carry information: under the null that the
# deformation does nothing, each flip is a coin toss, so the two-sided p is the
# binomial tail.
def mcnemar(lost, gained):
    n = lost + gained
    if n == 0:
        return 1.0
    k = max(lost, gained)
    tail = sum(comb(n, i) for i in range(k, n + 1)) / 2 ** n
    return min(1.0, 2 * tail)

per_difficulty = {}
for row in attempts:
    key = (row["difficulty"], row["pair"], row["model"])
    per_difficulty.setdefault(key, {})[row["condition"]] = row["solved"]

print("\nflips by difficulty (lost / gained / p):")
buckets = {}
for (difficulty, _, _), verdicts in per_difficulty.items():
    if len(verdicts) < 2:
        continue
    counts = buckets.setdefault(difficulty, Counter())
    if verdicts["off"] and not verdicts["on"]:
        counts["lost"] += 1
    elif verdicts["on"] and not verdicts["off"]:
        counts["gained"] += 1
for difficulty in sorted(buckets):
    counts = buckets[difficulty]
    print(
        f"  difficulty {difficulty}: {counts['lost']} lost, {counts['gained']} gained, "
        f"p = {mcnemar(counts['lost'], counts['gained']):.3f}"
    )
print(f"  overall:      {flips['lost']} lost, {flips['gained']} gained, "
      f"p = {mcnemar(flips['lost'], flips['gained']):.3f}")
print(
    "\nCaveat: the 36 comparisons are 18 texts seen by 2 models, and the models\n"
    "are correlated, so the effective sample is nearer 18 than 36 and these p\n"
    "values are optimistic."
)
