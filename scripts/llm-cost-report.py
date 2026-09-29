#!/usr/bin/env python3
"""Report recorded usage or estimate cost from exact serialized request bodies."""

from __future__ import annotations

import argparse
import base64
import json
import math
import re
from pathlib import Path
from typing import Any, Iterable


RMB_PER_MILLION = {
    "flash": {
        "offpeak": {"hit": 0.02, "miss": 1.0, "output": 4.0},
        "peak": {"hit": 0.04, "miss": 2.0, "output": 8.0},
    },
    "pro": {
        "offpeak": {"hit": 0.15, "miss": 4.5, "output": 13.5},
        "peak": {"hit": 0.30, "miss": 9.0, "output": 27.0},
    },
}
ASCII_CHAR_TOKENS = 0.3
NON_ASCII_CHAR_TOKENS = 0.6
IMAGE_TOKENS_UPPER_BOUND = 1024
PDF_PAGE_TOKENS_UPPER_BOUND = 1024
DEFAULT_COMPLETION_TOKENS_PER_CALL = 1024
PRICING_SOURCE = "https://api-docs.deepseek.com/zh-cn/quick_start/pricing/"


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
        if isinstance(record, dict) and record.get("recordType") in ("llm_usage", "llm_call"):
            records.append(record)
    return records


def _pdf_page_count(encoded_pdf: str) -> int | None:
    marker = "base64,"
    base64_data = encoded_pdf.split(marker, 1)[1] if marker in encoded_pdf else encoded_pdf
    try:
        pdf = base64.b64decode(base64_data, validate=False)
    except (ValueError, base64.binascii.Error):
        return None
    pages = len(re.findall(rb"/Type\s*/Page\b", pdf))
    return pages or None


def _record_pdf(encoded_pdf: str, stats: dict[str, int | None]) -> None:
    stats["pdfFiles"] = int(stats["pdfFiles"] or 0) + 1
    pages = _pdf_page_count(encoded_pdf)
    if pages is None:
        stats["unknownPdfPages"] = int(stats["unknownPdfPages"] or 0) + 1
        return
    stats["pdfPages"] = int(stats["pdfPages"] or 0) + pages
    stats["imageTokens"] = int(stats["imageTokens"] or 0) + pages * PDF_PAGE_TOKENS_UPPER_BOUND


def _collect_prompt_stats(value: Any, stats: dict[str, int | None]) -> None:
    if isinstance(value, dict):
        block_type = value.get("type")
        if block_type == "image_url":
            image = value.get("image_url")
            url = image.get("url") if isinstance(image, dict) else None
            if isinstance(url, str):
                if url.startswith("data:application/pdf;base64,"):
                    _record_pdf(url, stats)
                else:
                    stats["images"] = int(stats["images"] or 0) + 1
                    stats["imageTokens"] = int(stats["imageTokens"] or 0) + IMAGE_TOKENS_UPPER_BOUND
            return
        if block_type == "file":
            file_data = value.get("file")
            url = file_data.get("file_data") if isinstance(file_data, dict) else None
            filename = file_data.get("filename") if isinstance(file_data, dict) else None
            if isinstance(url, str) and (
                url.startswith("data:application/pdf;base64,")
                or (isinstance(filename, str) and filename.lower().endswith(".pdf"))
            ):
                _record_pdf(url, stats)
            return
        for child in value.values():
            _collect_prompt_stats(child, stats)
        return
    if isinstance(value, list):
        for child in value:
            _collect_prompt_stats(child, stats)
        return
    if isinstance(value, str):
        stats["textTokens"] = int(stats["textTokens"] or 0) + math.ceil(
            sum(ASCII_CHAR_TOKENS if ord(char) < 128 else NON_ASCII_CHAR_TOKENS for char in value)
        )


def estimate_prompt(body: dict[str, Any]) -> dict[str, int | None]:
    stats: dict[str, int | None] = {
        "textTokens": 0,
        "images": 0,
        "imageTokens": 0,
        "pdfFiles": 0,
        "pdfPages": 0,
        "unknownPdfPages": 0,
    }
    prompt_fields = {key: body[key] for key in ("messages", "tools", "functions", "response_format") if key in body}
    _collect_prompt_stats(prompt_fields, stats)
    stats["estimatedInputTokens"] = int(stats["textTokens"] or 0) + int(stats["imageTokens"] or 0)
    return stats


def longest_common_prefix(left: bytes, right: bytes) -> int:
    limit = min(len(left), len(right))
    index = 0
    while index < limit and left[index] == right[index]:
        index += 1
    return index


def _latest_request_dir(job_dir: Path) -> Path | None:
    candidates = list((job_dir / "llm-dry-run").glob("*/requests"))
    if not candidates:
        candidates = [job_dir / "requests"] if (job_dir / "requests").is_dir() else []
    return max(candidates, key=lambda path: path.stat().st_mtime) if candidates else None


def _request_rows(request_dir: Path) -> list[tuple[Path, bytes, dict[str, Any]]]:
    metadata: dict[str, dict[str, Any]] = {}
    metadata_path = request_dir.parent / "requests.jsonl"
    if metadata_path.is_file():
        for record in records_from_any_jsonl(metadata_path):
            name = record.get("fileName")
            if isinstance(name, str):
                metadata[name] = record
    rows = []
    for path in sorted(request_dir.glob("request-*.bin")):
        rows.append((path, path.read_bytes(), metadata.get(path.name, {})))
    if not rows:
        for path in sorted(request_dir.glob("request-*.json")):
            rows.append((path, path.read_bytes(), metadata.get(path.name, {})))
    return rows


def records_from_any_jsonl(path: Path) -> Iterable[dict[str, Any]]:
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            yield value


def _request_report(job_dir: Path, request_dir: Path, pricing_model: str, period: str, output_tokens: int) -> int:
    rows = _request_rows(request_dir)
    if not rows:
        print(f"No serialized request bodies found in {request_dir}.")
        return 2

    rates = RMB_PER_MILLION[pricing_model][period]
    previous: bytes | None = None
    totals = {"bytes": 0, "tokens": 0, "hit": 0, "miss": 0, "images": 0, "output": 0}
    predicted_cost = 0.0
    uncached_cost = 0.0
    print(f"Requests: {request_dir}")
    print(f"Pricing: DeepSeek {pricing_model} {period}; RMB/M input(hit/miss)/output={rates['hit']}/{rates['miss']}/{rates['output']}")
    print(f"Price source: {PRICING_SOURCE}")
    print("Token estimate: ASCII chars × 0.3 + non-ASCII chars × 0.6; page images and PDF pages use 1,024 tokens each (upper-bound proxy).")
    print(f"Output estimate: {output_tokens} tokens/call (configurable; actual completion usage is unavailable in dry-run).")
    print("# command                         step   bodyB   estIn textTok img pdfPg imgTok   LCPB    LCP%  hitUpper missLower outEst  RMB*  RMB-no-cache")
    for ordinal, (path, body_bytes, meta) in enumerate(rows, 1):
        try:
            body = json.loads(body_bytes)
        except (json.JSONDecodeError, UnicodeDecodeError):
            print(f"{ordinal:>2} {path.name}: invalid JSON body")
            continue
        if not isinstance(body, dict):
            print(f"{ordinal:>2} {path.name}: JSON body is not an object")
            continue
        stats = estimate_prompt(body)
        prompt_tokens = int(stats["estimatedInputTokens"] or 0)
        prefix_bytes = longest_common_prefix(previous, body_bytes) if previous is not None else 0
        prefix_ratio = prefix_bytes / len(body_bytes) if body_bytes else 0.0
        hit_upper = min(prompt_tokens, round(prompt_tokens * prefix_ratio)) if previous is not None else 0
        miss_lower = prompt_tokens - hit_upper
        output_estimate = output_tokens
        model = str(body.get("model", "unknown"))
        command = str(meta.get("commandName", path.name))
        step = meta.get("stepIndex", meta.get("callIndex", "-"))
        predicted = (hit_upper * rates["hit"] + miss_lower * rates["miss"] + output_estimate * rates["output"]) / 1_000_000
        no_cache = (prompt_tokens * rates["miss"] + output_estimate * rates["output"]) / 1_000_000
        unknown_pdf = int(stats["unknownPdfPages"] or 0)
        if unknown_pdf:
            print(f"   {path.name}: PDF page count unknown; its image-token estimate is omitted")
        print(
            f"{ordinal:>2} {command:<31.31} {str(step):>4} {len(body_bytes):>7} "
            f"{prompt_tokens:>7} {int(stats['textTokens'] or 0):>7} {int(stats['images'] or 0):>3} "
            f"{int(stats['pdfPages'] or 0):>5} {int(stats['imageTokens'] or 0):>7} "
            f"{prefix_bytes:>6} {100 * prefix_ratio:>6.1f} {hit_upper:>9} {miss_lower:>9} "
            f"{output_estimate:>6} {predicted:>6.4f} {no_cache:>12.4f}"
        )
        totals["bytes"] += len(body_bytes)
        totals["tokens"] += prompt_tokens
        totals["hit"] += hit_upper
        totals["miss"] += miss_lower
        totals["images"] += int(stats["images"] or 0)
        totals["output"] += output_estimate
        predicted_cost += predicted
        uncached_cost += no_cache
        previous = body_bytes
    hit_ratio = 100 * totals["hit"] / totals["tokens"] if totals["tokens"] else 0.0
    print(
        f"Total: calls={len(rows)}, bodyBytes={totals['bytes']}, estInputTokens={totals['tokens']}, "
        f"images={totals['images']}, prefixHitUpper={totals['hit']}, missLower={totals['miss']}, "
        f"prefixHitUpperRate={hit_ratio:.1f}%, assumedOutputTokens={totals['output']}, "
        f"costAtPrefixUpper=¥{predicted_cost:.4f}, "
        f"costWithoutCache=¥{uncached_cost:.4f}"
    )
    print(f"* `RMB*` is an optimistic estimate using the byte-LCP ratio as a token-cache upper bound; output is assumed at {output_tokens} tokens/call. Model in request: {model}")
    return 0


def _usage_report(job_dir: Path) -> int:
    retained = job_dir / "llm-usage.jsonl"
    transient = job_dir / "llm-calls.jsonl"
    path = retained if retained.is_file() else transient
    if not path.is_file():
        print("No serialized requests or LLM usage logs found.")
        return 2
    rows = records_from(path)
    if not rows:
        print(f"No LLM call records found in {path.name}.")
        return 0
    totals = {"prompt": 0, "completion": 0, "hit": 0, "miss": 0}
    known = {"prompt": 0, "completion": 0, "hit": 0, "miss": 0}
    print(f"Log: {path}")
    print("#  command                         step  requestB  inputB   prompt  output   hit   miss  hit%   sourceB draftB obsB imageB")
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
        request_bytes = integer(record.get("requestBytes"))
        input_bytes = integer(record.get("inputBytes"))
        if request_bytes is None:
            request_bytes = input_bytes
        cache_rate = f"{100 * hit / prompt:.1f}" if hit is not None and prompt else "-"
        command = str(record.get("commandName", "unknown"))[:31]
        step = integer(record.get("stepIndex"))
        print(
            f"{ordinal:>2} {command:<31} {str(step or '-'):>4} "
            f"{str(request_bytes if request_bytes is not None else '-'):>9} "
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
    print(f"Totals: calls={len(rows)}, prompt={totals['prompt']}, completion={totals['completion']}, cache_hit={totals['hit']}, cache_miss={totals['miss']}, hit_rate={hit_rate}")
    for name, count in known.items():
        if count != len(rows):
            print(f"Note: {name} token usage is missing on {len(rows) - count} call(s).")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("job_directory", type=Path, help="job directory or a serialized request directory")
    parser.add_argument("--requests-dir", type=Path, help="directory containing request-*.bin files")
    parser.add_argument("--pricing-model", choices=sorted(RMB_PER_MILLION), default="flash")
    parser.add_argument("--period", choices=("peak", "offpeak"), default="peak")
    parser.add_argument("--completion-tokens-per-call", type=int, default=DEFAULT_COMPLETION_TOKENS_PER_CALL)
    args = parser.parse_args()
    if not args.job_directory.is_dir():
        parser.error("job_directory must be an existing directory")
    request_dir = args.requests_dir or _latest_request_dir(args.job_directory)
    if request_dir and request_dir.is_dir():
        if args.completion_tokens_per_call < 0:
            parser.error("completion-tokens-per-call must be non-negative")
        return _request_report(args.job_directory, request_dir, args.pricing_model, args.period, args.completion_tokens_per_call)
    return _usage_report(args.job_directory)


if __name__ == "__main__":
    raise SystemExit(main())
