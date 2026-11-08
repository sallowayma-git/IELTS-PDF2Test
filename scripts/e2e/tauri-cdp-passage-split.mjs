// Real Tauri import workflow. Synthetic PDF text runs model fragmented headers;
// the missing-native run deliberately makes PDFium fail to bind.
import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import {
  assertBuildFresh, launchTauriAppCdp, repoRoot, sha256File,
} from "./lib/tauri-cdp-harness.mjs";

const root = path.join(repoRoot, "artifacts/e2e-cdp", `run-passage-split-${new Date().toISOString().replace(/[:.]/g, "-")}`);
const exePath = path.join(repoRoot, "src-tauri/target/debug/ielts-author-studio.exe");
fs.mkdirSync(root, { recursive: true });

function pdf(pages) {
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    `<< /Type /Pages /Kids [${pages.map((_, i) => `${4 + 2 * i} 0 R`).join(" ")}] /Count ${pages.length} >>`,
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
  ];
  pages.forEach((lines, i) => {
    objects.push(`<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R >> >> /Contents ${5 + 2 * i} 0 R >>`);
    const stream = "BT /F1 10 Tf 20 TL 40 800 Td\n" + lines.map(line => `(${line.replace(/[\\()]/g, "\\$&")}) Tj T*\n`).join("") + "ET";
    objects.push(`<< /Length ${Buffer.byteLength(stream)} >>\nstream\n${stream}\nendstream`);
  });
  let out = "%PDF-1.4\n";
  const offsets = [0];
  objects.forEach((object, i) => { offsets.push(Buffer.byteLength(out)); out += `${i + 1} 0 obj\n${object}\nendobj\n`; });
  const xref = Buffer.byteLength(out);
  out += `xref\n0 ${offsets.length}\n0000000000 65535 f \n` + offsets.slice(1).map(offset => `${String(offset).padStart(10, "0")} 00000 n \n`).join("");
  return out + `trailer\n<< /Size ${offsets.length} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF`;
}

const fixture = path.join(root, "fragmented-reading.pdf");
fs.writeFileSync(fixture, pdf([1, 2].map(n => [
  n === 1 ? "2" : "6", "READING", "PASSAGE", String(n),
  `Youshouldspendabout20minutesonQuestions${n === 1 ? "1-13" : "14-26"},`,
  `whicharebasedonReadingPassage${n}onpages${n === 1 ? "2and3" : "6and7"}.`,
  `Article ${n} explains a different topic.`,
  `Questions ${n === 1 ? "1-13" : "14-26"}`,
  "Do the following statements agree with the information in the passage?",
])));
const blankFixture = path.join(root, "unreadable-pages.pdf");
fs.writeFileSync(blankFixture, pdf([[], []]));
const watermarkFixture = path.join(root, "watermark-only-pages.pdf");
fs.writeFileSync(watermarkFixture, pdf([["2", "Publisher watermark"], ["6", "Publisher watermark"]]));

const report = { channel: "webview2-cdp", diagnosticPort: true, securityRelaxations: [], exeSha256: sha256File(exePath), build: assertBuildFresh({ exePath }), scenarios: [] };

async function run(name, { noNative = false, blocked = false, unreadable = false, watermark = false } = {}) {
  const runDir = path.join(root, name);
  fs.mkdirSync(runDir, { recursive: true });
  const badDll = path.join(runDir, "unloadable-pdfium.dll");
  if (noNative) fs.writeFileSync(badDll, "deliberately invalid native library");
  const chosen = watermark ? watermarkFixture : unreadable ? blankFixture : fixture;
  const session = await launchTauriAppCdp({ exePath, runDir, appEnv: {
    PDF2TEST_AUTOMATION_SOURCE_FILES: chosen,
    ...(noNative ? { EPIC8_PDFIUM_LIB: badDll } : {}),
  } });
  const facts = { name, noNative, blocked, unreadable };
  try {
    await session.waitFor(`!!document.querySelector('[data-testid="library-page"]')`, { timeoutMs: 40000 });
    await session.clickSelector('[data-testid="library-import"]');
    await session.waitFor(`!!document.querySelector('[data-testid="import-cloud-offline"]')`, { timeoutMs: 15000 });
    await session.clickSelectorWhenStable('[data-testid="import-pick-files"]');
    await session.waitFor(`document.querySelectorAll('[data-testid="import-picked-files"] li').length === 1`);
    await session.clickSelector('[data-testid="import-start"]');
    if (unreadable) {
      await session.waitFor(`!!document.querySelector('[data-testid="import-split-warning"]')`);
      facts.warning = await session.evaluate(`document.querySelector('[data-testid="import-split-warning"]').innerText`);
      assert.match(facts.warning, /没有可读文字/);
      assert.equal(await session.evaluate(`document.querySelectorAll('[data-testid="library-row"]').length`), 0);
      facts.screenshot = await session.screenshot("unreadable-warning");
    } else {
      await session.waitFor(`document.querySelectorAll('[data-testid="import-split-segment"]').length === 2`);
      facts.preview = await session.invoke("plan_import_split", { paths: [chosen] });
      assert.equal(facts.preview.ok, true);
      assert.equal(facts.preview.value[0].segments.length, 2);
      assert.deepEqual(facts.preview.value[0].segments.map(s => s.pages), [[1], [2]]);
      assert.equal(await session.evaluate(`document.querySelector('[data-testid="import-split-toggle"]').checked`), true);
      facts.previewScreenshot = await session.screenshot("split-preview");
      if (blocked) {
        fs.mkdirSync(path.join(runDir, "appdata/data/tmp"), { recursive: true });
        fs.writeFileSync(path.join(runDir, "appdata/data/tmp/import-split"), "blocked temp directory");
      }
      await session.clickSelector('[data-testid="import-start"]');
      if (blocked) {
        await session.waitFor(`!!document.querySelector('[data-testid="import-rejected"]')`);
        facts.rejection = await session.evaluate(`document.querySelector('[data-testid="import-rejected"]').innerText`);
        assert.match(facts.rejection, /拆分未完成/);
        assert.equal(await session.evaluate(`document.querySelectorAll('[data-testid="library-row"]').length`), 0);
      } else {
        await session.waitFor(`document.querySelectorAll('[data-testid="library-row"]').length === 2`, { timeoutMs: 30000 });
        facts.rows = await session.evaluate(`[...document.querySelectorAll('[data-testid="library-row"]')].map(row => row.innerText)`);
        const jobsDir = path.join(runDir, "appdata/data/jobs");
        facts.jobs = fs.readdirSync(jobsDir).map(id => JSON.parse(fs.readFileSync(path.join(jobsDir, id, "job.json"), "utf8")));
        assert.equal(facts.jobs.length, 2);
        assert(facts.jobs.every(job => job.sourceFiles.filter(source => source.role === "MainQuestion").length === 1));
      }
      facts.resultScreenshot = await session.screenshot("import-result");
    }
    facts.verdict = "passed";
  } catch (error) {
    facts.verdict = "failed"; facts.error = String(error);
    facts.screenshot = await session.screenshot("failure").catch(() => null);
    throw error;
  } finally {
    facts.output = (await session.close()).appOutput;
    report.scenarios.push(facts);
    fs.writeFileSync(path.join(root, "report.json"), JSON.stringify(report, null, 2));
  }
}

try {
  await run("normal");
  await run("without-native-library", { noNative: true });
  await run("failed-staging", { blocked: true });
  await run("unreadable-pages", { unreadable: true });
  await run("watermark-only-pages", { unreadable: true, watermark: true });
  report.verdict = "passed";
} catch (error) {
  report.verdict = "failed"; process.exitCode = 1; console.error(error);
} finally {
  fs.writeFileSync(path.join(root, "report.json"), JSON.stringify(report, null, 2));
  console.log(path.join(root, "report.json"));
}
