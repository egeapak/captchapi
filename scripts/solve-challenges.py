#!/usr/bin/env python3
"""Measure vision-model solve rates against a generated challenge set.

This is the attacker side of the security claims in `.claude/CLAUDE.md`: it
asks Claude models to read CAPTCHAs the service actually serves, and reports
exact solve rate and per-character accuracy by difficulty.

    cargo run --release --example challenge_set -- /tmp/challenges 3
    pip install anthropic
    export ANTHROPIC_API_KEY=...
    python3 scripts/solve-challenges.py /tmp/challenges

The solver is given every advantage a real attacker has: the character set (the
renderer is open source), the exact solution length, and a description of the
deformations. Measuring against a solver that has to guess the length would
flatter the service for reasons that have nothing to do with the rendering.

Grading is case-sensitive because `POST /sessions/{id}/validate` is. A "correct
letters, wrong case" answer is a failed solve for an attacker, so it is a failed
solve here.
"""

from __future__ import annotations

import argparse
import base64
import json
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

try:
    import anthropic
except ImportError:
    sys.exit("pip install anthropic")

# Model ids are exact — never append a date suffix.
DEFAULT_MODELS = ["claude-haiku-4-5", "claude-sonnet-5", "claude-opus-5"]

# Mirrors BASIC_CHAR in src/services/captcha/generator.rs. Kept here rather than
# parsed out of the Rust so the solver's knowledge is explicit and auditable.
CHARSET = (
    "23456789ABCDEFGHJKMNPQRSTUVWXYZ" "abcdefghjkmnpqrstuvwxyz"
)

PROMPT = """\
This is a CAPTCHA image from an open-source CAPTCHA service. I am the author of \
that service and I am measuring how well vision models defeat it, so read it as \
accurately as you can.

Everything known about the renderer:

- The solution is exactly {length} characters.
- Characters come only from this set (no 0, O, 1, I or l): {charset}
- Case matters: upper and lower case are different characters.
- Letters are individually rotated, skewed, wave-warped, resized and jittered, \
and they overlap each other. Some are drawn as hollow outlines rather than \
filled. Some fade from solid to partly transparent across the letter, and each \
letter's colour ramps between two hues, so no single colour or brightness \
threshold isolates a glyph.
- Two Bezier curves, two hollow circles, gaussian noise and colour speckle are \
drawn over the top. None of those are part of the solution.

Reply with the {length} characters only."""

SCHEMA = {
    "type": "object",
    "properties": {"solution": {"type": "string"}},
    "required": ["solution"],
    "additionalProperties": False,
}


def ask(client: anthropic.Anthropic, model: str, image: bytes, length: int, hint: bool) -> str:
    """One attempt. Returns the answer, or "REFUSED"/"ERROR: ..." verbatim."""
    prompt = PROMPT.format(
        length=length if hint else "an unknown number of", charset=CHARSET
    )

    request = {
        "model": model,
        "max_tokens": 8192,
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/jpeg",
                            "data": base64.standard_b64encode(image).decode(),
                        },
                    },
                    {"type": "text", "text": prompt},
                ],
            }
        ],
        "output_config": {"format": {"type": "json_schema", "schema": SCHEMA}},
    }

    # Haiku 4.5 predates adaptive thinking and rejects `effort`; the Claude 5
    # models think by default, and reading a warped glyph benefits from it.
    if model != "claude-haiku-4-5":
        request["thinking"] = {"type": "adaptive"}
        request["output_config"]["effort"] = "high"

    try:
        response = client.messages.create(**request)
    except anthropic.APIStatusError as error:
        return f"ERROR: {error.status_code} {error.message[:60]}"
    except anthropic.APIConnectionError:
        return "ERROR: connection"

    # A safety classifier may decline; that is not a wrong answer and must not
    # be scored as one. Check before touching content — on a refusal it is empty.
    if response.stop_reason == "refusal":
        return "REFUSED"
    if response.stop_reason == "max_tokens":
        return "ERROR: truncated"

    text = next((b.text for b in response.content if b.type == "text"), "")
    try:
        return json.loads(text)["solution"].strip()
    except (json.JSONDecodeError, KeyError, TypeError):
        return text.strip()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path, help="output of `challenge_set`")
    parser.add_argument("--models", nargs="+", default=DEFAULT_MODELS)
    parser.add_argument(
        "--no-length-hint",
        action="store_true",
        help="withhold the solution length from the solver",
    )
    parser.add_argument("--concurrency", type=int, default=4)
    args = parser.parse_args()

    manifest = json.loads((args.directory / "manifest.json").read_text())
    client = anthropic.Anthropic(max_retries=4)

    # Fail on the credential before spending time on a thread pool that would
    # otherwise raise the same TypeError once per challenge, inside a worker.
    try:
        client.models.retrieve(args.models[0])
    except TypeError:
        sys.exit(
            "no credential found: export ANTHROPIC_API_KEY, or run `ant auth login`"
        )
    except anthropic.APIStatusError as error:
        sys.exit(f"credential rejected ({error.status_code}): {error.message}")
    results: dict[str, dict[str, str]] = {}

    for model in args.models:
        print(f"\n=== {model} ({len(manifest)} challenges)", flush=True)

        def attempt(entry: dict) -> tuple[str, str]:
            image = (args.directory / entry["file"]).read_bytes()
            answer = ask(
                client, model, image, entry["length"], not args.no_length_hint
            )
            return entry["file"], answer

        with ThreadPoolExecutor(max_workers=args.concurrency) as pool:
            answers = dict(pool.map(attempt, manifest))
        results[model] = answers
        report(manifest, answers)

    (args.directory / "results.json").write_text(json.dumps(results, indent=2) + "\n")
    print(f"\nwrote {args.directory}/results.json")

    if len(args.models) > 1:
        print("\n=== summary")
        for model, answers in results.items():
            solved, total, chars, char_total = score(manifest, answers)
            print(
                f"  {model:<20} {solved}/{total} solved, "
                f"{chars}/{char_total} characters ({100 * chars / char_total:.0f}%)"
            )


def score(manifest: list[dict], answers: dict[str, str]) -> tuple[int, int, int, int]:
    solved = chars = char_total = 0
    for entry in manifest:
        truth, guess = entry["solution"], answers.get(entry["file"], "")
        solved += truth == guess
        chars += sum(1 for a, b in zip(truth, guess) if a == b)
        char_total += len(truth)
    return solved, len(manifest), chars, char_total


def report(manifest: list[dict], answers: dict[str, str]) -> None:
    print(f"{'file':<10} {'len':>3} {'diff':>4}  {'truth':<9} {'answer':<9} chars")
    by_difficulty: dict[int, list[tuple[bool, int, int]]] = {}
    for entry in sorted(manifest, key=lambda e: (e["difficulty"], e["length"])):
        truth, guess = entry["solution"], answers.get(entry["file"], "")
        exact = truth == guess
        matched = sum(1 for a, b in zip(truth, guess) if a == b)
        by_difficulty.setdefault(entry["difficulty"], []).append(
            (exact, matched, len(truth))
        )
        flag = "SOLVED" if exact else ""
        print(
            f"{entry['file']:<10} {entry['length']:>3} {entry['difficulty']:>4}  "
            f"{truth:<9} {guess or '-':<9} {matched}/{len(truth)} {flag}"
        )

    solved, total, chars, char_total = score(manifest, answers)
    print(
        f"\n  exact solves {solved}/{total}, "
        f"characters {chars}/{char_total} ({100 * chars / char_total:.0f}%)"
    )
    for difficulty, rows in sorted(by_difficulty.items()):
        hit = sum(1 for exact, _, _ in rows if exact)
        got = sum(matched for _, matched, _ in rows)
        want = sum(length for _, _, length in rows)
        print(
            f"    difficulty {difficulty:>2}: {hit}/{len(rows)} solved, "
            f"characters {got}/{want}"
        )


if __name__ == "__main__":
    main()
