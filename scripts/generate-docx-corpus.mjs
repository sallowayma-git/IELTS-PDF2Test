import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const crcTable = Array.from({ length: 256 }, (_, index) => {
  let value = index;
  for (let bit = 0; bit < 8; bit += 1) value = (value & 1) ? (0xedb88320 ^ (value >>> 1)) : (value >>> 1);
  return value >>> 0;
});
const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const defaultCorpusDir = resolveDefaultCorpusDir();
const defaultOutDir = path.join(os.tmpdir(), "pdf2test-word-corpus");
const args = parseArgs(process.argv.slice(2));

if (args.help === true || args.h === true) {
  printUsageAndExit(0);
}
if (args.selfTest === true || args["self-test"] === true) {
  runInternalSelfChecks();
  console.log("generate-docx-corpus self-test passed");
  process.exit(0);
}

const pdfDir = path.resolve(String(args.pdfDir ?? args["pdf-dir"] ?? defaultCorpusDir));
const outDir = path.resolve(String(args.outDir ?? args["out-dir"] ?? defaultOutDir));
const optionLayout = String(args.optionLayout ?? args["option-layout"] ?? "table");
const numberingMode = String(args.numberingMode ?? args["numbering-mode"] ?? "explicit");
const sampleSize = parseOptionalPositiveInteger(args.sample ?? args.limit, "--sample");
const seed = String(args.seed ?? "pdf2test-word-corpus");
const filter = String(args.filter ?? "").normalize("NFKC").toLowerCase();
const verify = parseBoolean(args.verify, false);
const strict = parseBoolean(args.strict, false);
const overwrite = parseBoolean(args.overwrite, false);
const skipBuild = parseBoolean(args.skipBuild ?? args["skip-build"], false);
const expectZeroFilter = String(args.expectZero ?? args["expect-zero"] ?? "").normalize("NFKC").toLowerCase();
const cli = path.resolve(String(args.cli ?? path.join(repoRoot, "src-tauri", "target", "debug", cliBinaryName())));

if (!new Set(["table", "paragraph"]).has(optionLayout)) {
  fail(`--option-layout must be "table" or "paragraph", received: ${optionLayout}`);
}
if (!new Set(["explicit", "word"]).has(numberingMode)) {
  fail(`--numbering-mode must be "explicit" or "word", received: ${numberingMode}`);
}

assertReadableDirectory(pdfDir, "--pdf-dir");
fs.mkdirSync(outDir, { recursive: true });
ensureCliBuilt(cli, skipBuild);

const allPdfs = listFilesRecursively(pdfDir)
  .filter((filePath) => filePath.toLowerCase().endsWith(".pdf"))
  .filter((filePath) => !filter || canonicalRelativePath(path.relative(pdfDir, filePath)).toLowerCase().includes(filter))
  .sort((left, right) => compareCanonicalPaths(
    path.relative(pdfDir, left),
    path.relative(pdfDir, right)
  ));
if (allPdfs.length === 0) {
  fail(`no PDF files found under: ${pdfDir}`);
}

const selectedPdfs = sampleSize == null || sampleSize >= allPdfs.length
  ? allPdfs
  : shuffle(allPdfs, seed).slice(0, sampleSize).sort((left, right) => compareCanonicalPaths(
    path.relative(pdfDir, left),
    path.relative(pdfDir, right)
  ));

const manifest = {
  schemaVersion: "PdfToDocxCorpusManifestV2",
  generatedAt: new Date().toISOString(),
  sourceDir: pdfDir,
  outputDir: outDir,
  generator: {
    script: path.relative(repoRoot, fileURLToPath(import.meta.url)).replaceAll(path.sep, "/"),
    cli,
    optionLayout,
    numberingMode,
    verify,
    seed,
    sampleSize,
    selectionStrategy: sampleSize == null || sampleSize >= allPdfs.length ? "all" : "seeded-random"
  },
  totalPdfCount: allPdfs.length,
  selectedCount: selectedPdfs.length,
  entries: []
};

for (const [index, pdfPath] of selectedPdfs.entries()) {
  const relativeSource = canonicalRelativePath(path.relative(pdfDir, pdfPath));
  const declaredZeroGroupSource = Boolean(zeroQuestionGroupReason(relativeSource, expectZeroFilter));
  const docxName = outputName(relativeSource, numberingMode, optionLayout, declaredZeroGroupSource);
  const docxPath = path.join(outDir, docxName);
  console.log(`[docx-corpus] ${index + 1}/${selectedPdfs.length} ${relativeSource}`);

  const entry = {
    sourcePdf: pdfPath,
    sourceRelativePath: relativeSource,
    outputDocx: docxPath,
    outputRelativePath: docxName,
    status: "pending"
  };
  manifest.entries.push(entry);

  try {
    entry.sourceSha256 = sha256File(pdfPath);
    const sourcePayload = runReadingSourceCli(cli, pdfPath);
    const sourceDocument = requireDocumentIr(sourcePayload, pdfPath);
    const sourceMarkers = collectMarkerStats(sourceDocument);
    const sourceAuthoring = collectAuthoringSignature(sourcePayload);
    const sourceAssets = collectAssetSummary(sourceDocument);
    const sourceBlocks = collectBlockSummary(sourceDocument);
    entry.expectation = deriveQuestionGroupExpectation(relativeSource, sourceAuthoring, expectZeroFilter);
    const built = buildDocx(sourceDocument, {
      title: path.basename(pdfPath, path.extname(pdfPath)),
      sourcePdf: relativeSource,
      optionLayout,
      numberingMode
    });
    const expectedDocxSha256 = sha256Buffer(built.buffer);
    entry.source = {
      parserProvider: sourceDocument?.parser?.provider ?? null,
      parserWarnings: sourceDocument?.parser?.warnings ?? [],
      pageCount: sourceDocument.pages?.length ?? 0,
      blockCount: countDocumentBlocks(sourceDocument),
      markers: sourceMarkers,
      authoring: {
        ...summarizeAuthoringSignature(sourceAuthoring),
        groups: sourceAuthoring.groups
      },
      assets: sourceAssets,
      blocks: sourceBlocks,
      expectationMatch: sourceAuthoring.groups.length === entry.expectation.expectedQuestionGroupCount
    };
    entry.docx = {
      ...built.stats,
      omittedAssetCount: sourceAssets.total
    };
    if (sourceAssets.total > 0) {
      entry.manualReviewReasons = ["source images/assets are not embedded in the derived DOCX"];
    }

    if (fs.existsSync(docxPath) && !overwrite) {
      const existingDocxSha256 = sha256File(docxPath);
      if (existingDocxSha256 !== expectedDocxSha256) {
        throw new Error(`existing DOCX differs from deterministic output; rerun with --overwrite: ${docxPath}`);
      }
      entry.status = "skipped_existing";
      entry.docxSha256 = existingDocxSha256;
      entry.docxBytes = fs.statSync(docxPath).size;
    } else {
      fs.writeFileSync(docxPath, built.buffer);
      entry.status = "generated";
      entry.docxSha256 = expectedDocxSha256;
      entry.docxBytes = built.buffer.length;
    }

    if (verify) {
      const verifiedPayload = runReadingSourceCli(cli, docxPath);
      const verifiedDocument = requireDocumentIr(verifiedPayload, docxPath);
      const verifiedMarkers = collectMarkerStats(verifiedDocument);
      const markerCoverage = compareMarkerStats(sourceMarkers, verifiedMarkers);
      const verifiedAuthoring = collectAuthoringSignature(verifiedPayload);
      const semanticComparison = compareAuthoringSignatures(
        sourceAuthoring,
        verifiedAuthoring
      );
      const expectationMatch = verifiedAuthoring.groups.length === entry.expectation.expectedQuestionGroupCount;
      const provider = verifiedDocument?.parser?.provider ?? null;
      entry.verification = {
        ok: String(provider).includes("docx") && markerCoverage.ratio >= 0.98 && semanticComparison.ok && expectationMatch,
        parserProvider: provider,
        parserWarnings: verifiedDocument?.parser?.warnings ?? [],
        pageCount: verifiedDocument.pages?.length ?? 0,
        blockCount: countDocumentBlocks(verifiedDocument),
        questionGroupCount: extractReadingSource(verifiedPayload)?.questionGroups?.length ?? 0,
        blocks: collectBlockSummary(verifiedDocument),
        numbering: collectRoundTripNumbering(verifiedDocument),
        markers: verifiedMarkers,
        markerCoverage,
        semanticComparison,
        authoring: {
          ...summarizeAuthoringSignature(verifiedAuthoring),
          groups: verifiedAuthoring.groups
        },
        expectationMatch
      };
    }
  } catch (error) {
    entry.status = "failed";
    entry.error = error instanceof Error ? error.message : String(error);
    console.error(`[docx-corpus] failed: ${relativeSource}: ${entry.error}`);
  }
}

manifest.completedAt = new Date().toISOString();
manifest.summary = summarizeManifest(manifest.entries);
const serializedManifest = `${JSON.stringify(manifest, null, 2)}\n`;
const manifestPath = path.join(outDir, variantManifestName(numberingMode, optionLayout));
const latestManifestPath = path.join(outDir, "manifest.json");
fs.writeFileSync(manifestPath, serializedManifest);
fs.writeFileSync(latestManifestPath, serializedManifest);
console.log(JSON.stringify({ ...manifest.summary, manifestPath, latestManifestPath }, null, 2));

if (strict && (
  manifest.summary.failedCount > 0
  || manifest.summary.verificationFailedCount > 0
  || manifest.summary.sourceExpectationFailedCount > 0
)) {
  process.exit(1);
}

function resolveDefaultCorpusDir() {
  if (process.env.PDF2TEST_PDF_CORPUS) return process.env.PDF2TEST_PDF_CORPUS;
  const windowsCorpus = "E:\\tmp\\PDF";
  if (fs.existsSync(windowsCorpus)) return windowsCorpus;
  return path.join(repoRoot, "fixtures", "parser");
}

function ensureCliBuilt(cliPath, skipBuildValue) {
  if (!skipBuildValue) {
    const build = spawnSync("cargo", ["build", "--manifest-path", path.join(repoRoot, "src-tauri", "Cargo.toml")], {
      cwd: repoRoot,
      stdio: "inherit"
    });
    if (build.status !== 0) process.exit(build.status ?? 1);
  }
  if (!fs.existsSync(cliPath)) {
    fail(`CLI binary not found: ${cliPath}. Run without --skip-build or pass --cli.`);
  }
}

function runReadingSourceCli(cliPath, sourcePath) {
  const temporaryOutput = path.join(
    os.tmpdir(),
    `pdf2test-docx-corpus-${process.pid}-${crypto.randomUUID()}.json`
  );
  try {
    const result = spawnSync(cliPath, ["--generate-reading-source", sourcePath, "--out", temporaryOutput], {
      cwd: repoRoot,
      encoding: "utf8",
      maxBuffer: 32 * 1024 * 1024
    });
    if (result.status !== 0) {
      const detail = result.stderr?.trim() || result.stdout?.trim() || `exit_${result.status}`;
      throw new Error(`reading_source_cli_failed:${detail}`);
    }
    return JSON.parse(fs.readFileSync(temporaryOutput, "utf8"));
  } finally {
    if (fs.existsSync(temporaryOutput)) fs.unlinkSync(temporaryOutput);
  }
}

function requireDocumentIr(payload, sourcePath) {
  const documentIr = payload?.documentIr ?? payload?.document_ir;
  if (!documentIr || !Array.isArray(documentIr.pages)) {
    throw new Error(`generated payload has no DocumentIR pages: ${sourcePath}`);
  }
  return documentIr;
}

function extractReadingSource(payload) {
  return payload?.readingSource ?? payload?.reading_source ?? payload;
}

function buildDocx(documentIr, options) {
  const modeled = modelDocument(documentIr, options);
  const createdAt = new Date("2000-01-01T00:00:00.000Z");
  const files = [
    ["[Content_Types].xml", contentTypesXml(modeled.numbering.length > 0)],
    ["_rels/.rels", packageRelationshipsXml()],
    ["docProps/core.xml", corePropertiesXml(options.title, options.sourcePdf, createdAt)],
    ["docProps/app.xml", appPropertiesXml()],
    ["word/document.xml", documentXml(modeled.items)],
    ["word/_rels/document.xml.rels", documentRelationshipsXml(modeled.numbering.length > 0)],
    ["word/styles.xml", stylesXml()]
  ];
  if (modeled.numbering.length > 0) files.push(["word/numbering.xml", numberingXml(modeled.numbering)]);
  const zipFiles = files.map(([name, content]) => ({ name, data: Buffer.from(content, "utf8") }));
  return {
    buffer: createZip(zipFiles, createdAt),
    stats: modeled.stats
  };
}

function modelDocument(documentIr, options) {
  const items = [];
  const pages = Array.isArray(documentIr.pages) ? documentIr.pages : [];
  for (const [pageOffset, page] of pages.entries()) {
    if (pageOffset > 0) items.push({ type: "pageBreak" });
    const blocks = Array.isArray(page?.blocks) ? page.blocks.slice() : [];
    blocks.sort((left, right) => blockOrdinal(left) - blockOrdinal(right));
    for (const block of blocks) {
      const table = tableFromBlock(block);
      if (table) {
        items.push({ type: "table", rows: table, source: "document-ir" });
        continue;
      }
      const text = cleanText(block?.text ?? stripHtml(block?.html ?? ""));
      if (!text) continue;
      for (const line of text.split(/\n+/).map((value) => value.trim()).filter(Boolean)) {
        items.push({
          type: "paragraph",
          text: line,
          style: paragraphStyle(line, block),
          roleHint: block?.roleHint ?? null,
          pageIndex: page?.pageIndex ?? pageOffset + 1
        });
      }
    }
  }

  const structuredItems = options.optionLayout === "table" ? groupMarkerRuns(items) : items;
  const numbered = options.numberingMode === "word"
    ? applyWordNumbering(structuredItems)
    : { items: structuredItems, numbering: [] };
  return {
    items: numbered.items,
    numbering: numbered.numbering,
    stats: {
      sourcePageCount: pages.length,
      paragraphCount: numbered.items.filter((item) => item.type === "paragraph").length,
      tableCount: numbered.items.filter((item) => item.type === "table").length,
      optionTableCount: numbered.items.filter((item) => item.type === "table" && item.source === "marker-run").length,
      pageBreakCount: numbered.items.filter((item) => item.type === "pageBreak").length,
      optionLayout: options.optionLayout,
      numberingMode: options.numberingMode,
      numberingInstanceCount: numbered.numbering.length,
      numberingFormats: Array.from(new Set(numbered.numbering.map((item) => item.format))),
      numberingDefinitions: numbered.numbering.map((item) => ({
        format: item.format,
        startOverride: item.start
      }))
    }
  };
}

function tableFromBlock(block) {
  const table = block?.table;
  if (table && Number.isInteger(table.rows) && Number.isInteger(table.cols) && Array.isArray(table.cells)) {
    const rows = Array.from({ length: table.rows }, () => []);
    for (const cell of table.cells) {
      const rowIndex = Number(cell.row ?? 0);
      if (!rows[rowIndex]) rows[rowIndex] = [];
      rows[rowIndex].push({
        col: Number(cell.col ?? rows[rowIndex].length),
        text: cleanText(cell.text ?? ""),
        colSpan: Math.max(1, Number(cell.colSpan ?? cell.col_span ?? 1))
      });
    }
    return rows.map((row) => row.sort((left, right) => left.col - right.col));
  }
  if (typeof block?.html === "string" && /<table\b/i.test(block.html)) {
    const rows = [];
    for (const rowMatch of block.html.matchAll(/<tr\b[^>]*>([\s\S]*?)<\/tr>/gi)) {
      const row = [];
      for (const cellMatch of rowMatch[1].matchAll(/<(?:td|th)\b([^>]*)>([\s\S]*?)<\/(?:td|th)>/gi)) {
        const span = Number(cellMatch[1].match(/colspan=["']?(\d+)/i)?.[1] ?? 1);
        row.push({ text: cleanText(stripHtml(cellMatch[2])), colSpan: Math.max(1, span) });
      }
      if (row.length > 0) rows.push(row);
    }
    if (rows.length > 0) return rows;
  }
  return null;
}

function groupMarkerRuns(items) {
  const output = [];
  for (let index = 0; index < items.length;) {
    const first = items[index];
    const firstMarker = first.type === "paragraph" ? parseOptionMarker(first.text) : null;
    if (!firstMarker || first.roleHint === "passage") {
      output.push(first);
      index += 1;
      continue;
    }

    const run = [];
    let cursor = index;
    while (cursor < items.length) {
      const candidate = items[cursor];
      const marker = candidate.type === "paragraph" ? parseOptionMarker(candidate.text) : null;
      if (!marker || marker.family !== firstMarker.family || candidate.roleHint === "passage" || candidate.pageIndex !== first.pageIndex) break;
      run.push({ candidate, marker });
      cursor += 1;
    }

    const distinctMarkers = new Set(run.map(({ marker }) => marker.marker.toUpperCase()));
    if (run.length >= 2 && distinctMarkers.size === run.length) {
      output.push({
        type: "table",
        source: "marker-run",
        markerFamily: firstMarker.family,
        rows: run.map(({ marker }) => [
          { text: marker.marker, bold: true },
          { text: marker.content }
        ])
      });
      index = cursor;
    } else {
      output.push(first);
      index += 1;
    }
  }
  return output;
}

function applyWordNumbering(items) {
  const numbering = [];
  let nextNumId = 1;
  let activeQuestions = null;
  const transformed = items.map((item) => {
    if (item.type === "paragraph" && item.style === "IELTSQuestion") {
      const match = cleanText(item.text).match(/^(\d{1,3})(?:[.)]|\s+)\s*([\s\S]*)$/);
      if (match) {
        const questionNumber = Number(match[1]);
        if (!activeQuestions || questionNumber !== activeQuestions.last + 1) {
          activeQuestions = { numId: nextNumId, last: questionNumber };
          numbering.push({ numId: nextNumId, format: "decimal", start: questionNumber });
          nextNumId += 1;
        } else {
          activeQuestions.last = questionNumber;
        }
        return {
          ...item,
          text: match[2] || "\u200c",
          numbering: { numId: activeQuestions.numId, level: 0 }
        };
      }
    }

    if (item.type === "table" && item.source === "marker-run" && new Set(["alpha", "roman"]).has(item.markerFamily)) {
      const format = item.markerFamily === "alpha" ? "upperLetter" : "lowerRoman";
      const markerStarts = item.rows.map((row) => markerOrdinal(row?.[0]?.text ?? "", item.markerFamily));
      const consecutive = markerStarts.every((start, index) => index === 0 || start === markerStarts[index - 1] + 1);
      if (!consecutive) {
        // A marker-run row may itself contain later inline markers. For
        // example, source rows A/E/F/G can represent an A-H bank when B-D and
        // H remain in the value cells. A shared Word list would silently
        // renumber those visible row labels as A/B/C/D. Give each sparse row
        // its own start override so the generated DOCX preserves the source
        // evidence exactly.
        return {
          ...item,
          rows: item.rows.map((row, rowIndex) => {
            const numId = nextNumId;
            nextNumId += 1;
            numbering.push({ numId, format, start: markerStarts[rowIndex] });
            return row.map((cell, cellIndex) => cellIndex === 0
              ? { ...cell, text: "\u200c", numbering: { numId, level: 0 } }
              : cell);
          })
        };
      }

      const numId = nextNumId;
      nextNumId += 1;
      numbering.push({ numId, format, start: markerStarts[0] });
      return {
        ...item,
        rows: item.rows.map((row) => row.map((cell, cellIndex) => cellIndex === 0
          ? { ...cell, text: "\u200c", numbering: { numId, level: 0 } }
          : cell))
      };
    }
    return item;
  });
  return { items: transformed, numbering };
}

function runInternalSelfChecks() {
  const sparse = applyWordNumbering([{
    type: "table",
    source: "marker-run",
    markerFamily: "alpha",
    rows: [
      [{ text: "A" }, { text: "natural evolution B creative thought C indigenous plants D trout" }],
      [{ text: "E" }, { text: "pollution" }],
      [{ text: "F" }, { text: "restoration" }],
      [{ text: "G" }, { text: "native fish H extinction" }]
    ]
  }]);
  assert.deepEqual(sparse.numbering.map(({ start }) => start), [1, 5, 6, 7]);
  assert.equal(new Set(sparse.items[0].rows.map((row) => row[0].numbering.numId)).size, 4);

  const contiguous = applyWordNumbering([{
    type: "table",
    source: "marker-run",
    markerFamily: "alpha",
    rows: ["A", "B", "C", "D"].map((marker) => [
      { text: marker },
      { text: `option ${marker}` }
    ])
  }]);
  assert.deepEqual(contiguous.numbering.map(({ start }) => start), [1]);
  assert.equal(new Set(contiguous.items[0].rows.map((row) => row[0].numbering.numId)).size, 1);
}

function markerOrdinal(marker, family) {
  if (family === "alpha") return Math.max(1, String(marker).toUpperCase().charCodeAt(0) - 64);
  const values = { i: 1, ii: 2, iii: 3, iv: 4, v: 5, vi: 6, vii: 7, viii: 8, ix: 9, x: 10, xi: 11, xii: 12 };
  return values[String(marker).toLowerCase()] ?? 1;
}

function parseOptionMarker(text) {
  const value = cleanText(text);
  const roman = value.match(/^(i|ii|iii|iv|v|vi|vii|viii|ix|x|xi|xii)(?:[.)]|\s+)\s*(\S[\s\S]*)$/i);
  if (roman) return { family: "roman", marker: roman[1], content: roman[2] };
  const alpha = value.match(/^([A-H])(?:[.)]|\s+)\s*(\S[\s\S]*)$/);
  if (alpha) return { family: "alpha", marker: alpha[1], content: alpha[2] };
  const judgment = value.match(/^(TRUE|FALSE|NOT GIVEN|YES|NO)(?:[.)]|\s+)\s*(\S[\s\S]*)$/i);
  if (judgment) return { family: "judgment", marker: judgment[1].toUpperCase(), content: judgment[2] };
  return null;
}

function paragraphStyle(text, block) {
  if (/^READING PASSAGE\s+\d+/i.test(text)) return "Heading1";
  if (/^Questions?\s+\d+\s*[-–—]\s*\d+/i.test(text)) return "Heading2";
  if (/^(List of (?:Headings|Options)|Example|Answer Key|Answers?)\b/i.test(text)) return "Heading3";
  if (block?.blockType === "header") return "Heading3";
  if (isInstruction(text)) return "IELTSInstruction";
  if (/^\d{1,3}(?:[.)]|\s+)/.test(text)) return "IELTSQuestion";
  if (block?.roleHint !== "passage" && parseOptionMarker(text)) return "IELTSOption";
  return "Normal";
}

function isInstruction(text) {
  return /^(?:Choose|Complete|Do the following|Write|Match|Classify|Label|Select|Which|Reading Passage .+ has|The passage has|Look at|Using NO MORE THAN|Answer the questions|Questions? \d)/i.test(text);
}

function documentXml(items) {
  const body = items.map((item) => {
    if (item.type === "pageBreak") return "<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>";
    if (item.type === "table") return tableXml(item.rows);
    return paragraphXml(item.text, item.style, { numbering: item.numbering });
  }).join("");
  return `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>${body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1134" w:right="1134" w:bottom="1134" w:left="1134" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr></w:body>
</w:document>`;
}

function paragraphXml(text, style = "Normal", options = {}) {
  const runs = [runXml(cleanText(text), { bold: options.bold === true || style === "IELTSQuestion" })];
  const numbering = options.numbering
    ? `<w:numPr><w:ilvl w:val="${Number(options.numbering.level ?? 0)}"/><w:numId w:val="${Number(options.numbering.numId)}"/></w:numPr>`
    : "";
  return `<w:p><w:pPr><w:pStyle w:val="${xmlEscape(style)}"/>${numbering}</w:pPr>${runs.join("")}</w:p>`;
}

function runXml(text, options = {}) {
  const value = stripInvalidXmlCharacters(String(text));
  if (!value) return "<w:r><w:t></w:t></w:r>";
  const properties = options.bold ? "<w:rPr><w:b/></w:rPr>" : "";
  const fragments = value.split(/(\t)/).map((fragment) => {
    if (fragment === "\t") return "<w:tab/>";
    const preserve = /^\s|\s$|\s{2,}/.test(fragment) ? " xml:space=\"preserve\"" : "";
    return `<w:t${preserve}>${xmlEscape(fragment)}</w:t>`;
  }).join("");
  return `<w:r>${properties}${fragments}</w:r>`;
}

function tableXml(rows) {
  const maxColumns = Math.max(1, ...rows.map((row) => row.reduce((sum, cell) => sum + Math.max(1, Number(cell.colSpan ?? 1)), 0)));
  const grid = Array.from({ length: maxColumns }, () => "<w:gridCol w:w=\"3000\"/>").join("");
  const body = rows.map((row) => `<w:tr>${row.map((cell) => {
    const colSpan = Math.max(1, Number(cell.colSpan ?? 1));
    const span = colSpan > 1 ? `<w:gridSpan w:val="${colSpan}"/>` : "";
    const style = cell.bold ? "IELTSOption" : "Normal";
    return `<w:tc><w:tcPr><w:tcW w:w="0" w:type="auto"/>${span}</w:tcPr>${paragraphXml(cell.text ?? "", style, { bold: cell.bold, numbering: cell.numbering })}</w:tc>`;
  }).join("")}</w:tr>`).join("");
  return `<w:tbl><w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="0" w:type="auto"/><w:tblLook w:val="04A0" w:firstRow="1" w:lastRow="0" w:firstColumn="1" w:lastColumn="0" w:noHBand="0" w:noVBand="1"/></w:tblPr><w:tblGrid>${grid}</w:tblGrid>${body}</w:tbl>`;
}

function stylesXml() {
  return `<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Arial" w:hAnsi="Arial"/><w:sz w:val="22"/><w:szCs w:val="22"/></w:rPr></w:rPrDefault><w:pPrDefault/></w:docDefaults>
  <w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>
  <w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="9"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="240" w:after="120"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="32"/><w:szCs w:val="32"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="9"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="200" w:after="100"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:sz w:val="28"/><w:szCs w:val="28"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:uiPriority w:val="9"/><w:qFormat/><w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before="160" w:after="80"/><w:outlineLvl w:val="2"/></w:pPr><w:rPr><w:b/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:style>
  <w:style w:type="paragraph" w:customStyle="1" w:styleId="IELTSInstruction"><w:name w:val="IELTS Instruction"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="80" w:after="80"/></w:pPr><w:rPr><w:i/></w:rPr></w:style>
  <w:style w:type="paragraph" w:customStyle="1" w:styleId="IELTSQuestion"><w:name w:val="IELTS Question"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="60" w:after="40"/></w:pPr></w:style>
  <w:style w:type="paragraph" w:customStyle="1" w:styleId="IELTSOption"><w:name w:val="IELTS Option"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:before="20" w:after="20"/></w:pPr></w:style>
  <w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:uiPriority w:val="99"/><w:semiHidden/><w:unhideWhenUsed/><w:qFormat/></w:style>
  <w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:basedOn w:val="TableNormal"/><w:uiPriority w:val="59"/><w:qFormat/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:space="0" w:color="B7B7B7"/><w:left w:val="single" w:sz="4" w:space="0" w:color="B7B7B7"/><w:bottom w:val="single" w:sz="4" w:space="0" w:color="B7B7B7"/><w:right w:val="single" w:sz="4" w:space="0" w:color="B7B7B7"/><w:insideH w:val="single" w:sz="4" w:space="0" w:color="D9D9D9"/><w:insideV w:val="single" w:sz="4" w:space="0" w:color="D9D9D9"/></w:tblBorders></w:tblPr></w:style>
</w:styles>`;
}

function contentTypesXml(hasNumbering) {
  const numbering = hasNumbering
    ? `<Override PartName="/word/numbering.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml"/>`
    : "";
  return `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>${numbering}<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/><Override PartName="/docProps/app.xml" ContentType="application/vnd.openxmlformats-officedocument.extended-properties+xml"/></Types>`;
}

function packageRelationshipsXml() {
  return `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/><Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/extended-properties" Target="docProps/app.xml"/></Relationships>`;
}

function documentRelationshipsXml(hasNumbering) {
  const numbering = hasNumbering
    ? `<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering" Target="numbering.xml"/>`
    : "";
  return `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>${numbering}</Relationships>`;
}

function numberingXml(instances) {
  const abstracts = [
    { id: 0, format: "decimal", text: "%1." },
    { id: 1, format: "upperLetter", text: "%1." },
    { id: 2, format: "lowerRoman", text: "%1" }
  ];
  const abstractXml = abstracts.map((item) => `<w:abstractNum w:abstractNumId="${item.id}"><w:multiLevelType w:val="singleLevel"/><w:lvl w:ilvl="0"><w:start w:val="1"/><w:numFmt w:val="${item.format}"/><w:lvlText w:val="${item.text}"/><w:lvlJc w:val="left"/><w:pPr><w:tabs><w:tab w:val="num" w:pos="720"/></w:tabs><w:ind w:left="720" w:hanging="360"/></w:pPr></w:lvl></w:abstractNum>`).join("");
  const formatId = { decimal: 0, upperLetter: 1, lowerRoman: 2 };
  const instanceXml = instances.map((item) => `<w:num w:numId="${item.numId}"><w:abstractNumId w:val="${formatId[item.format]}"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="${item.start}"/></w:lvlOverride></w:num>`).join("");
  return `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:numbering xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">${abstractXml}${instanceXml}</w:numbering>`;
}

function corePropertiesXml(title, sourcePdf, createdAt) {
  const timestamp = createdAt.toISOString();
  return `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:dcterms="http://purl.org/dc/terms/" xmlns:dcmitype="http://purl.org/dc/dcmitype/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance"><dc:title>${xmlEscape(title)}</dc:title><dc:subject>Derived IELTS Word parser regression fixture</dc:subject><dc:creator>PDF2TEST corpus generator</dc:creator><dc:description>Derived from ${xmlEscape(sourcePdf)}</dc:description><cp:lastModifiedBy>PDF2TEST corpus generator</cp:lastModifiedBy><dcterms:created xsi:type="dcterms:W3CDTF">${timestamp}</dcterms:created><dcterms:modified xsi:type="dcterms:W3CDTF">${timestamp}</dcterms:modified></cp:coreProperties>`;
}

function appPropertiesXml() {
  return `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Properties xmlns="http://schemas.openxmlformats.org/officeDocument/2006/extended-properties" xmlns:vt="http://schemas.openxmlformats.org/officeDocument/2006/docPropsVTypes"><Application>PDF2TEST corpus generator</Application><AppVersion>1.0</AppVersion></Properties>`;
}

function createZip(files, createdAt) {
  const localParts = [];
  const centralParts = [];
  let offset = 0;
  const { dosDate, dosTime } = toDosDateTime(createdAt);
  for (const file of files) {
    const name = Buffer.from(file.name.replaceAll("\\", "/"), "utf8");
    const data = Buffer.isBuffer(file.data) ? file.data : Buffer.from(file.data);
    const crc = crc32(data);
    const localHeader = Buffer.alloc(30);
    localHeader.writeUInt32LE(0x04034b50, 0);
    localHeader.writeUInt16LE(20, 4);
    localHeader.writeUInt16LE(0x0800, 6);
    localHeader.writeUInt16LE(0, 8);
    localHeader.writeUInt16LE(dosTime, 10);
    localHeader.writeUInt16LE(dosDate, 12);
    localHeader.writeUInt32LE(crc, 14);
    localHeader.writeUInt32LE(data.length, 18);
    localHeader.writeUInt32LE(data.length, 22);
    localHeader.writeUInt16LE(name.length, 26);
    localHeader.writeUInt16LE(0, 28);
    localParts.push(localHeader, name, data);

    const centralHeader = Buffer.alloc(46);
    centralHeader.writeUInt32LE(0x02014b50, 0);
    centralHeader.writeUInt16LE(20, 4);
    centralHeader.writeUInt16LE(20, 6);
    centralHeader.writeUInt16LE(0x0800, 8);
    centralHeader.writeUInt16LE(0, 10);
    centralHeader.writeUInt16LE(dosTime, 12);
    centralHeader.writeUInt16LE(dosDate, 14);
    centralHeader.writeUInt32LE(crc, 16);
    centralHeader.writeUInt32LE(data.length, 20);
    centralHeader.writeUInt32LE(data.length, 24);
    centralHeader.writeUInt16LE(name.length, 28);
    centralHeader.writeUInt16LE(0, 30);
    centralHeader.writeUInt16LE(0, 32);
    centralHeader.writeUInt16LE(0, 34);
    centralHeader.writeUInt16LE(0, 36);
    centralHeader.writeUInt32LE(0, 38);
    centralHeader.writeUInt32LE(offset, 42);
    centralParts.push(centralHeader, name);
    offset += localHeader.length + name.length + data.length;
  }

  const centralDirectory = Buffer.concat(centralParts);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(0, 4);
  end.writeUInt16LE(0, 6);
  end.writeUInt16LE(files.length, 8);
  end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(centralDirectory.length, 12);
  end.writeUInt32LE(offset, 16);
  end.writeUInt16LE(0, 20);
  return Buffer.concat([...localParts, centralDirectory, end]);
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) crc = crcTable[(crc ^ byte) & 0xff] ^ (crc >>> 8);
  return (crc ^ 0xffffffff) >>> 0;
}

function toDosDateTime(date) {
  const year = Math.max(1980, date.getFullYear());
  return {
    dosDate: ((year - 1980) << 9) | ((date.getMonth() + 1) << 5) | date.getDate(),
    dosTime: (date.getHours() << 11) | (date.getMinutes() << 5) | Math.floor(date.getSeconds() / 2)
  };
}

function collectMarkerStats(documentIr) {
  const counters = { questionRanges: {}, questionNumbers: {}, alphaMarkers: {}, romanMarkers: {} };
  for (const page of documentIr.pages ?? []) {
    for (const block of page.blocks ?? []) {
      for (const line of cleanText(stripWordFormatCharacters(block?.text ?? "")).split(/\n+/)) {
        for (const match of line.matchAll(/Questions?\s+(\d{1,3})\s*[-–—]\s*(\d{1,3})/gi)) {
          increment(counters.questionRanges, `${Number(match[1])}-${Number(match[2])}`);
        }
        const question = line.match(/^\s*(\d{1,3})(?:[.)]|\s+)/);
        if (question) increment(counters.questionNumbers, String(Number(question[1])));
        const alpha = line.match(/^\s*([A-H])(?:[.)]|\s+)/);
        if (alpha) increment(counters.alphaMarkers, alpha[1]);
        const roman = line.match(/^\s*(i|ii|iii|iv|v|vi|vii|viii|ix|x|xi|xii)(?:[.)]|\s+)/i);
        if (roman) increment(counters.romanMarkers, roman[1].toLowerCase());
      }
    }
  }
  return counters;
}

function compareMarkerStats(expected, actual) {
  let expectedCount = 0;
  let recoveredCount = 0;
  const families = {};
  for (const family of Object.keys(expected)) {
    let familyExpected = 0;
    let familyRecovered = 0;
    for (const [marker, count] of Object.entries(expected[family])) {
      familyExpected += count;
      familyRecovered += Math.min(count, actual?.[family]?.[marker] ?? 0);
    }
    expectedCount += familyExpected;
    recoveredCount += familyRecovered;
    families[family] = {
      expected: familyExpected,
      recovered: familyRecovered,
      ratio: familyExpected === 0 ? 1 : familyRecovered / familyExpected
    };
  }
  return {
    expected: expectedCount,
    recovered: recoveredCount,
    ratio: expectedCount === 0 ? 1 : recoveredCount / expectedCount,
    families
  };
}

function collectAuthoringSignature(payload) {
  const groups = Array.isArray(payload?.authoringIr?.groups) ? payload.authoringIr.groups : [];
  return {
    groups: groups.map((group) => ({
      range: normalizeQuestionRange(group.questionRange),
      kind: String(group.kind ?? "unknown"),
      instruction: normalizeComparableText(Array.isArray(group.instruction) ? group.instruction.join(" ") : group.instruction),
      allowOptionReuse: group.allowOptionReuse === true,
      questions: (Array.isArray(group.questions) ? group.questions : []).map((question) => ({
        id: normalizeQuestionId(question.id ?? question.displayNumber),
        displayNumber: String(question.displayNumber ?? ""),
        prompt: normalizeComparableText(question.prompt),
        interactionType: String(question?.interaction?.type ?? ""),
        options: (Array.isArray(question?.interaction?.options) ? question.interaction.options : [])
          .map((option) => normalizeComparableText(typeof option === "object" ? option.label ?? option.value : option))
          .filter(Boolean),
        optionTexts: normalizeOptionTexts(question?.interaction?.optionTexts)
      }))
    }))
  };
}

function summarizeAuthoringSignature(signature) {
  return {
    groupCount: signature.groups.length,
    ranges: signature.groups.map((group) => group.range),
    kinds: signature.groups.map((group) => group.kind),
    questionIds: signature.groups.flatMap((group) => group.questions.map((question) => question.id)).filter(Boolean)
  };
}

function compareAuthoringSignatures(expected, actual) {
  const usedActualGroups = new Set();
  const counts = {
    ranges: { expected: expected.groups.length, recovered: 0 },
    kinds: { expected: expected.groups.length, recovered: 0 },
    reusePolicy: { expected: expected.groups.length, recovered: 0 },
    instructions: { expected: 0, recovered: 0 },
    questionIds: { expected: 0, recovered: 0 },
    prompts: { expected: 0, recovered: 0 },
    interactionTypes: { expected: 0, recovered: 0 },
    options: { expected: 0, recovered: 0 },
    optionTexts: { expected: 0, recovered: 0 }
  };

  for (const expectedGroup of expected.groups) {
    const expectedRangeKey = rangeKey(expectedGroup.range);
    const actualIndex = actual.groups.findIndex((candidate, index) => (
      !usedActualGroups.has(index) && rangeKey(candidate.range) === expectedRangeKey
    ));
    if (actualIndex < 0) {
      counts.questionIds.expected += expectedGroup.questions.length;
      counts.prompts.expected += expectedGroup.questions.filter((question) => question.prompt).length;
      counts.interactionTypes.expected += expectedGroup.questions.filter((question) => question.interactionType).length;
      counts.options.expected += expectedGroup.questions.filter((question) => question.options.length > 0).length;
      counts.optionTexts.expected += expectedGroup.questions.filter((question) => Object.keys(question.optionTexts).length > 0).length;
      if (expectedGroup.instruction) counts.instructions.expected += 1;
      continue;
    }
    usedActualGroups.add(actualIndex);
    const actualGroup = actual.groups[actualIndex];
    counts.ranges.recovered += 1;
    if (actualGroup.kind === expectedGroup.kind) counts.kinds.recovered += 1;
    if (actualGroup.allowOptionReuse === expectedGroup.allowOptionReuse) counts.reusePolicy.recovered += 1;
    if (expectedGroup.instruction) {
      counts.instructions.expected += 1;
      if (actualGroup.instruction === expectedGroup.instruction) counts.instructions.recovered += 1;
    }

    const actualQuestions = new Map(actualGroup.questions.map((question) => [question.id, question]));
    for (const expectedQuestion of expectedGroup.questions) {
      counts.questionIds.expected += 1;
      const actualQuestion = actualQuestions.get(expectedQuestion.id);
      if (actualQuestion) counts.questionIds.recovered += 1;
      if (expectedQuestion.prompt) {
        counts.prompts.expected += 1;
        if (actualQuestion?.prompt === expectedQuestion.prompt) counts.prompts.recovered += 1;
      }
      if (expectedQuestion.interactionType) {
        counts.interactionTypes.expected += 1;
        if (actualQuestion?.interactionType === expectedQuestion.interactionType) counts.interactionTypes.recovered += 1;
      }
      if (expectedQuestion.options.length > 0) {
        counts.options.expected += 1;
        if (arraysEqual(actualQuestion?.options ?? [], expectedQuestion.options)) counts.options.recovered += 1;
      }
      if (Object.keys(expectedQuestion.optionTexts).length > 0) {
        counts.optionTexts.expected += 1;
        if (objectsEqual(actualQuestion?.optionTexts ?? {}, expectedQuestion.optionTexts)) counts.optionTexts.recovered += 1;
      }
    }
  }

  const ratios = Object.fromEntries(Object.entries(counts).map(([name, value]) => [
    name,
    { ...value, ratio: value.expected === 0 ? 1 : value.recovered / value.expected }
  ]));
  const zeroGroupMatch = expected.groups.length !== 0 || actual.groups.length === 0;
  const ok = zeroGroupMatch
    && actual.groups.length === expected.groups.length
    && ratios.ranges.ratio === 1
    && ratios.kinds.ratio === 1
    && ratios.reusePolicy.ratio === 1
    && ratios.instructions.ratio >= 0.9
    && ratios.questionIds.ratio >= 0.98
    && ratios.prompts.ratio >= 0.9
    && ratios.interactionTypes.ratio >= 0.98
    && ratios.options.ratio >= 0.9
    && ratios.optionTexts.ratio >= 0.9;
  return {
    ok,
    expectedGroupCount: expected.groups.length,
    actualGroupCount: actual.groups.length,
    zeroGroupMatch,
    metrics: ratios
  };
}

function collectAssetSummary(documentIr) {
  const embeddedAssets = Array.isArray(documentIr.assets) ? documentIr.assets.length : 0;
  const imageBlocks = (documentIr.pages ?? []).reduce((count, page) => count + (page.blocks ?? [])
    .filter((block) => ["image", "figure", "diagram"].includes(String(block?.blockType ?? "").toLowerCase())).length, 0);
  return {
    embeddedAssetCount: embeddedAssets,
    imageBlockCount: imageBlocks,
    total: embeddedAssets + imageBlocks
  };
}

function collectBlockSummary(documentIr) {
  const summary = { total: 0, paragraphs: 0, headers: 0, tables: 0, images: 0, other: 0 };
  for (const page of documentIr.pages ?? []) {
    for (const block of page.blocks ?? []) {
      summary.total += 1;
      const type = String(block?.blockType ?? "").toLowerCase();
      if (type === "paragraph") summary.paragraphs += 1;
      else if (type === "header") summary.headers += 1;
      else if (type === "table") summary.tables += 1;
      else if (["image", "figure", "diagram"].includes(type)) summary.images += 1;
      else summary.other += 1;
    }
  }
  return summary;
}

function collectRoundTripNumbering(documentIr) {
  const formats = {};
  const renderedLeadingNumbers = [];
  for (const page of documentIr.pages ?? []) {
    for (const block of page.blocks ?? []) {
      const format = block?.layoutHints?.numbering?.format;
      if (format) formats[format] = (formats[format] ?? 0) + 1;
      const number = String(block?.text ?? "").match(/^\s*(\d{1,3})(?:[.)]|\s+)/)?.[1];
      if (number) renderedLeadingNumbers.push(Number(number));
    }
  }
  return {
    formats,
    renderedLeadingNumbers: Array.from(new Set(renderedLeadingNumbers)).sort((left, right) => left - right)
  };
}

function deriveQuestionGroupExpectation(relativeSource, sourceAuthoring, extraZeroFilter) {
  const zeroGroupReason = zeroQuestionGroupReason(relativeSource, extraZeroFilter);
  if (zeroGroupReason) {
    return {
      type: "negative-zero-groups",
      expectedQuestionGroupCount: 0,
      reason: zeroGroupReason
    };
  }
  return {
    type: "source-derived",
    expectedQuestionGroupCount: sourceAuthoring.groups.length,
    reason: "derived from PDF AuthoringIR"
  };
}

function zeroQuestionGroupReason(relativeSource, extraZeroFilter) {
  const normalized = relativeSource.normalize("NFKC").toLowerCase();
  const builtInNegative = /仅原文无题|仅文章无题|无题版|passage[ -]?only|no questions?/.test(normalized);
  const configuredNegative = Boolean(extraZeroFilter && normalized.includes(extraZeroFilter));
  if (configuredNegative) return `matched --expect-zero ${extraZeroFilter}`;
  if (builtInNegative) return "source filename declares passage-only/no-question content";
  return null;
}

function normalizeQuestionRange(value) {
  if (Array.isArray(value) && value.length >= 2) return [Number(value[0]), Number(value[1])];
  return [];
}

function rangeKey(range) {
  return Array.isArray(range) && range.length >= 2 ? `${range[0]}-${range[1]}` : "unknown";
}

function normalizeQuestionId(value) {
  const text = String(value ?? "").trim();
  const match = text.match(/(?:q)?(\d{1,4})/i);
  return match ? `q${Number(match[1])}` : text.toLowerCase();
}

function normalizeComparableText(value) {
  return cleanText(stripWordFormatCharacters(value ?? ""))
    .replace(/[‐‑‒–—]/g, "-")
    .replace(/\s+/g, " ")
    .trim()
    .toLowerCase();
}

function arraysEqual(left, right) {
  return left.length === right.length && left.every((value, index) => value === right[index]);
}

function objectsEqual(left, right) {
  const leftKeys = Object.keys(left ?? {}).sort();
  const rightKeys = Object.keys(right ?? {}).sort();
  return leftKeys.length === rightKeys.length
    && leftKeys.every((key, index) => key === rightKeys[index] && left[key] === right[key]);
}

function normalizeOptionTexts(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  return Object.fromEntries(Object.entries(value)
    .map(([key, text]) => [
      normalizeComparableText(key),
      normalizeComparableText(text)
    ])
    .filter(([key, text]) => key && text));
}

function increment(counter, key) {
  counter[key] = (counter[key] ?? 0) + 1;
}

function summarizeManifest(entries) {
  return {
    generatedCount: entries.filter((entry) => entry.status === "generated").length,
    skippedCount: entries.filter((entry) => entry.status === "skipped_existing").length,
    failedCount: entries.filter((entry) => entry.status === "failed").length,
    verifiedCount: entries.filter((entry) => entry.verification).length,
    verificationPassedCount: entries.filter((entry) => entry.verification?.ok === true).length,
    verificationFailedCount: entries.filter((entry) => entry.verification?.ok === false).length,
    sourceExpectationFailedCount: entries.filter((entry) => entry.source?.expectationMatch === false).length,
    roundTripExpectationFailedCount: entries.filter((entry) => entry.verification?.expectationMatch === false).length
  };
}

function outputName(relativeSource, numberingModeValue, optionLayoutValue, declaredZeroGroupSource = false) {
  const withoutExtension = relativeSource.slice(0, -path.extname(relativeSource).length);
  const slug = withoutExtension
    .normalize("NFKD")
    .replace(/[^a-z0-9]+/gi, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 80) || "document";
  const suffix = crypto.createHash("sha256").update(relativeSource).digest("hex").slice(0, 10);
  const corpusRole = declaredZeroGroupSource ? "-passage-only" : "";
  const variant = numberingModeValue === "word" ? "-word-numbered" : "";
  const layout = optionLayoutValue === "paragraph" ? "-paragraph" : "";
  return `${slug}-${suffix}${corpusRole}${variant}${layout}.docx`;
}

function variantManifestName(numberingModeValue, optionLayoutValue) {
  const numbering = numberingModeValue === "word" ? "word-numbered" : "explicit";
  return `manifest-${numbering}-${optionLayoutValue}.json`;
}

function canonicalRelativePath(value) {
  return String(value ?? "")
    .replaceAll("\\", "/")
    .normalize("NFKC");
}

function compareCanonicalPaths(left, right) {
  const a = canonicalRelativePath(left);
  const b = canonicalRelativePath(right);
  return a < b ? -1 : a > b ? 1 : 0;
}

function countDocumentBlocks(documentIr) {
  return (documentIr.pages ?? []).reduce((sum, page) => sum + (page.blocks?.length ?? 0), 0);
}

function blockOrdinal(block) {
  const ordinal = Number(block?._epic8Ordinal);
  return Number.isFinite(ordinal) ? ordinal : Number.MAX_SAFE_INTEGER;
}

function cleanText(value) {
  return stripInvalidXmlCharacters(String(value ?? ""))
    .replace(/\r\n?/g, "\n")
    .replace(/[ \t]+$/gm, "")
    .trim();
}

function stripWordFormatCharacters(value) {
  // Native Word numbering may leave format-only characters between a
  // rendered label and its content. Preserve them while building DOCX so an
  // otherwise-empty numbered paragraph remains visible, but ignore them in
  // semantic and marker comparisons.
  return String(value ?? "").replace(/[\u200B\u200C\u200D\u2060\uFEFF]/g, "");
}

function stripHtml(value) {
  return decodeHtmlEntities(String(value ?? "")
    .replace(/<br\s*\/?>/gi, "\n")
    .replace(/<\/p\s*>/gi, "\n")
    .replace(/<\/tr\s*>/gi, "\n")
    .replace(/<\/t[dh]\s*>/gi, "\t")
    .replace(/<[^>]+>/g, ""));
}

function decodeHtmlEntities(value) {
  return value
    .replace(/&nbsp;/gi, " ")
    .replace(/&amp;/gi, "&")
    .replace(/&lt;/gi, "<")
    .replace(/&gt;/gi, ">")
    .replace(/&quot;/gi, "\"")
    .replace(/&#39;|&apos;/gi, "'")
    .replace(/&#(\d+);/g, (_, code) => String.fromCodePoint(Number(code)))
    .replace(/&#x([0-9a-f]+);/gi, (_, code) => String.fromCodePoint(Number.parseInt(code, 16)));
}

function stripInvalidXmlCharacters(value) {
  return value.replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\uFFFE\uFFFF]/g, "");
}

function xmlEscape(value) {
  return stripInvalidXmlCharacters(String(value ?? ""))
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll("\"", "&quot;")
    .replaceAll("'", "&apos;");
}

function sha256File(filePath) {
  return sha256Buffer(fs.readFileSync(filePath));
}

function sha256Buffer(buffer) {
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

function listFilesRecursively(directory) {
  const files = [];
  const pending = [directory];
  while (pending.length > 0) {
    const current = pending.pop();
    for (const entry of fs.readdirSync(current, { withFileTypes: true })) {
      const fullPath = path.join(current, entry.name);
      if (entry.isDirectory()) pending.push(fullPath);
      else if (entry.isFile()) files.push(fullPath);
    }
  }
  return files;
}

function seededRandom(seedValue) {
  let hash = 2166136261;
  for (const char of seedValue) {
    hash ^= char.charCodeAt(0);
    hash = Math.imul(hash, 16777619);
  }
  return () => {
    hash += 0x6d2b79f5;
    let value = hash;
    value = Math.imul(value ^ (value >>> 15), value | 1);
    value ^= value + Math.imul(value ^ (value >>> 7), value | 61);
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
}

function shuffle(items, seedValue) {
  const random = seededRandom(seedValue);
  const copy = items.slice();
  for (let index = copy.length - 1; index > 0; index -= 1) {
    const swap = Math.floor(random() * (index + 1));
    [copy[index], copy[swap]] = [copy[swap], copy[index]];
  }
  return copy;
}

function parseArgs(argv) {
  const parsed = {};
  for (let index = 0; index < argv.length; index += 1) {
    const argument = argv[index];
    if (!argument.startsWith("--")) continue;
    const equalIndex = argument.indexOf("=");
    if (equalIndex > 2) {
      parsed[argument.slice(2, equalIndex)] = argument.slice(equalIndex + 1);
      continue;
    }
    const key = argument.slice(2);
    const next = argv[index + 1];
    if (!next || next.startsWith("--")) parsed[key] = true;
    else {
      parsed[key] = next;
      index += 1;
    }
  }
  return parsed;
}

function parseBoolean(value, fallback) {
  if (value == null) return fallback;
  if (value === true || value === "true" || value === "1") return true;
  if (value === false || value === "false" || value === "0") return false;
  fail(`invalid boolean value: ${value}`);
}

function parseOptionalPositiveInteger(value, label) {
  if (value == null) return null;
  const number = Number(value);
  if (!Number.isInteger(number) || number <= 0) fail(`${label} must be a positive integer`);
  return number;
}

function assertReadableDirectory(directory, label) {
  if (!fs.existsSync(directory) || !fs.statSync(directory).isDirectory()) {
    fail(`${label} is not a readable directory: ${directory}`);
  }
}

function cliBinaryName() {
  return process.platform === "win32" ? "ielts-author-studio.exe" : "ielts-author-studio";
}

function fail(message) {
  console.error(`[docx-corpus] ${message}`);
  process.exit(2);
}

function printUsageAndExit(code) {
  const usage = [
    "usage: node scripts/generate-docx-corpus.mjs [options]",
    "",
    "Converts a PDF corpus into structurally equivalent DOCX parser fixtures.",
    "The default source is PDF2TEST_PDF_CORPUS, then E:\\tmp\\PDF when present,",
    "and finally fixtures/parser. Output defaults to the operating-system temp directory.",
    "",
    "Options:",
    "  --pdf-dir <dir>              PDF corpus directory (searched recursively).",
    "  --out-dir <dir>              DOCX and manifest output directory.",
    "  --sample <n>                 Deterministically sample n PDFs; default is all.",
    "  --seed <value>               Sampling seed. Default: pdf2test-word-corpus.",
    "  --filter <text>              Keep source paths containing this text.",
    "  --option-layout <mode>       table (default) or paragraph.",
    "  --numbering-mode <mode>      explicit (default) or native Word numbering.",
    "  --expect-zero <text>          Treat matching source names as zero-group negatives.",
    "  --verify                     Parse each generated DOCX through the same CLI.",
    "  --overwrite                  Replace existing DOCX files.",
    "  --strict                     Exit non-zero on generation/verification failure.",
    "  --cli <path>                 Existing ielts-author-studio CLI binary.",
    "  --skip-build                 Reuse the existing CLI without cargo build.",
    "  --self-test                  Run deterministic numbering-model checks and exit.",
    "  --help                       Show this help."
  ].join("\n");
  (code === 0 ? console.log : console.error)(usage);
  process.exit(code);
}
