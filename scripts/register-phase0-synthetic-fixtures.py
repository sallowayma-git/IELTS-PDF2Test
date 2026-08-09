"""Register generated synthetic fixtures in the Phase 0 golden manifest."""

from __future__ import annotations

import hashlib
import json
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
MANIFEST_PATH = ROOT / "fixtures" / "golden" / "manifest.json"
METADATA_ROOT = ROOT / "fixtures" / "golden" / "metadata"

SPECS = [
    ("pdf-two-column", "pdf", "pdf/two-column", 1, [(1, 4, "true_false_not_given", "radio")], [], ["passage", "question"], ["Two visual columns must remain separate in reading order."]),
    ("pdf-three-column", "pdf", "pdf/three-column", 1, [(1, 6, "matching", "select")], [], ["passage", "question"], ["Three columns are intentionally ambiguous without geometry-aware ordering."]),
    ("pdf-rotated-page", "pdf", "pdf/rotated-page", 1, [(1, 3, "true_false_not_given", "radio")], [], ["passage", "question"], ["Page rotation metadata must be normalized before grouping."]),
    ("pdf-image-only", "pdf", "pdf/image-only", 1, [], [("page-1-raster", "page_image")], ["visual_review"], ["The page has no native text and requires a visual asset/OCR review."]),
    ("pdf-hidden-ocr", "pdf", "pdf/hidden-ocr", 1, [(1, 3, "true_false_not_given", "radio")], [("page-1-raster", "page_image")], ["visual_review", "question"], ["The page contains an image plus a hidden text layer; native/OCR provenance must be retained."]),
    ("pdf-native-ocr-conflict", "pdf", "pdf/native-ocr-conflict", 1, [(1, 2, "single_choice", "radio")], [], ["question"], ["Visible native text and hidden OCR text intentionally disagree."]),
    ("pdf-broken-font", "pdf", "pdf/broken-font", 1, [(1, 3, "short_answer", "text")], [], ["passage", "question"], ["Unicode and missing-glyph behavior needs explicit diagnostics."]),
    ("pdf-mixed-text-image", "pdf", "pdf/mixed-text-image", 1, [(1, 2, "diagram_completion", "text")], [("figure-1", "embedded_image")], ["passage", "question", "figure"], ["Text and the embedded figure must share a source region without flattening the asset."]),
    ("pdf-ruled-table", "pdf", "pdf/ruled-table", 1, [(1, 4, "table_completion", "text")], [], ["question", "table"], ["Grid lines provide topology evidence for table reconstruction."]),
    ("pdf-borderless-table", "pdf", "pdf/borderless-table", 1, [(1, 4, "table_completion", "text")], [], ["question", "table"], ["Column alignment, not vector rules, is the only table evidence."]),
    ("pdf-vector-diagram", "pdf", "pdf/vector-diagram", 1, [(1, 3, "diagram_completion", "text")], [("vector-figure-1", "vector_figure")], ["question", "figure"], ["The diagram is composed of vector shapes and nearby labels."]),
    ("pdf-map-hotspot", "pdf", "pdf/map-hotspot", 1, [(1, 4, "diagram_completion", "text")], [("map-1", "map")], ["question", "figure"], ["Answer locations are spatial hotspots inside the map."]),
    ("pdf-flowchart", "pdf", "pdf/flowchart", 1, [(1, 4, "diagram_completion", "text")], [("flowchart-1", "flowchart")], ["question", "figure"], ["Arrows and boxes define a flow layout rather than ordinary paragraphs."]),
    ("pdf-header-footer", "pdf", "pdf/header-footer", 2, [(1, 3, "summary_completion", "text")], [], ["passage", "question", "header", "footer"], ["Repeated header/footer text must not be assigned to passage content."]),
    ("pdf-question-before-passage", "pdf", "pdf/question-before-passage", 2, [(1, 3, "heading_matching", "select")], [], ["question", "passage"], ["Questions appear on page 1 and the passage on page 2; physical order is not semantic order."]),
    ("docx-floating-text-box", "docx", "docx/floating-text-box", 1, [(1, 2, "short_answer", "text")], [], ["question", "text_box"], ["The question text is inside a floating VML text box."]),
    ("docx-external-image", "docx", "docx/external-image", 1, [], [("external-image-1", "external_image")], ["visual_review"], ["The image relationship targets an unavailable external resource and must block formal publish."]),
    ("docx-smartart", "docx", "docx/smartart", 1, [(1, 3, "diagram_completion", "text")], [("smartart-1", "smartart")], ["question", "figure"], ["SmartArt is represented by diagram parts and relationships, not only document text."]),
    ("docx-section-columns", "docx", "docx/section-columns", 1, [(1, 4, "matching", "select")], [], ["passage", "question"], ["Section column settings must survive OOXML extraction."]),
    ("docx-table-merged-cells", "docx", "docx/table-merged-cells", 1, [(1, 4, "table_completion", "text")], [], ["question", "table"], ["Merged cells and grid spans must remain explicit in the table IR."]),
]


def sha256(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def group_entry(group_number: int, start: int, end: int, kind: str, response_type: str) -> tuple[dict, list[dict]]:
    slot_ids = [f"q{number}" for number in range(start, end + 1)]
    group = {
        "id": f"group-{group_number}",
        "displayRange": [start, end],
        "kind": kind,
        "slotIds": slot_ids,
    }
    slots = [
        {"id": slot_id, "displayNumber": str(number), "responseType": response_type}
        for number, slot_id in zip(range(start, end + 1), slot_ids)
    ]
    return group, slots


def main() -> None:
    manifest = json.loads(MANIFEST_PATH.read_text(encoding="utf-8"))
    manifest_by_id = {entry["fixtureId"]: entry for entry in manifest.get("fixtures", [])}
    metadata_paths = []
    for fixture_id, format_name, source_suffix, page_count, groups, assets, roles, known_issues in SPECS:
        extension = "pdf" if format_name == "pdf" else "docx"
        source_dir = "pdf" if format_name == "pdf" else "docx"
        source_path = f"fixtures/golden/synthetic/{source_dir}/{fixture_id}.{extension}"
        source = ROOT / source_path
        if not source.is_file():
            raise FileNotFoundError(source)
        task_groups = []
        slots = []
        for index, group_spec in enumerate(groups, start=1):
            group, group_slots = group_entry(index, *group_spec)
            task_groups.append(group)
            slots.extend(group_slots)
        metadata = {
            "schemaVersion": "GoldenFixtureMetadataV1",
            "fixtureId": fixture_id,
            "source": {
                "path": source_path,
                "sha256": sha256(source),
                "sizeBytes": source.stat().st_size,
                "format": format_name,
            },
            "expected": {
                "pageRoles": [{"pageIndex": index, "roles": roles if index == 1 else ["passage"]} for index in range(1, page_count + 1)],
                "taskGroups": task_groups,
                "slots": slots,
                "assets": [{"id": asset_id, "type": asset_type, "required": True} for asset_id, asset_type in assets],
            },
            "knownIssues": known_issues,
            "baseline": {
                "v1Path": f"fixtures/golden/baseline/v1/{fixture_id}.json",
                "observed": {},
            },
        }
        metadata_path = METADATA_ROOT / f"{fixture_id}.json"
        if metadata_path.is_file():
            existing_metadata = json.loads(metadata_path.read_text(encoding="utf-8"))
            metadata["baseline"]["observed"] = existing_metadata.get("baseline", {}).get("observed", {})
        metadata_path.write_text(json.dumps(metadata, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
        metadata_paths.append(metadata_path)
        manifest_by_id[fixture_id] = {
            "fixtureId": fixture_id,
            "status": "available",
            "sourcePath": source_path,
            "sha256": metadata["source"]["sha256"],
            "sizeBytes": metadata["source"]["sizeBytes"],
            "metadataPath": f"fixtures/golden/metadata/{fixture_id}.json",
            "baselinePath": f"fixtures/golden/baseline/v1/{fixture_id}.json",
        }

    existing_ids = {fixture_id for fixture_id, *_ in SPECS}
    manifest["fixtures"] = sorted(manifest_by_id.values(), key=lambda entry: entry["fixtureId"])
    manifest["plannedSyntheticFixtures"] = [
        entry for entry in manifest.get("plannedSyntheticFixtures", []) if entry.get("fixtureId") not in existing_ids
    ]
    MANIFEST_PATH.write_text(json.dumps(manifest, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"registered": len(SPECS), "metadata": [path.relative_to(ROOT).as_posix() for path in metadata_paths]}, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
