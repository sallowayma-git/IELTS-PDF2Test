#!/usr/bin/env python3
"""Deterministic local parser sidecar for Epic 8 authoring.

The sidecar converts TXT/MD/PDF/DOCX inputs into DocumentIRV1. It intentionally
keeps extraction deterministic and offline: LLMs must not participate in raw
fact extraction. PDF uses pypdf when available; DOCX uses the OOXML zip package
with Python stdlib so the adapter still works without python-docx.
"""
from __future__ import annotations

import argparse
import hashlib
import html
import json
import mimetypes
import re
import sys
import zipfile
from pathlib import Path
from typing import Iterable
from xml.etree import ElementTree as ET

NS_W = "{http://schemas.openxmlformats.org/wordprocessingml/2006/main}"
PAPER_WIDTH = 595
PAPER_HEIGHT = 842


def role_hint(text: str) -> str | None:
    lower = collapse(text).lower()
    if lower.startswith("answers") or "answer key" in lower or "答案" in lower:
        return "answer"
    if (
        "questions " in lower
        or lower.startswith("question ")
        or "choose one" in lower
        or "complete the" in lower
        or "true" in lower and "false" in lower and "not given" in lower
    ):
        return "question"
    if "reading passage" in lower or lower.startswith("passage ") or lower.startswith("passage "):
        return "passage"
    return None


def block_type(text: str) -> str:
    stripped = text.strip()
    if stripped.startswith("#") or stripped.upper().startswith("READING PASSAGE") or stripped.lower().startswith("questions "):
        return "header"
    if stripped.count("|") >= 2 or "\t" in stripped:
        return "table"
    if any(line.lstrip().startswith(("- ", "* ")) for line in stripped.splitlines()):
        return "list"
    return "paragraph"


def collapse(value: str) -> str:
    return re.sub(r"\s+", " ", value).strip()


def render_html(text: str, kind: str) -> str:
    stripped = text.strip()
    if kind == "header":
        return f"<h3>{html.escape(stripped.lstrip('#').strip())}</h3>"
    if kind == "list":
        items = []
        for line in stripped.splitlines():
            item = line.lstrip().removeprefix("- ").removeprefix("* ").strip()
            if item:
                items.append(f"<li>{html.escape(item)}</li>")
        return f"<ul>{''.join(items)}</ul>"
    if kind == "table":
        rows = []
        for line in stripped.splitlines():
            if "\t" in line:
                parts = line.split("\t")
            elif "|" in line and not set(line.strip()) <= {"|", "-", ":", " "}:
                parts = line.strip("|").split("|")
            else:
                continue
            cells = "".join(f"<td>{html.escape(cell.strip())}</td>" for cell in parts)
            if cells:
                rows.append(f"<tr>{cells}</tr>")
        return f"<table>{''.join(rows)}</table>" if rows else f"<p>{html.escape(stripped)}</p>"
    return f"<p>{html.escape(stripped)}</p>"


def paragraph_chunks(content: str) -> list[str]:
    chunks: list[str] = []
    current: list[str] = []
    for line in content.replace("\r\n", "\n").replace("\r", "\n").split("\n"):
        if line.strip():
            current.append(line)
        elif current:
            chunks.append("\n".join(current))
            current = []
    if current:
        chunks.append("\n".join(current))
    return chunks


def semantic_chunks(content: str) -> list[str]:
    chunks = paragraph_chunks(content)
    if len(chunks) > 1:
        return chunks

    text = collapse(content)
    if not text:
        return []

    # PDF text extraction often glues visual lines together. Recover the major
    # IELTS reading boundaries before falling back to generic marker slicing.
    recovered = re.sub(r"(READING\s+PASSAGE\s+\d+)(?=[A-Z])", r"\1\n", text, flags=re.IGNORECASE)
    recovered = re.sub(r"(?<!^)(?=Questions?\s+\d+)", "\n", recovered, flags=re.IGNORECASE)
    recovered = re.sub(r"(?<!^)(?=Answers?\b|Answer\s+Key\b)", "\n", recovered, flags=re.IGNORECASE)
    recovered = re.sub(
        r"((?:TRUE\s+FALSE|YES\s+NO)\s+NOT\s+GIVEN)(?=\d{1,3}\s+[A-Z])",
        r"\1\n",
        recovered,
        flags=re.IGNORECASE,
    )
    recovered = re.sub(r"(?<=[.!?])(?=\d{1,3}\s+[A-Z])", "\n", recovered)
    recovered_lines = [line.strip() for line in recovered.splitlines() if line.strip()]
    if len(recovered_lines) > 1:
        return recovered_lines

    marker = re.compile(
        r"(READING\s+PASSAGE\s+\d+|Questions?\s+\d+(?:\s*[-\u2013\u2014]\s*\d+)?|Answers?|Answer\s+Key|\b\d{1,3}\s+(?=[A-Z]))",
        re.IGNORECASE,
    )
    matches = list(marker.finditer(text))
    if not matches:
        return [text]

    semantic: list[str] = []
    for index, match in enumerate(matches):
        start = match.start()
        end = matches[index + 1].start() if index + 1 < len(matches) else len(text)
        chunk = text[start:end].strip()
        if chunk:
            semantic.append(chunk)
    prefix = text[: matches[0].start()].strip()
    if prefix:
        semantic.insert(0, prefix)
    return semantic or [text]


def make_block(block_id: str, text: str, page_index: int, ordinal: int, kind: str | None = None, confidence: float = 1.0) -> dict:
    kind = kind or block_type(text)
    y0 = 72 + (ordinal % 16) * 42
    block = {
        "blockId": block_id,
        "blockType": kind,
        "text": text,
        "html": render_html(text, kind),
        "bbox": [72, y0, 520, min(y0 + 36, PAPER_HEIGHT - 48)],
        "confidence": confidence,
    }
    hint = role_hint(text)
    if hint:
        block["roleHint"] = hint
    return block


def document_ir(job_id: str, pages: list[dict], mode: str, provider: str, warnings: list[str] | None = None) -> dict:
    return {
        "schemaVersion": "DocumentIRV1",
        "jobId": job_id,
        "pages": pages,
        "assets": [],
        "parser": {
            "provider": provider,
            "version": "0.3.0",
            "mode": mode,
            "warnings": warnings or [],
        },
    }


def sha256_bytes(content: bytes) -> str:
    return hashlib.sha256(content).hexdigest()


def image_mime_type(name: str) -> str:
    guessed, _ = mimetypes.guess_type(name)
    return guessed or "application/octet-stream"


def extract_pdf_images(input_path: Path, output_dir: Path, job_id: str) -> dict:
    warnings: list[str] = []
    try:
        from pypdf import PdfReader  # type: ignore
    except Exception as exc:  # pragma: no cover - depends on host env
        raise SystemExit(f"missing_pdf_dependency:pypdf:{exc}")

    reader = PdfReader(str(input_path))
    output_dir.mkdir(parents=True, exist_ok=True)
    pages: list[dict] = []
    image_counter = 1

    for page_index, page in enumerate(reader.pages, start=1):
        width = int(float(page.mediabox.width or PAPER_WIDTH))
        height = int(float(page.mediabox.height or PAPER_HEIGHT))
        page_images = []
        try:
            images = list(page.images)
        except Exception as exc:
            warnings.append(f"page {page_index} image extraction failed: {exc}")
            images = []

        for image in images:
            raw = getattr(image, "data", b"") or b""
            if not raw:
                warnings.append(f"page {page_index} image has no extractable bytes")
                continue
            original_name = getattr(image, "name", "") or f"image-{image_counter}.bin"
            suffix = Path(original_name).suffix or ".bin"
            file_name = f"page-{page_index:03d}-image-{image_counter:03d}{suffix}"
            image_path = output_dir / file_name
            image_path.write_bytes(raw)
            ref = getattr(image, "indirect_reference", None) or {}
            page_images.append(
                {
                    "assetId": f"pdf-page-{page_index}-image-{image_counter}",
                    "pageIndex": page_index,
                    "fileName": file_name,
                    "path": str(image_path),
                    "mimeType": image_mime_type(file_name),
                    "width": int(ref.get("/Width", 0) or 0) if hasattr(ref, "get") else 0,
                    "height": int(ref.get("/Height", 0) or 0) if hasattr(ref, "get") else 0,
                    "sha256": sha256_bytes(raw),
                    "sizeBytes": len(raw),
                }
            )
            image_counter += 1

        pages.append({"pageIndex": page_index, "width": width, "height": height, "images": page_images})

    if not any(page["images"] for page in pages):
        warnings.append("PDF contains no extractable embedded page images; manual transcription required")

    return {
        "schemaVersion": "PdfImageExtractionV1",
        "jobId": job_id,
        "sourcePath": str(input_path),
        "pages": pages,
        "warnings": warnings,
    }


def parse_text(input_path: Path, job_id: str, mode: str) -> dict:
    content = input_path.read_text(encoding="utf-8", errors="replace")
    chunks = semantic_chunks(content) or [input_path.stem]
    blocks = [make_block(f"b{index:03d}", chunk, 1, index - 1) for index, chunk in enumerate(chunks, start=1)]
    return document_ir(job_id, [{"pageIndex": 1, "width": PAPER_WIDTH, "height": PAPER_HEIGHT, "blocks": blocks}], mode, "python-parser-sidecar:text")


def parse_pdf(input_path: Path, job_id: str, mode: str) -> dict:
    warnings: list[str] = []
    try:
        from pypdf import PdfReader  # type: ignore
    except Exception as exc:  # pragma: no cover - depends on host env
        raise SystemExit(f"missing_pdf_dependency:pypdf:{exc}")

    reader = PdfReader(str(input_path))
    pages: list[dict] = []
    block_counter = 1
    for page_index, page in enumerate(reader.pages, start=1):
        width = int(float(page.mediabox.width or PAPER_WIDTH))
        height = int(float(page.mediabox.height or PAPER_HEIGHT))
        text = page.extract_text() or ""
        chunks = semantic_chunks(text)
        if not chunks:
            warnings.append(f"page {page_index} has no extractable text; OCR/manual review required")
            chunks = [f"[No extractable text on page {page_index}]"]
        blocks = []
        for ordinal, chunk in enumerate(chunks):
            confidence = 0.98 if not chunk.startswith("[No extractable text") else 0.2
            blocks.append(make_block(f"b{block_counter:03d}", chunk, page_index, ordinal, confidence=confidence))
            block_counter += 1
        pages.append({"pageIndex": page_index, "width": width, "height": height, "blocks": blocks})
    return document_ir(job_id, pages, mode, "python-parser-sidecar:pdf:pypdf", warnings)


def xml_text(node: ET.Element) -> str:
    return "".join(text_node.text or "" for text_node in node.iter(f"{NS_W}t"))


def docx_table_text(table: ET.Element) -> str:
    rows: list[str] = []
    for row in table.findall(f"{NS_W}tr"):
        cells = []
        for cell in row.findall(f"{NS_W}tc"):
            cell_text = collapse(" ".join(xml_text(paragraph) for paragraph in cell.findall(f"{NS_W}p")))
            cells.append(cell_text)
        if any(cells):
            rows.append("\t".join(cells))
    return "\n".join(rows)


def iter_docx_blocks(document_xml: bytes) -> Iterable[tuple[str, str]]:
    root = ET.fromstring(document_xml)
    body = root.find(f"{NS_W}body")
    if body is None:
        return []
    blocks: list[tuple[str, str]] = []
    for child in body:
        if child.tag == f"{NS_W}p":
            text = collapse(xml_text(child))
            if text:
                blocks.append(("paragraph", text))
        elif child.tag == f"{NS_W}tbl":
            text = docx_table_text(child)
            if text:
                blocks.append(("table", text))
    return blocks


def parse_docx(input_path: Path, job_id: str, mode: str) -> dict:
    warnings: list[str] = []
    with zipfile.ZipFile(input_path) as package:
        try:
            document_xml = package.read("word/document.xml")
        except KeyError as exc:
            raise SystemExit(f"invalid_docx_missing_document_xml:{exc}")
    raw_blocks = list(iter_docx_blocks(document_xml))
    if not raw_blocks:
        warnings.append("DOCX contains no extractable paragraphs or tables; manual review required")
        raw_blocks = [("paragraph", input_path.stem)]
    blocks = []
    for index, (kind, text) in enumerate(raw_blocks, start=1):
        block_kind = "table" if kind == "table" else block_type(text)
        blocks.append(make_block(f"b{index:03d}", text, 1, index - 1, kind=block_kind, confidence=0.99))
    return document_ir(job_id, [{"pageIndex": 1, "width": PAPER_WIDTH, "height": PAPER_HEIGHT, "blocks": blocks}], mode, "python-parser-sidecar:docx:ooxml", warnings)


def parse(input_path: Path, job_id: str, mode: str) -> dict:
    ext = input_path.suffix.lower()
    if ext in {".txt", ".md"}:
        return parse_text(input_path, job_id, mode)
    if ext == ".pdf":
        return parse_pdf(input_path, job_id, mode)
    if ext == ".docx":
        return parse_docx(input_path, job_id, mode)
    raise SystemExit(f"unsupported_parser_input:{ext or 'none'}")


def main() -> int:
    parser = argparse.ArgumentParser(prog="python-parser")
    sub = parser.add_subparsers(dest="command", required=True)
    parse_cmd = sub.add_parser("parse")
    parse_cmd.add_argument("--input", required=True)
    parse_cmd.add_argument("--output", required=True)
    parse_cmd.add_argument("--job-id", required=True)
    parse_cmd.add_argument("--mode", default="auto")
    extract_cmd = sub.add_parser("extract_pdf_images")
    extract_cmd.add_argument("--input", required=True)
    extract_cmd.add_argument("--output", required=True)
    extract_cmd.add_argument("--job-id", required=True)
    extract_cmd.add_argument("--asset-dir", required=True)
    args = parser.parse_args()

    input_path = Path(args.input)
    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    if args.command == "parse":
        result = parse(input_path, args.job_id, args.mode)
    elif args.command == "extract_pdf_images":
        result = extract_pdf_images(input_path, Path(args.asset_dir), args.job_id)
    else:  # pragma: no cover - argparse enforces this
        raise SystemExit(f"unsupported_command:{args.command}")
    output_path.write_text(json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
