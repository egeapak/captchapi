# Tuning length and difficulty

The defaults are **`length: 6`, `difficulty: 5`**. Both were set by measuring how often frontier
vision models actually solve the images this service serves — Opus 5 and Sonnet 5, given the
character set, the exact solution length, a description of every deformation, and (for the
tool-equipped arms) python, Pillow, numpy and the bundled font to template-match against. That is
a maximally advantaged attacker, which is the right one to design against.

## Length is the sharper knob, and it is not close

Pooled over difficulty 5–8 and every arm:

| length | solved | per-character accuracy |
|---|---|---|
| 4 | 8/40 = 20% | 58% |
| 5 | 8/40 = 20% | 64% |
| **6 (default)** | **0/40 = 0%** — 95% CI [0%, 8.8%] | 44% |

The mechanism is arithmetic: solving requires *every* character, so the solve rate is roughly the
per-character accuracy raised to the length. At 44–64% per character, `0.64^5` is about 11% and
`0.44^6` is under 1%. Each extra character multiplies the attacker's problem.

It is also nearly free — length 3 to 12 moves the median render from 6.6 ms to 7.9 ms, so 5 to 6
costs about 2% — and it is far gentler on a human than cranking difficulty: a longer string of
legible letters beats a shorter string of mangled ones.

## Difficulty is the blunter knob

Past the default it mostly costs legibility:

| level | solved | per-character accuracy |
|---|---|---|
| 5 (default) | 7/84 = 8.3% — 95% CI [4.1%, 16.2%] | 66% |
| 6 | 6/24 = 25% | 63% |
| 7 | 2/24 = 8.3% | 38% |
| 8 | 1/24 = 4.2% | 37% |

**Read the character column, not the solve column.** The solve counts rest on 24 attempts per
level, are not even monotonic, and cannot be at that sample size; the character rate rests on 120
per level and falls cleanly. Levels 5 and 8 have almost completely overlapping intervals, so
choosing 8 buys an unmeasurable amount of safety for **20% more render time and 20% more stored
bytes**, plus images a human finds materially harder.

The practical consequence: **the difficulty dial cannot be tuned on this evidence between 5 and
8.** If a deployment needs a demonstrably lower solve rate, the lever is length.

## Practical guidance

| you want | do this |
|---|---|
| more resistance | raise `length` to 7+ before touching `difficulty` |
| accessibility | drop `difficulty` to 2–3; those levels stay clearly legible |
| smaller images | lower `difficulty`, not `length` — noise dominates the encoded size |
| a short input field | lower `length`, and accept the measured cost above |

## What holds the rate is the rendering

Ten deformations are drawn independently **per letter** — jitter, scale, skew, wave, rotation,
clustering, outline, transparency, gradient and blur — so no single rule describes a whole
solution.

Two of them specifically defeated the image-processing attack that used to work. `gradient` ramps
hue and lightness across a *single* letter, and the blur is applied **after** compositing, so it
smears each letter into its neighbour rather than merely softening its edges. Hue-band splitting
was the tooled attack's entire segmentation strategy — every glyph used to have one random hue, so
isolating a hue band isolated a letter.

Measured at difficulty 5, tools no longer help:

| arm | solved | chars | cost |
|---|---|---|---|
| vision Opus | 1/12 | 65% | 49k tokens, 22 calls |
| vision Sonnet | 3/12 | 67% | 52k tokens, 16 calls |
| tooled Opus | 3/12 | 73% | 319k tokens, 138 calls, 39 min |
| tooled Sonnet | 0/12 | 60% | 255k tokens, 238 calls, 32 min |

Paired per image and per model: tools won 3, looking won 4, neither solved 17 — **p = 1.0, no
effect**, at 5–6× the token cost. On the previous renderer, tooling was decisive at this level and
is what forced the default difficulty up to 8 in the first place.

## Caveats worth stating

These figures rest on tens of attempts, not thousands; treat directions as reliable and
magnitudes as indicative. `0/40` is an upper bound of 8.8%, not a guarantee of zero — two
individual sets came back 0/18 and calling that a floor was a mistake: a zero on 18 attempts has a
95% upper bound near 18% by itself, and a third set of fresh images then drew 4/24. Pool the sets;
do not quote the lucky cell.

Every arm recovers 44–66% of individual characters, so many failures are near-misses that
case-sensitive validation converts into failed solves — **the margin is thinner than the solve
rate suggests.** Case sensitivity is free and worth keeping, but it is not what holds a difficulty
level; the rendering is.

## Reproducing

Full methodology, the per-deformation A/B pipeline, and the measurement history of every
individual deformation live in [`.claude/CLAUDE.md`](../.claude/CLAUDE.md).

```bash
# Render a difficulty × length grid
cargo run --example challenge_set

# Paired A/B for a single deformation
CAPTCHA_SAMPLE_DIR=/tmp/ab CAPTCHA_AB_FIELD=rotation CAPTCHA_AB_LEVELS=3,5 \
  cargo test --release --lib deformation_ab_set -- --ignored
python3 scripts/split-ab.py /tmp/ab /tmp/blind
python3 scripts/score-ab.py /tmp/ab/manifest.json <answers dir>
```

Use the paired A/B pipeline to evaluate a single deformation — it renders the *same* solution text
under both conditions so per-string difficulty cancels, and an unpaired comparison spends most of
its statistical power on whether one set of random strings happened to be harder. Read the **flip
counts**, not the totals: a deformation that flips as many solves on as off is noise however the
totals fall.
