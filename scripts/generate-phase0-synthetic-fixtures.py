"""Generate the deterministic synthetic fixtures required by Phase 0.

The files are deliberately small and self-contained. They exercise layout and
container boundaries; they are not copies of real IELTS material.
"""

from __future__ import annotations

import html
import io
import zipfile
from pathlib import Path

from PIL import Image, ImageDraw
from pypdf import PdfReader, PdfWriter
from reportlab.lib import colors
from reportlab.lib.pagesizes import A4, letter, landscape
from reportlab.lib.utils import ImageReader
from reportlab.pdfgen import canvas


ROOT = Path(__file__).resolve().parents[1]
SYNTHETIC_ROOT = ROOT / "fixtures" / "golden" / "synthetic"
PDF_ROOT = SYNTHETIC_ROOT / "pdf"
DOCX_ROOT = SYNTHETIC_ROOT / "docx"
ASSET_ROOT = SYNTHETIC_ROOT / "assets"


def xml_escape(value: str) -> str:
    return html.escape(value, quote=True)


def paragraph(text: str, style: str = "Normal") -> str:
    return f'<w:p><w:pPr><w:pStyle w:val="{style}"/></w:pPr><w:r><w:t xml:space="preserve">{xml_escape(text)}</w:t></w:r></w:p>'


def make_image(name: str, label: str, size: tuple[int, int] = (900, 460)) -> Path:
    path = ASSET_ROOT / name
    if path.exists():
        return path
    image = Image.new("RGB", size, "white")
    draw = ImageDraw.Draw(image)
    draw.rectangle((12, 12, size[0] - 12, size[1] - 12), outline="#204060", width=4)
    draw.line((40, size[1] - 70, size[0] // 2, 80, size[0] - 80, size[1] - 120), fill="#cc6633", width=8)
    draw.ellipse((size[0] // 2 - 24, 55, size[0] // 2 + 24, 103), fill="#336699")
    draw.text((36, size[1] - 52), label, fill="#111111")
    path.parent.mkdir(parents=True, exist_ok=True)
    image.save(path, format="PNG", optimize=False)
    return path


def static_pdf_metadata(writer: PdfWriter) -> None:
    writer.add_metadata({
        "/Title": "IELTS Phase 0 Synthetic Fixture",
        "/Author": "IELTS Author Studio Phase 0",
        "/Creator": "generate-phase0-synthetic-fixtures.py",
        "/Producer": "Phase 0 fixture generator",
    })


def finalize_pdf(temp_path: Path, target_path: Path) -> None:
    reader = PdfReader(str(temp_path))
    writer = PdfWriter()
    for page in reader.pages:
        writer.add_page(page)
    static_pdf_metadata(writer)
    target_path.parent.mkdir(parents=True, exist_ok=True)
    with target_path.open("wb") as handle:
        writer.write(handle)
    temp_path.unlink(missing_ok=True)


def draw_wrapped(c: canvas.Canvas, text: str, x: float, y: float, width: float, leading: float = 15) -> float:
    words = text.split()
    line = ""
    for word in words:
        candidate = f"{line} {word}".strip()
        if c.stringWidth(candidate, "Helvetica", 10) > width and line:
            c.drawString(x, y, line)
            y -= leading
            line = word
        else:
            line = candidate
    if line:
        c.drawString(x, y, line)
        y -= leading
    return y


def heading(c: canvas.Canvas, text: str, y: float, x: float = 48) -> float:
    c.setFont("Helvetica-Bold", 14)
    c.drawString(x, y, text)
    c.setFont("Helvetica", 10)
    return y - 24


def draw_questions(c: canvas.Canvas, start: int, end: int, x: float, y: float, kind: str) -> None:
    c.setFont("Helvetica-Bold", 11)
    c.drawString(x, y, f"Questions {start}-{end}")
    c.setFont("Helvetica", 9)
    c.drawString(x, y - 15, kind)
    y -= 36
    for number in range(start, end + 1):
        c.drawString(x, y, f"{number}. The source statement describes a synthetic test condition.")
        y -= 15
        if "completion" in kind.lower():
            c.drawString(x + 18, y, "Answer: ______________________________")
        else:
            c.drawString(x + 18, y, "A   B   C   D")
        y -= 24


def create_pdf(fixture_id: str, kind: str) -> Path:
    target = PDF_ROOT / f"{fixture_id}.pdf"
    temp = target.with_suffix(".tmp.pdf")
    target.parent.mkdir(parents=True, exist_ok=True)
    if kind == "two-column":
        c = canvas.Canvas(str(temp), pagesize=letter)
        heading(c, "Synthetic two-column reading fixture", 748)
        c.line(306, 54, 306, 720)
        draw_wrapped(c, "The left column contains a passage whose lines must be read top to bottom before the right column. This is a controlled reading-order fixture.", 48, 700, 230)
        draw_wrapped(c, "Column two contains question material and must not be merged into the passage text.", 330, 700, 230)
        draw_questions(c, 1, 4, 330, 620, "Do the statements agree with the passage? TRUE / FALSE / NOT GIVEN")
        c.showPage()
        c.save()
    elif kind == "three-column":
        c = canvas.Canvas(str(temp), pagesize=landscape(A4))
        heading(c, "Synthetic three-column matching fixture", 560)
        for x in (48, 280, 512):
            c.rect(x, 80, 190, 430)
            draw_wrapped(c, "Column content stays in its own region. Reading order is part of the expected annotation.", x + 12, 480, 166)
        draw_questions(c, 1, 6, 512, 300, "Match each statement to the correct section.")
        c.showPage()
        c.save()
    elif kind == "rotated-page":
        c = canvas.Canvas(str(temp), pagesize=letter)
        c.setPageRotation(90)
        heading(c, "Synthetic rotated page fixture", 560)
        draw_wrapped(c, "The page rotation metadata is intentional. Text and question coordinates must be normalized before ordering.", 48, 520, 690)
        draw_questions(c, 1, 3, 48, 430, "TRUE / FALSE / NOT GIVEN")
        c.showPage()
        c.save()
    elif kind == "image-only":
        image = make_image("image-only.png", "image-only page; no native text")
        c = canvas.Canvas(str(temp), pagesize=letter)
        c.drawImage(ImageReader(str(image)), 54, 250, width=504, height=258, preserveAspectRatio=True, mask="auto")
        c.showPage()
        c.save()
    elif kind == "hidden-ocr":
        image = make_image("hidden-ocr.png", "visual source")
        c = canvas.Canvas(str(temp), pagesize=letter)
        c.drawImage(ImageReader(str(image)), 54, 260, width=504, height=258, mask="auto")
        c.setFillColor(colors.white)
        c.setFont("Helvetica", 10)
        c.drawString(60, 250, "Hidden OCR layer: Questions 1-3 TRUE FALSE NOT GIVEN")
        c.setFillColor(colors.black)
        c.showPage()
        c.save()
    elif kind == "native-ocr-conflict":
        c = canvas.Canvas(str(temp), pagesize=letter)
        heading(c, "Synthetic native/OCR conflict fixture", 748)
        c.drawString(54, 700, "Visible native text says: The answer is TRUE.")
        c.setFillColor(colors.white)
        c.drawString(54, 680, "OCR layer says: The answer is FALSE.")
        c.setFillColor(colors.black)
        draw_questions(c, 1, 2, 54, 620, "Choose the answer supported by the source.")
        c.showPage()
        c.save()
    elif kind == "broken-font":
        c = canvas.Canvas(str(temp), pagesize=letter)
        heading(c, "Synthetic font and Unicode mapping fixture", 748)
        c.setFont("Helvetica", 11)
        c.drawString(54, 700, "Unicode probe: cafe - resume - naive - replacement glyph: ")
        c.drawString(54, 680, "The source may contain a missing-glyph marker and unusual dash characters.")
        draw_questions(c, 1, 3, 54, 620, "Short answer: write no more than three words.")
        c.showPage()
        c.save()
    elif kind == "mixed-text-image":
        image = make_image("mixed-text-image.png", "embedded figure")
        c = canvas.Canvas(str(temp), pagesize=letter)
        heading(c, "Synthetic mixed text and image fixture", 748)
        c.drawString(54, 710, "The figure belongs to the passage and must remain an asset.")
        c.drawImage(ImageReader(str(image)), 54, 360, width=504, height=258, mask="auto")
        draw_questions(c, 1, 2, 54, 310, "Complete the diagram labels below.")
        c.showPage()
        c.save()
    elif kind == "ruled-table":
        c = canvas.Canvas(str(temp), pagesize=letter)
        heading(c, "Synthetic ruled table completion fixture", 748)
        x, y, row_h, col_w = 54, 650, 42, (150, 160, 150)
        for row in range(5):
            for col in range(3):
                c.rect(x + sum(col_w[:col]), y - row * row_h, col_w[col], row_h)
        c.setFont("Helvetica", 10)
        for index, label in enumerate(("Item", "Evidence", "Answer")):
            c.drawString(x + sum(col_w[:index]) + 10, y + 15, label)
        for row, number in enumerate(range(1, 5), start=1):
            c.drawString(x + 10, y - row * row_h + 15, str(number))
            c.drawString(x + col_w[0] + 10, y - row * row_h + 15, "table evidence")
            c.drawString(x + col_w[0] + col_w[1] + 10, y - row * row_h + 15, "________")
        c.showPage()
        c.save()
    elif kind == "borderless-table":
        c = canvas.Canvas(str(temp), pagesize=letter)
        heading(c, "Synthetic borderless table fixture", 748)
        c.setFont("Helvetica-Bold", 10)
        c.drawString(60, 690, "Item")
        c.drawString(250, 690, "Evidence")
        c.drawString(470, 690, "Response")
        c.setFont("Helvetica", 10)
        for row, number in enumerate(range(1, 5), start=1):
            y = 690 - row * 42
            c.drawString(60, y, str(number))
            c.drawString(250, y, "aligned text without border")
            c.drawString(470, y, "________")
        c.showPage()
        c.save()
    elif kind == "vector-diagram":
        c = canvas.Canvas(str(temp), pagesize=letter)
        heading(c, "Synthetic vector diagram fixture", 748)
        c.setStrokeColor(colors.HexColor("#204060"))
        c.rect(100, 470, 170, 80)
        c.rect(350, 470, 170, 80)
        c.line(270, 510, 350, 510)
        c.setFont("Helvetica", 10)
        c.drawString(138, 510, "Input region")
        c.drawString(390, 510, "Output region")
        c.drawString(160, 430, "Labels are vector text near connected shapes.")
        draw_questions(c, 1, 3, 54, 370, "Label the diagram.")
        c.showPage()
        c.save()
    elif kind == "map-hotspot":
        c = canvas.Canvas(str(temp), pagesize=letter)
        heading(c, "Synthetic map hotspot fixture", 748)
        c.setStrokeColor(colors.HexColor("#306090"))
        c.rect(80, 350, 430, 300)
        c.line(100, 500, 460, 580)
        c.line(100, 430, 460, 390)
        c.setFillColor(colors.HexColor("#993333"))
        for x, y, label in ((160, 540, "A"), (320, 480, "B"), (430, 410, "C")):
            c.circle(x, y, 12, fill=1)
            c.setFillColor(colors.white)
            c.drawCentredString(x, y - 3, label)
            c.setFillColor(colors.HexColor("#993333"))
        c.setFillColor(colors.black)
        draw_questions(c, 1, 4, 54, 310, "Label the locations on the map.")
        c.showPage()
        c.save()
    elif kind == "flowchart":
        c = canvas.Canvas(str(temp), pagesize=letter)
        heading(c, "Synthetic flow-chart completion fixture", 748)
        for index, label in enumerate(("Start", "Process", "Check", "End")):
            y = 620 - index * 80
            c.roundRect(190, y, 220, 42, 8)
            c.drawCentredString(300, y + 16, label)
            if index < 3:
                c.line(300, y, 300, y - 38)
        draw_questions(c, 1, 4, 54, 280, "Complete the flow-chart.")
        c.showPage()
        c.save()
    elif kind == "header-footer":
        c = canvas.Canvas(str(temp), pagesize=letter)
        for page in range(1, 3):
            c.setFont("Helvetica-Bold", 9)
            c.drawString(48, 760, "Synthetic repeating header - should not become passage text")
            c.setFont("Helvetica", 10)
            c.drawString(48, 720, f"Passage body on page {page}; repeated footer is a page decoration.")
            c.line(48, 70, 560, 70)
            c.drawString(48, 52, f"Synthetic footer page {page}")
            if page == 1:
                draw_questions(c, 1, 3, 48, 650, "Complete the summary.")
            c.showPage()
        c.save()
    elif kind == "question-before-passage":
        c = canvas.Canvas(str(temp), pagesize=letter)
        heading(c, "Questions appear before the passage", 748)
        draw_questions(c, 1, 3, 54, 690, "Choose the correct heading for each paragraph.")
        c.showPage()
        heading(c, "Reading Passage 1", 748)
        draw_wrapped(c, "The passage is intentionally placed after the question list. Semantic role, not page order, determines the final authoring layout.", 54, 700, 500)
        c.showPage()
        c.save()
    else:
        raise ValueError(f"unknown PDF fixture kind: {kind}")
    finalize_pdf(temp, target)
    return target


CONTENT_TYPES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
  <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
  <Default Extension="xml" ContentType="application/xml"/>
  <Default Extension="png" ContentType="image/png"/>
  <Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
  <Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
  <Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/>
</Types>"""

ROOT_RELS = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
  <Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/>
</Relationships>"""

STYLES = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style></w:styles>"""

CORE = """<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>IELTS Phase 0 Synthetic Fixture</dc:title><dc:creator>IELTS Author Studio</dc:creator></cp:coreProperties>"""


def doc_relationships(extra: str = "") -> str:
    return f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{extra}</Relationships>"""


def write_docx(target: Path, document: str, extra_files: dict[str, bytes | str] | None = None, relationships: str = "") -> None:
    target.parent.mkdir(parents=True, exist_ok=True)
    extra_files = extra_files or {}
    entries: dict[str, bytes | str] = {
        "[Content_Types].xml": CONTENT_TYPES,
        "_rels/.rels": ROOT_RELS,
        "word/document.xml": document,
        "word/styles.xml": STYLES,
        "word/_rels/document.xml.rels": doc_relationships(relationships),
        "docProps/core.xml": CORE,
        **extra_files,
    }
    with zipfile.ZipFile(target, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for name, content in sorted(entries.items()):
            info = zipfile.ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, content.encode("utf-8") if isinstance(content, str) else content)


def create_docx(fixture_id: str, kind: str) -> Path:
    target = DOCX_ROOT / f"{fixture_id}.docx"
    body = [paragraph(f"Synthetic {kind} DOCX fixture.")]
    relationships = ""
    extra_files: dict[str, bytes | str] = {}
    if kind == "floating-text-box":
        body.append("""<w:p><w:r><w:pict xmlns:v="urn:schemas-microsoft-com:vml" xmlns:o="urn:schemas-microsoft-com:office:office"><v:shape id="TextBox1" style="position:absolute;width:300pt;height:80pt"><v:textbox><w:txbxContent><w:p><w:r><w:t>Questions 1-2 are inside a floating text box.</w:t></w:r></w:p></w:txbxContent></v:textbox></v:shape></w:pict></w:r></w:p>""")
        body.append(paragraph("1. Floating prompt answer: __________"))
        body.append(paragraph("2. Floating prompt answer: __________"))
    elif kind == "external-image":
        relationships = '<Relationship Id="rIdExternalImage" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="https://example.invalid/missing-image.png" TargetMode="External"/>'
        body.append("""<w:p><w:r><w:drawing xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><wp:inline><wp:extent cx="3000000" cy="1500000"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:blipFill><a:blip r:embed="rIdExternalImage"/></pic:blipFill></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>""")
        body.append(paragraph("External image must be reported as missing; do not publish silently."))
    elif kind == "smartart":
        relationships = '<Relationship Id="rIdDiagramData" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramData" Target="diagrams/data1.xml"/><Relationship Id="rIdDiagramDrawing" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/diagramDrawing" Target="diagrams/drawing1.xml"/>'
        extra_files["word/diagrams/data1.xml"] = '<dgm:dataModel xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:ptLst><dgm:pt modelId="1"><dgm:spPr/></dgm:pt></dgm:ptLst></dgm:dataModel>'
        extra_files["word/diagrams/drawing1.xml"] = '<dgm:drawing xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:relIds/></dgm:drawing>'
        body.append("""<w:p><w:r><w:drawing xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:dgm="http://schemas.openxmlformats.org/drawingml/2006/diagram"><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/diagram"><dgm:relIds r:dm="rIdDiagramData" r:lo="rIdDiagramDrawing"/></a:graphicData></a:graphic></w:drawing></w:r></w:p>""")
        body.append(paragraph("Questions 1-3 label nodes in the SmartArt diagram."))
    elif kind == "section-columns":
        body.append(paragraph("Column one contains the passage. Column two contains the question instruction."))
        body.append(paragraph("Questions 1-4: Match each statement to a column."))
    elif kind == "table-merged-cells":
        body.append("""<w:tbl xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:tblGrid><w:gridCol w:w="2400"/><w:gridCol w:w="2400"/></w:tblGrid><w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>Merged heading cell</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>1</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>__________</w:t></w:r></w:p></w:tc></w:tr><w:tr><w:tc><w:p><w:r><w:t>2</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>__________</w:t></w:r></w:p></w:tc></w:tr></w:tbl>""")
        body.append(paragraph("Questions 1-4 complete the merged-cell table."))
    else:
        raise ValueError(f"unknown DOCX fixture kind: {kind}")
    sect_pr = "<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\"/>"
    if kind == "section-columns":
        sect_pr += '<w:cols w:num="2" w:space="720"/>'
    sect_pr += "</w:sectPr>"
    document = f"""<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><w:body>{''.join(body)}{sect_pr}</w:body></w:document>"""
    write_docx(target, document, extra_files, relationships)
    return target


PDF_SPECS = [
    ("pdf-two-column", "two-column"),
    ("pdf-three-column", "three-column"),
    ("pdf-rotated-page", "rotated-page"),
    ("pdf-image-only", "image-only"),
    ("pdf-hidden-ocr", "hidden-ocr"),
    ("pdf-native-ocr-conflict", "native-ocr-conflict"),
    ("pdf-broken-font", "broken-font"),
    ("pdf-mixed-text-image", "mixed-text-image"),
    ("pdf-ruled-table", "ruled-table"),
    ("pdf-borderless-table", "borderless-table"),
    ("pdf-vector-diagram", "vector-diagram"),
    ("pdf-map-hotspot", "map-hotspot"),
    ("pdf-flowchart", "flowchart"),
    ("pdf-header-footer", "header-footer"),
    ("pdf-question-before-passage", "question-before-passage"),
]

DOCX_SPECS = [
    ("docx-floating-text-box", "floating-text-box"),
    ("docx-external-image", "external-image"),
    ("docx-smartart", "smartart"),
    ("docx-section-columns", "section-columns"),
    ("docx-table-merged-cells", "table-merged-cells"),
]


def main() -> None:
    for fixture_id, kind in PDF_SPECS:
        path = create_pdf(fixture_id, kind)
        print(f"generated {path.relative_to(ROOT).as_posix()}")
    for fixture_id, kind in DOCX_SPECS:
        path = create_docx(fixture_id, kind)
        print(f"generated {path.relative_to(ROOT).as_posix()}")


if __name__ == "__main__":
    main()
