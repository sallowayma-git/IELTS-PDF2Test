#!/usr/bin/env python3
"""Summarize LLM token usage from one retained job directory."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Any


def integer(value: Any) -> int | None:
    return value if isinstance(value, int) and not isinstance(value, bool) else None


def usage_value(record: dict[str, Any], usage: dict[str, Any], key: str) -> int | None:
    value = integer(record.get(key))
    if value is not None:
        return value
    value = integer(usage.get(key))
    if value is not None:
        return value
    if key == "prompt_cache_hit_tokens":
        details = usage.get("prompt_tokens_details")
        if isinstance(details, dict):
            return integer(details.get("cached_tokens"))
    return None


def records_from(path: Path) -> list[dict[str, Any]]:
    records: list[dict[str, Any]] = []
    for line_number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not line.strip():
            continue
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            print(f"Skipping malformed JSONL entry at line {line_number}.")
            continue
        if not isinstance(record, dict):
            continue
        if record.get("recordType") in ("llm_usage", "llm_call"):
            records.append(record)
    return records


def report(job_dir: Path) -> int:
    retained = job_dir / "llm-usage.jsonl"
    transient = job_dir / "llm-calls.jsonl"
    path = retained if retained.is_file() else transient
    if not path.is_file():
        print("No llm-usage.jsonl or llm-calls.jsonl found in the job directory.")
        return 2

    rows = records_from(path)
    if not rows:
        print(f"No LLM call records found in {path.name}.")
        return 0

    totals = {"prompt": 0, "completion": 0, "hit": 0, "miss": 0}
    known = {"prompt": 0, "completion": 0, "hit": 0, "miss": 0}
    print(f"Log: {path}")
    print("#  command                         step   inputB   prompt  output   hit   miss  hit%   sourceB draftB obsB imageB")
    for ordinal, record in enumerate(rows, 1):
        usage = record.get("usage") if isinstance(record.get("usage"), dict) else {}
        prompt = usage_value(record, usage, "prompt_tokens")
        completion = usage_value(record, usage, "completion_tokens")
        hit = usage_value(record, usage, "prompt_cache_hit_tokens")
        miss = usage_value(record, usage, "prompt_cache_miss_tokens")
        if miss is None and prompt is not None and hit is not None:
            miss = max(0, prompt - hit)
        for name, value in (("prompt", prompt), ("completion", completion), ("hit", hit), ("miss", miss)):
            if value is not None:
                totals[name] += value
                known[name] += 1

        segments = record.get("segments") if isinstance(record.get("segments"), dict) else {}
        input_bytes = integer(record.get("inputBytes"))
        if input_bytes is None:
            input_bytes = integer(record.get("requestBytes"))
        cache_rate = f"{100 * hit / prompt:.1f}" if hit is not None and prompt else "-"
        command = str(record.get("commandName", "unknown"))[:31]
        step = integer(record.get("stepIndex"))
        print(
            f"{ordinal:>2} {command:<31} {str(step or '-'):>4} "
            f"{str(input_bytes if input_bytes is not None else '-'):>7} "
            f"{str(prompt if prompt is not None else '-'):>7} "
            f"{str(completion if completion is not None else '-'):>7} "
            f"{str(hit if hit is not None else '-'):>5} "
            f"{str(miss if miss is not None else '-'):>6} "
            f"{cache_rate:>5} "
            f"{str(integer(segments.get('source')) or 0):>7} "
            f"{str(integer(segments.get('draft')) or 0):>5} "
            f"{str(integer(segments.get('observations')) or 0):>4} "
            f"{str(integer(segments.get('images')) or 0):>6}"
        )

    hit_rate = f"{100 * totals['hit'] / totals['prompt']:.1f}%" if totals["prompt"] else "-"
    print(
        "Totals: "
        f"calls={len(rows)}, prompt={totals['prompt']}, completion={totals['completion']}, "
        f"cache_hit={totals['hit']}, cache_miss={totals['miss']}, hit_rate={hit_rate}"
    )
    for name, count in known.items():
        if count != len(rows):
            print(f"Note: {name} token usage is missing on {len(rows) - count} call(s).")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("job_directory", type=Path, help="directory for one import job")
    args = parser.parse_args()
    if not args.job_directory.is_dir():
        parser.error("job_directory must be an existing directory")
    return report(args.job_directory)


if __name__ == "__main__":
    raise SystemExit(main())
