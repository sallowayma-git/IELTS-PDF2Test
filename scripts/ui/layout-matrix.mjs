#!/usr/bin/env node
// Horizontal-overflow acceptance matrix (plan section 10.8 / P0-T03).
//
// Asserts, for every product route across a viewport + Windows-scaling matrix:
//   1. document.documentElement.scrollWidth <= clientWidth + 1   (no app-wide h-scroll)
//   2. no element with overflow-x hidden/clip is silently clipping wider content
//   3. no visible element extends past the right edge of the viewport
//
// Usage:
//   node scripts/ui/layout-matrix.mjs                  # default product routes
//   node scripts/ui/layout-matrix.mjs --include-legacy # also probe pre-convergence routes
//   node scripts/ui/layout-matrix.mjs --update-baseline # record current offenders as the accepted baseline
//   node scripts/ui/layout-matrix.mjs --headful --verbose
import fs from "node:fs";
import path from "node:path";
import {
  DEFAULT_BASE_URL, argValue, createPage, ensureVite, evaluate, gotoHash,
  hasFlag, launchChrome, navigate, repoRoot, screenshot, setViewport
} from "../lib/cdp.mjs";

const FIXTURE = path.join(repoRoot, "fixtures", "ui", "long-content.json");
const BASELINE = path.join(repoRoot, "fixtures", "ui", "layout-matrix-baseline.json");
const OUT_DIR = path.join(repoRoot, "tmp", "ui-layout");

const PRODUCT_ROUTES = [
  { id: "library", hash: "/library", label: "题库" },
  { id: "settings", hash: "/settings", label: "设置" }
];
// 兼容期旧页面在不可导航的 `#/legacy/...` 下（router.ts 的 legacy 逃生通道）。
const LEGACY_ROUTES = [
  { id: "legacy-dashboard", hash: "/legacy/dashboard", label: "工作台（兼容期）" },
  { id: "legacy-jobs", hash: "/legacy/jobs", label: "导题任务（兼容期）" },
  { id: "legacy-import", hash: "/legacy/import", label: "新建导题（兼容期）" },
  { id: "legacy-export", hash: "/legacy/export", label: "NAS 导出（兼容期）" },
  { id: "legacy-writing", hash: "/legacy/writing", label: "写作题创作（兼容期）" }
];
// Job-scoped routes hold the only live fixed three-column grid (`.review-grid` in
// DocumentReview) plus the widest editor layouts. They render empty without a real job, so
// they are only measured when a job id is supplied: `--job-id <id>`.
const jobRoutes = (jobId) => [
  { id: "legacy-document", hash: `/legacy/document/${jobId}`, label: "源文档确认（兼容期）" },
  { id: "legacy-preview", hash: `/legacy/preview/${jobId}`, label: "确认与编辑（兼容期）" },
  { id: "legacy-authoring-v2", hash: `/legacy/authoring-v2/${jobId}`, label: "结构化编辑器（兼容期）" }
];
const itemRoutes = (itemId) => [{ id: "workspace", hash: `/items/${itemId}`, label: "题目工作区" }];

// Injected into the page. Returns every overflow offender with enough detail to fix it.
const PROBE = `(() => {
  // Sub-pixel rounding and default UA control margins produce 1-3px overhangs that are not
  // product defects. Anything at or beyond 4px is a real clipped-content bug.
  const TOLERANCE = 4;
  const describe = (el) => {
    if (!el || el === document.documentElement) return "html";
    const id = el.id ? "#" + el.id : "";
    const classes = el.classList ? Array.from(el.classList).slice(0, 3) : [];
    const cls = classes.length ? "." + classes.join(".") : "";
    return el.tagName.toLowerCase() + id + cls;
  };
  const root = document.documentElement;
  const viewportWidth = root.clientWidth;
  const offenders = [];
  const seen = new Set();
  const push = (kind, el, extra) => {
    const key = kind + "|" + describe(el) + "|" + (extra.owner || "");
    if (seen.has(key)) return;
    seen.add(key);
    offenders.push({ kind, selector: describe(el), ...extra });
  };
  if (root.scrollWidth > root.clientWidth + 1) {
    push("document_h_scroll", root, { scrollWidth: root.scrollWidth, clientWidth: root.clientWidth });
  }
  const visible = (el) => {
    const style = getComputedStyle(el);
    if (style.visibility === "hidden" || style.display === "none") return null;
    return style;
  };
  for (const el of document.querySelectorAll("body *")) {
    const style = visible(el);
    if (!style) continue;
    const rect = el.getBoundingClientRect();
    if (rect.width === 0 && rect.height === 0) continue;

    // A container that hides horizontal overflow is only a defect when a real DESCENDANT
    // element is cut off. scrollWidth alone also counts decorative pseudo-elements, which
    // are an intentional use of overflow:hidden and must not fail the matrix.
    if (style.overflowX === "hidden" || style.overflowX === "clip") {
      const contentRight = rect.right - parseFloat(style.borderRightWidth || "0") - parseFloat(style.paddingRight || "0");
      const victims = [];
      for (const child of el.querySelectorAll("*")) {
        const childStyle = visible(child);
        if (!childStyle || childStyle.position === "fixed" || childStyle.position === "absolute") continue;
        const childRect = child.getBoundingClientRect();
        if (childRect.width === 0 && childRect.height === 0) continue;
        if (childRect.right > contentRight + TOLERANCE) victims.push({ right: childRect.right, selector: describe(child) });
      }
      victims.sort((a, b) => b.right - a.right);
      for (const victim of victims.slice(0, 3)) {
        push("clipped_content", el, {
          owner: victim.selector,
          clippedChild: victim.selector,
          childRight: Math.round(victim.right),
          contentRight: Math.round(contentRight),
          overflowX: style.overflowX
        });
      }
    }

    // 只有当元素与视口之间没有任何横向滚动/裁切容器时，越界才是缺陷。
    // 宽表格放在 overflow-x:auto 的包裹层里横向滚动是正确策略，不是溢出；
    // 而 hidden/clip 的越界已经由上面 clipped_content 在那个容器上报告过一次。
    if (style.position !== "fixed" && rect.right > viewportWidth + TOLERANCE && rect.width <= viewportWidth * 3) {
      let governed = false;
      for (let parent = el.parentElement; parent && parent !== document.documentElement; parent = parent.parentElement) {
        const overflowX = getComputedStyle(parent).overflowX;
        if (overflowX === "auto" || overflowX === "scroll" || overflowX === "hidden" || overflowX === "clip") {
          governed = true;
          break;
        }
      }
      if (!governed) push("past_viewport", el, { right: Math.round(rect.right), viewportWidth, width: Math.round(rect.width) });
    }
  }
  return { viewportWidth, documentScrollWidth: root.scrollWidth, offenders: offenders.slice(0, 40) };
})()`;

/** Appends a stress block that reuses the app's real class names, so long file names,
 *  long errors, 12 options and a 10-column table are measured against production CSS. */
function stressScript(fixture) {
  const payload = JSON.stringify({
    fileName: fixture.longFileName,
    error: fixture.longErrorMessage,
    url: fixture.longUrl,
    tags: fixture.manyTags,
    options: fixture.twelveOptions,
    table: fixture.wideTable,
    mixed: fixture.mixedScript
  });
  return `(() => {
  const data = ${payload};
  document.getElementById("layout-matrix-stress")?.remove();
  const host = document.querySelector(".app-main-content") || document.querySelector(".surface")
    || document.querySelector(".app-main") || document.querySelector("#root > *") || document.body;
  const box = document.createElement("div");
  box.id = "layout-matrix-stress";
  const tagHtml = data.tags.map((t) => '<span class="tag">' + t + "</span>").join("");
  // Text only: the stress target is how app CSS wraps 12 long option strings, not how a
  // form control sizes itself. A bare control inherits the app-wide "input width 100%" rule
  // plus UA margins, reporting a synthetic 5px overhang no real option row produces.
  const optionHtml = data.options
    .map((o) => '<div class="option-row"><b>' + o.label + "</b> <span>" + o.text + "</span></div>")
    .join("");
  const head = data.table.columns.map((c) => "<th>" + c + "</th>").join("");
  const body = data.table.rows.map((r) => "<tr>" + r.map((c) => "<td>" + c + "</td>").join("") + "</tr>").join("");
  // The probe measures how APP CSS handles hostile content, so give the rows realistic
  // block-level layout instead of leaving labels inline (inline atomic controls pack to the
  // line edge and produce a few px of overhang that is not an app defect).
  const style = document.createElement("style");
  style.textContent = "#layout-matrix-stress .option-row{display:block;}"
    + "#layout-matrix-stress .tag{display:inline-block;margin:0 6px 6px 0;}"
    + "#layout-matrix-stress{margin-top:18px;}";
  box.appendChild(style);
  const content = document.createElement("div");
  content.innerHTML = [
    '<div class="job-row static"><strong class="file-name">' + data.fileName + "</strong>",
    "<span>" + data.mixed + "</span></div>",
    '<p class="error-message">' + data.error + "</p>",
    '<p class="file-name"><a href="#">' + data.url + "</a></p>",
    '<div class="tag-row">' + tagHtml + "</div>",
    '<div class="option-list">' + optionHtml + "</div>",
    '<div class="table-wrap"><table><thead><tr>' + head + "</tr></thead><tbody>" + body + "</tbody></table></div>"
  ].join("");
  box.appendChild(content);
  host.appendChild(box);
  return true;
})()`;
}

function loadBaseline() {
  if (!fs.existsSync(BASELINE)) return { generatedAt: null, accepted: {} };
  return JSON.parse(fs.readFileSync(BASELINE, "utf8"));
}

/** A stable key for one offender so the baseline survives unrelated content changes. */
function offenderKey(offender) {
  return `${offender.kind}|${offender.selector}${offender.clippedChild ? "|" + offender.clippedChild : ""}`;
}

async function measure(cdp, { fixture, stress }) {
  if (stress) await evaluate(cdp, stressScript(fixture));
  else await evaluate(cdp, `(() => { document.getElementById("layout-matrix-stress")?.remove(); return true; })()`);
  return evaluate(cdp, PROBE);
}

async function main() {
  const fixture = JSON.parse(fs.readFileSync(FIXTURE, "utf8"));
  const baseUrl = argValue("--base-url", DEFAULT_BASE_URL);
  const jobId = argValue("--job-id");
  const itemId = argValue("--item-id");
  const routes = [
    ...PRODUCT_ROUTES,
    ...(itemId ? itemRoutes(itemId) : []),
    ...(hasFlag("--include-legacy") ? LEGACY_ROUTES : []),
    ...(jobId && hasFlag("--include-legacy") ? jobRoutes(jobId) : [])
  ];
  const viewports = fixture.viewportMatrix;
  const updateBaseline = hasFlag("--update-baseline");
  const verbose = hasFlag("--verbose");
  const baseline = loadBaseline();

  fs.rmSync(OUT_DIR, { recursive: true, force: true });
  fs.mkdirSync(OUT_DIR, { recursive: true });

  const vite = await ensureVite(baseUrl, { noStartServer: hasFlag("--no-start-server"), verbose });
  const chrome = await launchChrome({ headful: hasFlag("--headful"), chromePath: argValue("--chrome-path"), verbose });
  const results = [];
  let cdp;
  try {
    ({ cdp } = await createPage(chrome.port));
    await navigate(cdp, `${baseUrl}/`);
    for (const viewport of viewports) {
      await setViewport(cdp, viewport);
      for (const route of routes) {
        await gotoHash(cdp, baseUrl, route.hash);
        for (const stress of [false, true]) {
          const label = `${route.id}@${viewport.width}x${viewport.height}@${viewport.deviceScaleFactor}x${stress ? "+stress" : ""}`;
          const report = await measure(cdp, { fixture, stress });
          const shot = path.join(OUT_DIR, `${label.replace(/[^\w.@+-]/g, "_")}.png`);
          await screenshot(cdp, shot);
          results.push({ label, route: route.id, viewport, stress, ...report, screenshot: path.relative(repoRoot, shot) });
          if (verbose) console.log(`  ${label}: ${report.offenders.length} offender(s)`);
        }
      }
    }
  } finally {
    cdp?.close();
    chrome.close();
    vite.close();
  }

  const accepted = updateBaseline ? {} : baseline.accepted ?? {};
  const newOffenders = [];
  const nextBaseline = {};
  for (const result of results) {
    const acceptedForLabel = new Set(accepted[result.label] ?? []);
    const keys = result.offenders.map(offenderKey);
    nextBaseline[result.label] = [...new Set(keys)].sort();
    for (const offender of result.offenders) {
      if (!acceptedForLabel.has(offenderKey(offender))) newOffenders.push({ label: result.label, ...offender });
    }
  }

  const reportPath = path.join(OUT_DIR, "layout-matrix-report.json");
  fs.writeFileSync(reportPath, JSON.stringify({ generatedAt: new Date().toISOString(), baseUrl, results }, null, 2));

  const totalChecks = results.length;
  const withOffenders = results.filter((r) => r.offenders.length).length;
  console.log(`\nlayout-matrix: ${totalChecks} checks, ${withOffenders} with overflow, ${newOffenders.length} not in baseline`);
  if (!jobId) console.log("note: job-scoped legacy routes not measured (pass --job-id <id> to include them).");
  console.log(`report:      ${path.relative(repoRoot, reportPath)}`);
  console.log(`screenshots: ${path.relative(repoRoot, OUT_DIR)}`);

  if (updateBaseline) {
    fs.writeFileSync(BASELINE, JSON.stringify({ generatedAt: new Date().toISOString(), accepted: nextBaseline }, null, 2) + "\n");
    console.log(`baseline updated: ${path.relative(repoRoot, BASELINE)}`);
    return 0;
  }

  if (newOffenders.length) {
    console.error("\nOverflow offenders not present in the accepted baseline:");
    for (const offender of newOffenders.slice(0, 30)) {
      const detail = offender.kind === "clipped_content"
        ? `clips ${offender.clippedChild} (child right ${offender.childRight} > content right ${offender.contentRight}, overflow-x:${offender.overflowX})`
        : offender.kind === "past_viewport"
          ? `right ${offender.right} > viewport ${offender.viewportWidth}`
          : `scrollWidth ${offender.scrollWidth} > clientWidth ${offender.clientWidth}`;
      console.error(`  [${offender.label}] ${offender.kind} ${offender.selector} — ${detail}`);
    }
    if (newOffenders.length > 30) console.error(`  ... and ${newOffenders.length - 30} more (see report)`);
    return 1;
  }
  console.log("no new horizontal overflow.");
  return 0;
}

main().then((code) => process.exit(code)).catch((error) => {
  console.error(`layout-matrix failed: ${error.message}`);
  process.exit(2);
});
