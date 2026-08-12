"""Generate deterministic, checked-in DOCX packages for Phase 3 adversarial tests."""

from __future__ import annotations

import struct
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1] / "fixtures" / "golden" / "synthetic" / "docx"
W = "http://schemas.openxmlformats.org/wordprocessingml/2006/main"
R = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
P = "http://schemas.openxmlformats.org/package/2006/relationships"
CT = "http://schemas.openxmlformats.org/package/2006/content-types"


def xml_document(body: str, sect: str = "") -> bytes:
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="{W}" xmlns:r="{R}" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart" xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram" xmlns:v="urn:schemas-microsoft-com:vml">
<w:body>{body}{sect}</w:body></w:document>'''.encode()


def paragraph(text: str = "fixture") -> str:
    return f"<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"


def content_types(extra: str = "") -> bytes:
    return f'''<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="{CT}">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Default Extension="png" ContentType="image/png"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
{extra}</Types>'''.encode()


def root_rels() -> bytes:
    return f'''<Relationships xmlns="{P}"><Relationship Id="rIdRoot" Type="{R}/officeDocument" Target="word/document.xml"/></Relationships>'''.encode()


def rels(entries: list[tuple[str, str, str, str | None]]) -> bytes:
    values = []
    for rid, kind, target, mode in entries:
        target_mode = f' TargetMode="{mode}"' if mode else ""
        values.append(f'<Relationship Id="{rid}" Type="{R}/{kind}" Target="{target}"{target_mode}/>')
    return f'<Relationships xmlns="{P}">{"".join(values)}</Relationships>'.encode()


def image_drawing(rid: str, floating: bool = False, textbox: str | None = None) -> str:
    container = "anchor" if floating else "inline"
    geometry = "" if floating else "<wp:extent cx=\"914400\" cy=\"457200\"/>"
    if floating:
        geometry = "<wp:extent cx=\"914400\" cy=\"457200\"/><wp:wrapSquare/>"
    text_box = f"<w:txbxContent>{paragraph(textbox)}</w:txbxContent>" if textbox else ""
    return f'''<w:r><w:drawing><wp:{container} relativeHeight="0">{geometry}<wp:docPr id="1" descr="fixture image"/>{text_box}<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:embed="{rid}"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:{container}></w:drawing></w:r>'''


def smartart_drawing(rid: str, kind: str = "diagramData") -> str:
    uri = "http://schemas.openxmlformats.org/drawingml/2006/diagram" if kind == "diagramData" else "http://schemas.openxmlformats.org/drawingml/2006/chart"
    tag = "dgm:relIds" if kind == "diagramData" else "c:chart"
    attribute = "r:dm" if kind == "diagramData" else "r:id"
    return f'''<w:r><w:drawing><wp:inline><wp:extent cx="1200000" cy="700000"/><wp:docPr id="2" descr="composite fixture"/><a:graphic><a:graphicData uri="{uri}"><{tag} {attribute}="{rid}"/></a:graphicData></a:graphic></wp:inline></w:drawing></w:r>'''


def table_xml(rows: list[list[str]], merges: list[tuple[int, int, str]] | None = None) -> str:
    merges = merges or []
    values = ["<w:tbl><w:tblGrid>" + "".join('<w:gridCol w:w="1440"/>' for _ in range(max(len(row) for row in rows))) + "</w:tblGrid>"]
    for row_index, row in enumerate(rows):
        values.append("<w:tr>")
        for col_index, text in enumerate(row):
            merge = next((value for r, c, value in merges if r == row_index and c == col_index), None)
            property_xml = f"<w:tcPr><w:vMerge w:val=\"{merge}\"/></w:tcPr>" if merge else ""
            values.append(f"<w:tc>{property_xml}{paragraph(text) if text else '<w:p/>'}</w:tc>")
        values.append("</w:tr>")
    values.append("</w:tbl>")
    return "".join(values)


def base_entries(document: bytes, relationships: list[tuple[str, str, str, str | None]] | None = None, extra: dict[str, bytes] | None = None, types: str = "") -> dict[str, bytes]:
    result = {
        "[Content_Types].xml": content_types(types),
        "_rels/.rels": root_rels(),
        "word/document.xml": document,
    }
    if relationships:
        result["word/_rels/document.xml.rels"] = rels(relationships)
    if extra:
        result.update(extra)
    return result


def write_package(name: str, entries: dict[str, bytes]) -> None:
    path = ROOT / name
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_STORED) as archive:
        for entry_name in sorted(entries):
            info = zipfile.ZipInfo(entry_name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED
            archive.writestr(info, entries[entry_name])


def write_text_pdf(name: str, placements: list[tuple[int, int, str]]) -> None:
    def literal(value: str) -> str:
        return value.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")

    content = "\n".join(
        f"BT /F1 11 Tf 1 0 0 1 {x} {y} Tm ({literal(text)}) Tj ET"
        for x, y, text in placements
    ).encode("ascii")
    objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
        f"<< /Length {len(content)} >>\nstream\n".encode("ascii") + content + b"\nendstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>",
    ]
    output = bytearray(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n")
    offsets = [0]
    for index, value in enumerate(objects, start=1):
        offsets.append(len(output))
        output.extend(f"{index} 0 obj\n".encode("ascii"))
        output.extend(value)
        output.extend(b"\nendobj\n")
    xref = len(output)
    output.extend(f"xref\n0 {len(offsets)}\n".encode("ascii"))
    output.extend(b"0000000000 65535 f \n")
    for offset in offsets[1:]:
        output.extend(f"{offset:010} 00000 n \n".encode("ascii"))
    output.extend(
        f"trailer\n<< /Size {len(offsets)} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode(
            "ascii"
        )
    )
    (ROOT / name).write_bytes(output)


def patch_encrypted_flag(path: Path) -> None:
    data = bytearray(path.read_bytes())
    for signature, offset in ((b"PK\x03\x04", 6), (b"PK\x01\x02", 8)):
        cursor = 0
        while True:
            index = data.find(signature, cursor)
            if index < 0:
                break
            flags = struct.unpack_from("<H", data, index + offset)[0]
            struct.pack_into("<H", data, index + offset, flags | 0x1)
            cursor = index + 4
    path.write_bytes(data)


def generate() -> None:
    ROOT.mkdir(parents=True, exist_ok=True)
    simple = base_entries(xml_document(paragraph("secure package fixture")))

    zip_slip = dict(simple)
    zip_slip["../escape.xml"] = b"escape"
    write_package("bad-01-zip-slip-entry.docx", zip_slip)

    duplicate = dict(simple)
    duplicate["WORD/DOCUMENT.XML"] = b"duplicate"
    write_package("bad-02-duplicate-casefolded-entry.docx", duplicate)

    write_package("bad-03-encrypted-entry.docx", simple)
    patch_encrypted_flag(ROOT / "bad-03-encrypted-entry.docx")

    write_package("bad-04-missing-content-types.docx", {"word/document.xml": b"<w:document/>"})

    broken_rels = base_entries(
        xml_document(paragraph("broken relationship")),
        [("rIdMissing", "image", "media/missing.png", None)],
    )
    write_package("bad-05-broken-internal-relationship.docx", broken_rels)

    external = base_entries(
        xml_document('<w:p><w:r><w:t>fixture</w:t></w:r>' + image_drawing("rIdExternal") + "</w:p>"),
        [("rIdExternal", "image", "https://example.invalid/image.png", "External")],
    )
    write_package("bad-06-external-image.docx", external)

    empty_table = base_entries(xml_document(table_xml([["filled", ""], ["second", "cell"]])))
    write_package("bad-07-empty-table-cell.docx", empty_table)

    vmerge = base_entries(xml_document(table_xml([["", "top"], ["", "bottom"]], [(0, 0, "continue")])))
    write_package("bad-08-vmerge-without-restart.docx", vmerge)

    floating_textbox = base_entries(
        xml_document("<w:p><w:r><w:drawing><wp:anchor><wp:extent cx=\"1000000\" cy=\"500000\"/><wp:docPr id=\"3\"/><w:txbxContent>" + paragraph("floating text") + "</w:txbxContent></wp:anchor></w:drawing></w:r></w:p>"),
    )
    write_package("bad-09-floating-textbox-without-offset.docx", floating_textbox)

    vml = base_entries(
        xml_document("<w:p><w:pict><v:shape style=\"left:12pt;top:18pt;width:120pt;height:40pt\"><w:txbxContent>" + paragraph("VML text box") + "</w:txbxContent></v:shape></w:pict></w:p>"),
    )
    write_package("bad-10-vml-textbox.docx", vml)

    smartart = base_entries(
        xml_document('<w:p><w:r><w:t>fixture</w:t></w:r>' + smartart_drawing("rIdDiagram") + "</w:p>"),
        [("rIdDiagram", "diagramData", "diagrams/data1.xml", None)],
        {"word/diagrams/data1.xml": b"<dgm:dataModel xmlns:dgm=\"http://schemas.openxmlformats.org/drawingml/2006/diagram\"><dgm:pt t=\"text\"><dgm:t>SmartArt node</dgm:t></dgm:pt></dgm:dataModel>"},
        '<Override PartName="/word/diagrams/data1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.diagramData+xml"/>',
    )
    write_package("bad-11-smartart-without-preview.docx", smartart)

    chart = base_entries(
        xml_document('<w:p><w:r><w:t>fixture</w:t></w:r>' + smartart_drawing("rIdChart", "chart") + "</w:p>"),
        [("rIdChart", "chart", "charts/chart1.xml", None)],
        {"word/charts/chart1.xml": b"<c:chartSpace xmlns:c=\"http://schemas.openxmlformats.org/drawingml/2006/chart\"><c:title><c:tx><c:rich><c:t>Chart title</c:t></c:rich></c:tx></c:title></c:chartSpace>"},
        '<Override PartName="/word/charts/chart1.xml" ContentType="application/vnd.openxmlformats-officedocument.drawingml.chart+xml"/>',
    )
    write_package("bad-12-chart-without-preview.docx", chart)

    columns = base_entries(
        xml_document(paragraph("two column section"), '<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:cols w:num="2" w:space="720" w:sep="1"/></w:sectPr>'),
    )
    write_package("bad-13-two-column-section.docx", columns)

    raw = base_entries(
        xml_document('<w:p><w:r><w:t xml:space="preserve">  lead  </w:t><w:tab/><w:t>tabbed</w:t><w:br w:type="page"/></w:r><w:fldSimple w:instr=" PAGE "><w:r><w:t>3</w:t></w:r></w:fldSimple></w:p>'),
    )
    write_package("bad-14-raw-whitespace-and-breaks.docx", raw)

    render_assist_options = base_entries(
        xml_document(
            paragraph("Questions 21-24")
            + paragraph("Choose FOUR answers from the box.")
            + '<w:p><w:r><w:t>A Alpine survey</w:t><w:tab/><w:t>B Coastal archive</w:t></w:r></w:p>'
            + '<w:p><w:r><w:t xml:space="preserve">C Desert expedition          D Forest census</w:t></w:r></w:p>',
            '<w:sectPr><w:pgSz w:w="12240" w:h="15840"/><w:cols w:num="2" w:space="720"/></w:sectPr>',
        )
    )
    write_package("render-assisted-two-column-options.docx", render_assist_options)
    write_text_pdf(
        "render-assisted-two-column-options.provider-output.pdf",
        [
            (72, 734, "Questions 21-24"),
            (72, 710, "Choose FOUR answers from the box."),
            (72, 660, "A Alpine survey"),
            (340, 660, "B Coastal archive"),
            (72, 636, "C Desert expedition"),
            (340, 636, "D Forest census"),
        ],
    )

    # Keep the five named fixtures used by the original Phase 3 regression tests.
    write_package("docx-external-image.docx", external)
    write_package("docx-floating-text-box.docx", floating_textbox)
    write_package("docx-section-columns.docx", columns)
    write_package("docx-smartart.docx", smartart)
    write_package("docx-table-merged-cells.docx", empty_table)


if __name__ == "__main__":
    generate()
