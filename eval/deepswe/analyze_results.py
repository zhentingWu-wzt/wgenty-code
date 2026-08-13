#!/usr/bin/env python3
"""Analyze DeepSWE eval results under jobs/ and print a per-task summary.

Extracts f2p/p2p/reward from result.json, failed test names + assertion
details from verifier test-stdout.txt (go test JSONL / pytest / vitest
formats are handled heuristically).

Usage:
    python3 eval/deepswe/analyze_results.py [jobs_dir] [--json]
"""

import argparse
import json
import os
import re
import sys


def load_result(path: str) -> dict:
    try:
        with open(path) as f:
            return json.load(f)
    except (OSError, json.JSONDecodeError):
        return {}


def extract_failures(stdout_path: str) -> list[dict]:
    """Heuristic extraction of failed tests + assertion lines."""
    failures: list[dict] = []
    try:
        with open(stdout_path, encoding="utf-8", errors="replace") as f:
            text = f.read()
    except OSError:
        return failures

    # go test JSONL: {"Action":"fail","Test":"X","Package":"P","Output":"..."}
    # Aggregate Output lines per failed test name.
    out_by_test: dict[str, str] = {}
    seen_fail: set[str] = set()
    for line in text.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            rec = json.loads(line)
        except json.JSONDecodeError:
            continue
        test = rec.get("Test") or ""
        action = rec.get("Action") or ""
        if action == "fail" and test:
            seen_fail.add(test)
        if action == "output" and test:
            out_by_test[test] = out_by_test.get(test, "") + rec.get("Output", "")

    for test in sorted(seen_fail):
        out = out_by_test.get(test, "")
        detail = _first_assertion(out) or _first_error_line(out) or out.strip().splitlines()[0] if out.strip() else ""
        failures.append({"test": test, "detail": detail[:400]})

    # pytest / vitest text fallback: --- FAILED --- / FAIL lines
    if not failures:
        for m in re.finditer(r"^(?:FAILED|FAIL)\s+(\S+)(?:\s*-\s*(.*))?$", text, re.M):
            failures.append({"test": m.group(1), "detail": (m.group(2) or "")[:400]})
    return failures


def _first_assertion(out: str) -> str:
    for kw in ("Error Trace", "Error:", "expected", "got", "want", "actual"):
        idx = out.find(kw)
        if idx >= 0:
            return out[idx : idx + 220].strip()
    return ""


def _first_error_line(out: str) -> str:
    for line in out.splitlines():
        s = line.strip()
        if s.startswith(("Error", "assert", "AssertionError", "Traceback")):
            return s
    return ""


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("jobs_dir", nargs="?", default="jobs")
    ap.add_argument("--json", action="store_true", help="dump machine-readable JSON")
    args = ap.parse_args()

    rows = []
    for group in sorted(os.listdir(args.jobs_dir)):
        gdir = os.path.join(args.jobs_dir, group)
        if not os.path.isdir(gdir):
            continue
        for task_dir in sorted(os.listdir(gdir)):
            tdir = os.path.join(gdir, task_dir)
            if not os.path.isdir(tdir):
                continue
            res = load_result(os.path.join(tdir, "result.json")) or load_result(
                os.path.join(gdir, "result.json")
            )
            stats = res.get("stats", {}) if isinstance(res.get("stats"), dict) else {}
            failures = extract_failures(os.path.join(tdir, "verifier", "test-stdout.txt"))
            rows.append(
                {
                    "group": group,
                    "task": task_dir,
                    "f2p": stats.get("f2p"),
                    "p2p": stats.get("p2p"),
                    "f2p_total": stats.get("f2p_total"),
                    "p2p_total": stats.get("p2p_total"),
                    "partial": stats.get("partial"),
                    "reward": res.get("reward") or stats.get("reward"),
                    "failures": failures,
                }
            )

    if args.json:
        print(json.dumps(rows, indent=1, ensure_ascii=False))
        return

    print(f"{'group':<36} {'task':<46} {'f2p':>12} {'p2p':>12} {'reward':>7} {'fail':>4}")
    for r in rows:
        f2p = r["f2p"] if r["f2p"] is not None else ""
        p2p = r["p2p"] if r["p2p"] is not None else ""
        reward = r["reward"] if r["reward"] is not None else ""
        fails = ",".join(f['test'] for f in r["failures"])[:40]
        print(f"{r['group'][:36]:<36} {r['task'][:46]:<46} {str(f2p):>12} {str(p2p):>12} {str(reward):>7} {fails:>4}")

    # failure detail dump for reward-0 tasks
    print("\n=== failure details (reward=0 / f2p misses) ===")
    for r in rows:
        if r["reward"] != 1 and r["failures"]:
            print(f"\n-- {r['task']} --")
            for f in r["failures"][:5]:
                print(f"  FAIL {f['test']}")
                if f["detail"]:
                    print(f"       {f['detail'][:300]}")


if __name__ == "__main__":
    main()
