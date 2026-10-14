"""Small source-backed heading/scan-answer fixture for the native bug regression."""
import io
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[2] / "tmp/pdf-test-deps"))
from PIL import Image, ImageDraw
from reportlab.lib.utils import ImageReader
from reportlab.pdfgen import canvas
import pymupdf

out = pathlib.Path(sys.argv[1]).resolve()
out.parent.mkdir(parents=True, exist_ok=True)
c = canvas.Canvas(str(out), pagesize=(595, 842))
c.setFont("Helvetica-Bold", 16)
c.drawString(50, 790, "READING PASSAGE 1")
c.setFont("Helvetica", 12)
lines = [
    "Village Records",
    "A",
    "Farmers recorded rainfall and harvest dates in notebooks.",
    "These records helped them choose the best time to sow their crops.",
    "The final sentence of paragraph A must remain visible.",
    "B",
    "The village library kept the notebooks for later generations.",
    "Readers compared the old records with modern observations.",
    "The final sentence of paragraph B must remain visible.",
    "Questions 1-2",
    "Choose the correct heading for each paragraph from the list below.",
    "Write the correct number, i or ii, in boxes 1-2 on your answer sheet.",
    "List of Headings",
    "i    Farming records",
    "ii   A village library",
    "1    Paragraph A",
    "2    Paragraph B",
]
for index, line in enumerate(lines):
    c.drawString(50, 750 - index * 30, line)
c.showPage()
image = Image.new("RGB", (800, 1000), "white")
draw = ImageDraw.Draw(image)
draw.text((80, 100), "Answer Key\n\n1 i\n\n2 ii", fill="black", font_size=44)
buffer = io.BytesIO()
image.save(buffer, format="PNG")
c.drawImage(ImageReader(buffer), 50, 110, width=480, height=600)
c.save()
pdf = pymupdf.open(out)
for index, page in enumerate(pdf):
    page.get_pixmap(matrix=pymupdf.Matrix(1, 1)).save(out.with_name(f"{out.stem}-page-{index + 1}.png"))
print(out)
