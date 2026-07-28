#!/usr/bin/env python3
"""Turn an lcov report into a coverage gate and a markdown summary.

This exists so that "coverage must not regress" is enforceable by a *required
GitHub check* without depending on a third party being reachable, activated and
configured. Codecov still receives the same lcov.info and still draws the nice
graphs; this script is what branch protection can actually block a merge on.

Two numbers are produced, and they answer different questions:

  project  every instrumented line in the report. Moves slowly, so it is a
           floor ("the codebase stays above N%"), not a signal about the change.
  patch    only the lines this pull request *added or modified*. This is the
           number worth gating on: a 2000-line codebase at 91% does not notice
           a 40-line untested function, and project coverage will happily stay
           at 91% while the new code is at 0%.

Patch coverage is computed against instrumented lines only. Blank lines, `}`,
comments and `#[derive(...)]` never appear in an lcov DA record, so a diff of
pure formatting has no patch denominator at all and reports N/A rather than 0%.

Usage:
    coverage-report.py --lcov lcov.info [--base-sha SHA] [--head-sha SHA]
                       [--min-project 85] [--min-patch 70] [--root .]

Exits 0 if both gates pass (or are not applicable), 1 if either fails, and 2 on
a usage/IO error. Writes GitHub Actions outputs when $GITHUB_OUTPUT is set and
appends the markdown summary to $GITHUB_STEP_SUMMARY when that is set.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

# Files whose coverage is reported but which should not drag the *patch* gate
# down, because no test can reach them in-process. Keep this list short and
# justified: every entry is coverage the project has decided it cannot get.
PATCH_EXCLUDE = (
    # `fn main` and the startup sequence. Exercised end-to-end by the Bruno API
    # suite and tests/cli_smoke_test.rs, but those drive a *subprocess*, so the
    # in-process llvm-cov instrumentation never sees them.
    "src/main.rs",
)


def parse_lcov(path: Path, root: Path) -> tuple[dict[str, dict[int, int]], int, int]:
    """Return ({repo-relative path: {line number: hit count}}, found, hit).

    cargo-llvm-cov emits absolute SF: paths, so they are made relative to the
    repository root to line up with what `git diff` reports.

    The per-line map and the LF/LH totals deliberately disagree, and the
    difference is not a bug in either. A line inside a generic or an inlined
    function gets one DA record per instantiation, so the map (keyed by line
    number, merged with max) sees fewer lines than LF counts. LF/LH is what
    `cargo llvm-cov --summary-only` and Codecov report, so the *project* number
    is taken from it and stays comparable with both; the map is what patch
    coverage needs, and there "covered by any instantiation" is the right merge.
    """
    files: dict[str, dict[int, int]] = defaultdict(dict)
    found = hit = 0
    current: str | None = None

    try:
        text = path.read_text(encoding="utf-8", errors="replace")
    except OSError as exc:
        sys.exit(f"error: cannot read {path}: {exc}")

    for line in text.splitlines():
        if line.startswith("SF:"):
            raw = Path(line[3:].strip())
            try:
                current = raw.resolve().relative_to(root.resolve()).as_posix()
            except ValueError:
                # Outside the repo (registry sources, generated files). Keep the
                # path as-is; it simply will not match anything in the diff.
                current = raw.as_posix()
        elif line.startswith("DA:") and current is not None:
            number, _, count = line[3:].strip().partition(",")
            try:
                # A line can appear more than once (generics, inlining). Take
                # the maximum: covered by any instantiation means covered.
                lineno = int(number)
                hits = int(count.split(",")[0])
            except ValueError:
                continue
            files[current][lineno] = max(files[current].get(lineno, 0), hits)
        elif line.startswith("LF:"):
            found += int(line[3:].strip() or 0)
        elif line.startswith("LH:"):
            hit += int(line[3:].strip() or 0)
        elif line.startswith("end_of_record"):
            current = None

    return dict(files), found, hit


HUNK = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")


def changed_lines(base: str, head: str, root: Path) -> dict[str, set[int]]:
    """Return {repo-relative path: set of added/modified line numbers}.

    `--unified=0` keeps context lines out of the result, so only lines the pull
    request actually wrote are counted. The three-dot range asks for the changes
    on this branch rather than every change on the base since it forked.
    """
    cmd = [
        "git", "diff", "--unified=0", "--no-color", "--no-renames",
        "--diff-filter=d",  # deleted files have no lines left to cover
        f"{base}...{head}", "--", "*.rs",
    ]
    try:
        out = subprocess.run(
            cmd, cwd=root, check=True, capture_output=True, text=True
        ).stdout
    except subprocess.CalledProcessError as exc:
        sys.exit(f"error: {' '.join(cmd)} failed: {exc.stderr.strip()}")
    except FileNotFoundError:
        sys.exit("error: git not found on PATH")

    result: dict[str, set[int]] = defaultdict(set)
    current: str | None = None
    for line in out.splitlines():
        if line.startswith("+++ b/"):
            current = line[6:].strip()
        elif line.startswith("+++ /dev/null"):
            current = None
        elif line.startswith("@@") and current is not None:
            m = HUNK.match(line)
            if m:
                start = int(m.group(1))
                count = int(m.group(2)) if m.group(2) is not None else 1
                result[current].update(range(start, start + count))
    return dict(result)


def pct(covered: int, total: int) -> float:
    """Percentage, treating "nothing to cover" as fully covered rather than 0%."""
    return 100.0 * covered / total if total else 100.0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--lcov", type=Path, default=Path("lcov.info"))
    ap.add_argument("--root", type=Path, default=Path("."))
    ap.add_argument("--base-sha", default=os.environ.get("COVERAGE_BASE_SHA", ""))
    ap.add_argument("--head-sha", default=os.environ.get("COVERAGE_HEAD_SHA", "HEAD"))
    ap.add_argument("--min-project", type=float, default=85.0)
    ap.add_argument("--min-patch", type=float, default=70.0)
    args = ap.parse_args()

    root = args.root.resolve()
    files, total, covered = parse_lcov(args.lcov, root)
    if not files:
        sys.exit(f"error: {args.lcov} contained no coverage records")

    # ---- project coverage -------------------------------------------------
    project = pct(covered, total)

    # ---- patch coverage ---------------------------------------------------
    patch: float | None = None
    patch_total = patch_covered = 0
    uncovered: list[tuple[str, list[int]]] = []

    if args.base_sha:
        diff = changed_lines(args.base_sha, args.head_sha, root)
        for path, lines in sorted(diff.items()):
            if path in PATCH_EXCLUDE or path not in files:
                continue
            hits = files[path]
            missing = []
            for lineno in sorted(lines):
                if lineno not in hits:
                    continue  # not instrumented: comment, blank, brace
                patch_total += 1
                if hits[lineno] > 0:
                    patch_covered += 1
                else:
                    missing.append(lineno)
            if missing:
                uncovered.append((path, missing))
        if patch_total:
            patch = pct(patch_covered, patch_total)

    # ---- verdict ----------------------------------------------------------
    project_ok = project >= args.min_project
    patch_ok = patch is None or patch >= args.min_patch

    def mark(ok: bool) -> str:
        return "✅" if ok else "❌"

    lines_out = [
        "## Coverage",
        "",
        "| scope | coverage | threshold | |",
        "|---|---:|---:|:--:|",
        f"| project | **{project:.2f}%** ({covered}/{total} lines) "
        f"| {args.min_project:.0f}% | {mark(project_ok)} |",
    ]
    if patch is not None:
        lines_out.append(
            f"| patch | **{patch:.2f}%** ({patch_covered}/{patch_total} new lines) "
            f"| {args.min_patch:.0f}% | {mark(patch_ok)} |"
        )
    elif args.base_sha:
        lines_out.append("| patch | n/a — no instrumented lines changed | — | ✅ |")

    if uncovered:
        shown = uncovered[:20]
        lines_out += ["", "<details><summary>Uncovered new lines</summary>", ""]
        for path, missing in shown:
            preview = ", ".join(str(n) for n in missing[:25])
            if len(missing) > 25:
                preview += f", … (+{len(missing) - 25} more)"
            lines_out.append(f"- `{path}`: {preview}")
        if len(uncovered) > len(shown):
            lines_out.append(f"- … and {len(uncovered) - len(shown)} more files")
        lines_out += ["", "</details>"]

    # Per-file table, worst first: this is what makes the summary actionable
    # rather than merely a number.
    ranked = sorted(
        (
            (p, sum(1 for h in l.values() if h > 0), len(l))
            for p, l in files.items()
            if len(l) > 0
        ),
        key=lambda r: (pct(r[1], r[2]), -r[2]),
    )
    worst = [r for r in ranked if pct(r[1], r[2]) < 100.0][:10]
    if worst:
        lines_out += [
            "",
            "<details><summary>Lowest-covered files</summary>",
            "",
            "| file | coverage | missed |",
            "|---|---:|---:|",
        ]
        for path, c, t in worst:
            lines_out.append(f"| `{path}` | {pct(c, t):.2f}% | {t - c} |")
        lines_out += ["", "</details>"]

    summary = "\n".join(lines_out)
    print(summary)

    step_summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if step_summary:
        with open(step_summary, "a", encoding="utf-8") as fh:
            fh.write(summary + "\n")

    gh_out = os.environ.get("GITHUB_OUTPUT")
    if gh_out:
        with open(gh_out, "a", encoding="utf-8") as fh:
            fh.write(f"project={project:.2f}\n")
            fh.write(f"patch={'' if patch is None else f'{patch:.2f}'}\n")
            fh.write(f"passed={'true' if project_ok and patch_ok else 'false'}\n")

    if not project_ok:
        print(
            f"::error::project coverage {project:.2f}% is below the "
            f"{args.min_project:.0f}% floor",
            file=sys.stderr,
        )
    if not patch_ok and patch is not None:
        print(
            f"::error::patch coverage {patch:.2f}% is below the "
            f"{args.min_patch:.0f}% threshold — new code needs tests",
            file=sys.stderr,
        )

    return 0 if (project_ok and patch_ok) else 1


if __name__ == "__main__":
    sys.exit(main())
