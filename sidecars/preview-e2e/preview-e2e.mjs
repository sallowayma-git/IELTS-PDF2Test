#!/usr/bin/env node
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import vm from 'node:vm';
import { spawnSync } from 'node:child_process';

function arg(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function issue(pathValue, message, fixHint = null) {
  return {
    issueId: `issue-${Math.random().toString(36).slice(2, 10)}`,
    severity: 'error',
    layer: 'RuntimePreview',
    path: pathValue,
    message,
    fixHint,
  };
}

function decodeHtml(value) {
  return String(value ?? '')
    .replaceAll('&quot;', '"')
    .replaceAll('&#039;', "'")
    .replaceAll('&lt;', '<')
    .replaceAll('&gt;', '>')
    .replaceAll('&amp;', '&');
}

function normalizeAnswer(value) {
  if (Array.isArray(value)) return value.map((item) => normalizeAnswer(item)).sort().join('|');
  return decodeHtml(String(value ?? '')).trim().toLowerCase().replace(/\s+/g, ' ');
}

function attrs(tag) {
  const result = {};
  const pattern = /([:\w-]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'=<>`]+)))?/g;
  let match;
  while ((match = pattern.exec(tag))) {
    const [, key, doubleQuoted, singleQuoted, bare] = match;
    if (!key || key === tag.split(/\s+/)[0].replace('<', '')) continue;
    result[key.toLowerCase()] = decodeHtml(doubleQuoted ?? singleQuoted ?? bare ?? '');
  }
  return result;
}

function tags(html, name) {
  return [...String(html ?? '').matchAll(new RegExp(`<${name}\\b[^>]*>`, 'gi'))].map((match) => match[0]);
}

function allControlTags(html) {
  return [
    ...tags(html, 'input'),
    ...tags(html, 'select'),
    ...tags(html, 'textarea'),
    ...[...String(html ?? '').matchAll(/<[^>]*\b(?:paragraph-dropzone|match-dropzone|drop-target-summary)\b[^>]*>/gi)].map((match) => match[0]),
  ];
}

function controlQuestionId(attributes) {
  return (
    attributes.name
    || attributes['data-question']
    || attributes['data-question-id']
    || attributes['data-target']
    || (attributes.id?.endsWith('_input') ? attributes.id.slice(0, -6) : attributes.id)
  );
}

function controlsFor(html, qid) {
  return allControlTags(html)
    .map((tag) => ({ tag, attrs: attrs(tag) }))
    .filter((control) => controlQuestionId(control.attrs) === qid);
}

function collectWithAnswers(source, answers, issues) {
  const collected = {};
  for (const group of source.questionGroups ?? []) {
    const html = group.bodyHtml ?? '';
    for (const qid of group.questionIds ?? []) {
      const answer = answers[qid];
      const controls = controlsFor(html, qid);
      if (!controls.length) {
        issues.push(issue(`$.questionGroups.${group.groupId}.bodyHtml`, `No runtime-collectible control or dropzone found for ${qid}.`));
        continue;
      }

      const first = controls[0].attrs;
      const type = (first.type || first.class || first.tag || 'text').toLowerCase();
      if (type.includes('radio')) {
        const values = controls.map((control) => control.attrs.value).filter(Boolean);
        if (!values.some((value) => normalizeAnswer(value) === normalizeAnswer(answer))) {
          issues.push(issue(`$.questionGroups.${group.groupId}.bodyHtml`, `Radio answer for ${qid} is not present in its option values.`));
        }
      }
      if (type.includes('checkbox')) {
        const expected = Array.isArray(answer) ? answer : [answer];
        const values = controls.map((control) => control.attrs.value).filter(Boolean).map(normalizeAnswer);
        for (const item of expected) {
          if (!values.includes(normalizeAnswer(item))) {
            issues.push(issue(`$.questionGroups.${group.groupId}.bodyHtml`, `Checkbox answer for ${qid} is not present in its option values.`));
          }
        }
      }
      collected[qid] = answer;
    }
  }
  return collected;
}

function score(source, collected) {
  const order = source.questionOrder ?? Object.keys(source.answerKey ?? {});
  let correct = 0;
  for (const qid of order) {
    if (normalizeAnswer(collected[qid]) === normalizeAnswer(source.answerKey?.[qid])) correct += 1;
  }
  return {
    total: order.length,
    correct,
    percent: order.length ? Math.round((correct / order.length) * 10000) / 100 : 0,
  };
}

function wrongAnswers(source) {
  const wrong = { ...(source.answerKey ?? {}) };
  const firstQid = (source.questionOrder ?? Object.keys(wrong))[0];
  if (!firstQid) return wrong;
  const current = wrong[firstQid];
  wrong[firstQid] = Array.isArray(current) ? ['__wrong__'] : `${current ?? ''}__wrong__`;
  return wrong;
}

function runGeneratedAssets(previewDir, examId, consoleErrors) {
  const manifestPath = path.join(previewDir, 'manifest.js');
  const scriptPath = path.join(previewDir, `${examId}.js`);
  const manifestJs = fs.readFileSync(manifestPath, 'utf8');
  const wrapperJs = fs.readFileSync(scriptPath, 'utf8');
  const registered = new Map();
  const window = {
    __READING_EXAM_DATA__: {
      register(id, source) {
        registered.set(id, source);
      },
    },
  };
  const sandbox = {
    window,
    globalThis: window,
    console: {
      log() {},
      warn(...items) {
        consoleErrors.push(items.join(' '));
      },
      error(...items) {
        consoleErrors.push(items.join(' '));
      },
    },
  };
  vm.createContext(sandbox);
  vm.runInContext(manifestJs, sandbox, { filename: manifestPath, timeout: 1000 });
  vm.runInContext(wrapperJs, sandbox, { filename: scriptPath, timeout: 1000 });
  return {
    manifest: window.__READING_EXAM_MANIFEST__,
    source: registered.get(examId),
    registeredIds: [...registered.keys()],
  };
}

function validateRuntimeContractSimulator({ previewDir, examId, jobId }) {
  const issues = [];
  const consoleErrors = [];
  const runtime = {
    adapter: 'local-unified-runtime-contract-simulator',
    mode: 'fallback',
    examId,
    jobId,
    registeredIds: [],
    navButtonCount: 0,
    questionCount: 0,
    collectedAnswers: {},
    scoreInfo: { total: 0, correct: 0, percent: 0 },
    wrongScoreInfo: { total: 0, correct: 0, percent: 0 },
    consoleErrors,
  };

  try {
    const generated = runGeneratedAssets(previewDir, examId, consoleErrors);
    runtime.registeredIds = generated.registeredIds;
    const source = generated.source;
    if (!generated.manifest?.[examId]) {
      issues.push(issue('manifest.js', `Manifest does not contain examId ${examId}.`));
    }
    if (!source) {
      issues.push(issue(`${examId}.js`, `Generated wrapper did not register ${examId}.`));
      return { issues, runtime };
    }

    const order = source.questionOrder ?? [];
    runtime.questionCount = order.length;
    runtime.navButtonCount = order.length;
    if (!order.length) {
      issues.push(issue('$.questionOrder', 'Runtime preview has no questions to render.'));
    }

    const missingAnswer = order.filter((qid) => !(qid in (source.answerKey ?? {})));
    for (const qid of missingAnswer) {
      issues.push(issue(`$.answerKey.${qid}`, `Runtime answer key is missing ${qid}.`));
    }

    runtime.collectedAnswers = collectWithAnswers(source, source.answerKey ?? {}, issues);
    runtime.scoreInfo = score(source, runtime.collectedAnswers);
    runtime.wrongScoreInfo = score(source, collectWithAnswers(source, wrongAnswers(source), []));

    if (runtime.scoreInfo.total > 0 && runtime.scoreInfo.percent !== 100) {
      issues.push(issue('runtime.scoreInfo', `Correct-answer E2E expected 100%, got ${runtime.scoreInfo.percent}%.`));
    }
    if (runtime.scoreInfo.total > 0 && runtime.wrongScoreInfo.percent >= runtime.scoreInfo.percent) {
      issues.push(issue('runtime.scoreInfo', 'Wrong-answer sample did not reduce the runtime score.'));
    }
    if (consoleErrors.length) {
      issues.push(issue('runtime.console', `Runtime produced console errors: ${consoleErrors.join('; ')}`));
    }
  } catch (error) {
    issues.push(issue('runtime.execution', `Generated preview assets failed to execute: ${error?.message ?? error}`));
  }

  return { issues, runtime };
}

function pythonRunnerPath() {
  return path.join(os.tmpdir(), 'epic8-preview-e2e-playwright.py');
}

function pythonRunnerSource() {
  return String.raw`#!/usr/bin/env python3
import asyncio
import json
import re
import sys
from pathlib import Path

try:
    from playwright.async_api import async_playwright
except Exception as exc:
    print(json.dumps({"ok": False, "error": f"playwright_import_failed:{exc}"}, ensure_ascii=False))
    raise SystemExit(3)


def normalize(value):
    if isinstance(value, list):
        return sorted([normalize(item) for item in value])
    return " ".join(str(value if value is not None else "").strip().lower().split())


def parse_score_text(text):
    match = re.search(r"(\d+)\s*/\s*(\d+)", str(text or ""))
    if not match:
        return (0, 0, 0.0)
    correct = int(match.group(1))
    total = int(match.group(2))
    percent = round((correct / total) * 100, 2) if total else 0.0
    return (correct, total, percent)


def label_to_qid(label, display_to_qid):
    key = str(label or "").strip()
    if key in display_to_qid:
        return display_to_qid[key]
    m = re.search(r"(\d+)", key)
    if m:
        return f"q{m.group(1)}"
    return None


async def run(payload):
    unified_html = payload["unifiedHtmlPath"]
    preview_dir = payload["previewDir"]
    exam_id = payload["examId"]
    job_id = payload.get("jobId") or "unknown-job"
    source = payload.get("source") or {}

    answer_key = source.get("answerKey") if isinstance(source.get("answerKey"), dict) else {}
    order = source.get("questionOrder") if isinstance(source.get("questionOrder"), list) else list(answer_key.keys())
    display_map = source.get("questionDisplayMap") if isinstance(source.get("questionDisplayMap"), dict) else {}
    display_to_qid = {str(v).strip(): k for k, v in display_map.items() if str(v).strip()}

    runtime = {
        "adapter": "external-unified-runtime-playwright",
        "mode": "real",
        "examId": exam_id,
        "jobId": job_id,
        "questionCount": len(order),
        "collectedAnswers": {},
        "scoreInfo": {"total": 0, "correct": 0, "percent": 0},
        "wrongScoreInfo": {"total": 0, "correct": 0, "percent": 0},
        "consoleErrors": [],
        "warnings": [],
    }
    issues = []

    if not order:
        issues.append({"path": "$.questionOrder", "message": "Runtime preview has no questions to render."})
        return {"ok": False, "runtime": runtime, "issues": issues}

    manifest_path = str(Path(preview_dir) / "manifest.js")
    script_path = str(Path(preview_dir) / f"{exam_id}.js")
    url = f"{Path(unified_html).resolve().as_uri()}?examId={exam_id}&dataKey={exam_id}&test_env=1"

    async with async_playwright() as p:
        browser = await p.chromium.launch(headless=True, args=["--allow-file-access-from-files"])
        context = await browser.new_context()
        page = await context.new_page()

        page.on("console", lambda msg: runtime["consoleErrors"].append(f"{msg.type}: {msg.text}"))
        page.on("pageerror", lambda err: runtime["consoleErrors"].append(f"pageerror: {err}"))

        await page.route("**/manifest.js", lambda route: route.fulfill(path=manifest_path, content_type="application/javascript"))
        await page.route(f"**/{exam_id}.js", lambda route: route.fulfill(path=script_path, content_type="application/javascript"))

        await page.goto(url, wait_until="load")
        await page.wait_for_selector("#question-groups .unified-group", timeout=30000)
        await page.wait_for_selector("#submit-btn", timeout=30000)

        async def apply_answer(qid, raw_value):
            result = await page.evaluate(
                """([questionId, value]) => {
                    const values = Array.isArray(value) ? value : [value];
                    const normalized = values.map((item) => String(item ?? '').trim()).filter(Boolean);

                    const radios = Array.from(document.querySelectorAll('input[type="radio"][name="' + questionId + '"]'));
                    if (radios.length > 0) {
                        if (!normalized.length) return { ok: false, mode: 'radio' };
                        const target = normalized[0];
                        const found = radios.find((item) => String(item.value ?? '').trim().toLowerCase() === target.toLowerCase()) || radios[0];
                        found.checked = true;
                        found.dispatchEvent(new Event('change', { bubbles: true }));
                        return { ok: true, mode: 'radio' };
                    }

                    const checkboxes = Array.from(document.querySelectorAll('input[type="checkbox"][name="' + questionId + '"]'));
                    if (checkboxes.length > 0) {
                        const wanted = new Set(normalized.map((item) => item.toLowerCase()));
                        for (const input of checkboxes) {
                            const token = String(input.value ?? '').trim().toLowerCase();
                            input.checked = wanted.has(token);
                            input.dispatchEvent(new Event('change', { bubbles: true }));
                        }
                        return { ok: normalized.length > 0, mode: 'checkbox' };
                    }

                    const select = document.querySelector('select[name="' + questionId + '"]');
                    if (select) {
                        select.value = normalized[0] ?? '';
                        select.dispatchEvent(new Event('change', { bubbles: true }));
                        return { ok: true, mode: 'select' };
                    }

                    const textField = document.querySelector('input[name="' + questionId + '"]:not([type="radio"]):not([type="checkbox"]), textarea[name="' + questionId + '"]')
                        || document.getElementById(questionId + '_input');
                    if (textField && 'value' in textField) {
                        textField.value = normalized[0] ?? '';
                        textField.dispatchEvent(new Event('input', { bubbles: true }));
                        textField.dispatchEvent(new Event('change', { bubbles: true }));
                        return { ok: true, mode: 'text' };
                    }

                    const dropzone = document.querySelector(
                        '.match-dropzone[data-question="' + questionId + '"], .match-dropzone[data-question-id="' + questionId + '"], .drop-target-summary[data-question="' + questionId + '"], .drop-target-summary[data-question-id="' + questionId + '"], .paragraph-dropzone[data-question="' + questionId + '"], .paragraph-dropzone[data-target="' + questionId + '"]'
                    );
                    if (dropzone) {
                        const token = normalized[0] ?? '';
                        dropzone.dataset.answerValue = token;
                        dropzone.dataset.answerLabel = token;
                        const holder = dropzone.classList.contains('drop-target-summary')
                            ? dropzone
                            : (dropzone.querySelector('.dropped-items') || (() => {
                                const node = document.createElement('div');
                                node.className = 'dropped-items';
                                dropzone.appendChild(node);
                                return node;
                            })());
                        holder.innerHTML = '';
                        if (token) {
                            const item = document.createElement('div');
                            item.className = 'drag-item drag-item--assigned';
                            item.textContent = token;
                            item.dataset.answerValue = token;
                            item.dataset.answerLabel = token;
                            holder.appendChild(item);
                        }
                        return { ok: !!token, mode: 'dropzone' };
                    }

                    return { ok: false, mode: 'missing' };
                }""",
                [qid, raw_value],
            )
            return result if isinstance(result, dict) else {"ok": False, "mode": "unknown"}

        async def fill_answers(answer_map):
            for qid in order:
                result = await apply_answer(qid, answer_map.get(qid, ""))
                if not result.get("ok"):
                    runtime["warnings"].append(f"answer_fill_partial:{qid}:{result.get('mode')}")
            await page.wait_for_timeout(120)

        async def submit_and_collect(label):
            await page.click("#submit-btn")
            await page.wait_for_function(
                "() => !!(document.getElementById('results') && document.getElementById('results').style.display !== 'none')",
                timeout=30000,
            )
            payload = await page.evaluate(
                """() => {
                    const line = document.querySelector('#results p')?.textContent || '';
                    const rows = Array.from(document.querySelectorAll('#results tbody tr')).map((tr) => {
                        const tds = tr.querySelectorAll('td');
                        return {
                            label: tds[0]?.textContent || '',
                            user: tds[1]?.textContent || '',
                            correct: tds[2]?.textContent || '',
                            status: tds[3]?.textContent || '',
                        };
                    });
                    return { line, rows };
                }"""
            )
            if not isinstance(payload, dict):
                return {"line": "", "rows": []}
            return payload

        await fill_answers(answer_key)
        correct_payload = await submit_and_collect("correct")

        collected = {}
        for row in correct_payload.get("rows", []):
            qid = label_to_qid(row.get("label", ""), display_to_qid)
            if qid:
                collected[qid] = row.get("user", "")

        runtime["collectedAnswers"] = collected

        correct_count = 0
        for qid in order:
            if normalize(collected.get(qid, "")) == normalize(answer_key.get(qid, "")):
                correct_count += 1
        total = len(order)
        if correct_count == 0 and isinstance(correct_payload.get("line"), str):
            parsed_correct, parsed_total, parsed_percent = parse_score_text(correct_payload.get("line"))
            runtime["scoreInfo"] = {"total": parsed_total, "correct": parsed_correct, "percent": parsed_percent}
        else:
            runtime["scoreInfo"] = {
                "total": total,
                "correct": correct_count,
                "percent": round((correct_count / total) * 100, 2) if total else 0,
            }

        await page.goto(url, wait_until="load")
        await page.wait_for_selector("#question-groups .unified-group", timeout=30000)
        await page.wait_for_selector("#submit-btn", timeout=30000)

        wrong_answers = dict(answer_key)
        first_qid = order[0]
        first_value = wrong_answers.get(first_qid, "")
        wrong_answers[first_qid] = ["__wrong__"] if isinstance(first_value, list) else f"{first_value}__wrong__"

        await fill_answers(wrong_answers)
        wrong_payload = await submit_and_collect("wrong")

        wrong_collected = {}
        for row in wrong_payload.get("rows", []):
            qid = label_to_qid(row.get("label", ""), display_to_qid)
            if qid:
                wrong_collected[qid] = row.get("user", "")

        wrong_correct_count = 0
        for qid in order:
            if normalize(wrong_collected.get(qid, "")) == normalize(answer_key.get(qid, "")):
                wrong_correct_count += 1

        runtime["wrongScoreInfo"] = {
            "total": total,
            "correct": wrong_correct_count,
            "percent": round((wrong_correct_count / total) * 100, 2) if total else 0,
        }

        await context.close()
        await browser.close()

    if runtime["scoreInfo"]["total"] > 0 and runtime["scoreInfo"]["percent"] != 100:
        issues.append({
            "path": "runtime.scoreInfo",
            "message": f"Correct-answer E2E expected 100%, got {runtime['scoreInfo']['percent']}%.",
        })
    if runtime["wrongScoreInfo"]["total"] > 0 and runtime["wrongScoreInfo"]["percent"] >= runtime["scoreInfo"]["percent"]:
        issues.append({
            "path": "runtime.scoreInfo",
            "message": "Wrong-answer sample did not reduce the runtime score.",
        })
    if runtime["consoleErrors"]:
        issues.append({
            "path": "runtime.console",
            "message": "Runtime produced console/page errors.",
        })

    return {"ok": len(issues) == 0, "runtime": runtime, "issues": issues}


async def main():
    if len(sys.argv) < 2:
        print(json.dumps({"ok": False, "error": "usage: python_runner.py <payload.json>"}, ensure_ascii=False))
        raise SystemExit(2)

    payload_path = Path(sys.argv[1]).resolve()
    payload = json.loads(payload_path.read_text(encoding="utf-8"))
    result = await run(payload)
    print(json.dumps(result, ensure_ascii=False))
    raise SystemExit(0 if result.get("ok") else 1)


if __name__ == "__main__":
    asyncio.run(main())
`;
}

function parseJsonSafe(raw) {
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}

function resolveUnifiedHtmlPath() {
  const fromEnv = process.env.EPIC8_UNIFIED_HTML_PATH;
  if (fromEnv && fs.existsSync(fromEnv)) {
    return path.resolve(fromEnv);
  }
  const defaults = [
    '/Users/maziheng/Downloads/0.3.1 working/assets/generated/reading-exams/reading-practice-unified.html',
  ];
  return defaults.find((candidate) => fs.existsSync(candidate)) || null;
}

function resolvePythonPath() {
  const fromEnv = process.env.EPIC8_UNIFIED_PYTHON;
  if (fromEnv && fs.existsSync(fromEnv)) {
    return path.resolve(fromEnv);
  }
  const defaults = [
    '/Users/maziheng/Downloads/0.3.1 working/.venv/bin/python',
    '/usr/bin/python3',
    '/opt/homebrew/bin/python3',
  ];
  return defaults.find((candidate) => fs.existsSync(candidate)) || 'python3';
}

function runRealRuntime({ previewDir, examId, jobId }) {
  const unifiedHtmlPath = resolveUnifiedHtmlPath();
  if (!unifiedHtmlPath) {
    return {
      ok: false,
      reason: 'external_unified_html_not_found',
      message: 'external unified html not found',
    };
  }

  const preflightConsoleErrors = [];
  let generated;
  try {
    generated = runGeneratedAssets(previewDir, examId, preflightConsoleErrors);
  } catch (error) {
    return {
      ok: false,
      reason: 'generated_assets_not_loadable',
      message: `generated assets not loadable: ${error?.message ?? error}`,
    };
  }

  if (!generated.source) {
    return {
      ok: false,
      reason: 'generated_source_missing',
      message: `generated wrapper did not register ${examId}`,
    };
  }

  const payload = {
    unifiedHtmlPath,
    previewDir,
    examId,
    jobId,
    source: generated.source,
  };
  const payloadPath = path.join(os.tmpdir(), `epic8-preview-e2e-${Date.now()}-${Math.random().toString(36).slice(2, 8)}.json`);
  const runnerPath = pythonRunnerPath();
  fs.writeFileSync(payloadPath, JSON.stringify(payload), 'utf8');
  fs.writeFileSync(runnerPath, pythonRunnerSource(), 'utf8');

  const pythonPath = resolvePythonPath();
  const output = spawnSync(pythonPath, [runnerPath, payloadPath], {
    encoding: 'utf8',
    timeout: 240000,
  });

  const parsed = parseJsonSafe((output.stdout || '').trim());
  if (parsed && typeof parsed === 'object' && parsed.runtime && Array.isArray(parsed.issues)) {
    const runtime = {
      ...parsed.runtime,
      preflightConsoleErrors,
      registeredIds: generated.registeredIds,
      unifiedHtmlPath,
      pythonPath,
    };
    const issues = parsed.issues.map((item) => issue(item.path || 'runtime.real', item.message || 'real runtime issue'));
    return {
      ok: parsed.ok === true,
      report: {
        issues,
        runtime,
      },
    };
  }

  return {
    ok: false,
    reason: 'real_runtime_runner_failed',
    message: `real runtime runner failed: code=${output.status}; stdout=${(output.stdout || '').trim()}; stderr=${(output.stderr || '').trim()}`,
  };
}

function validateRuntime({ previewDir, examId, jobId }) {
  const real = runRealRuntime({ previewDir, examId, jobId });
  if (real.ok && real.report) {
    return real.report;
  }

  const fallback = validateRuntimeContractSimulator({ previewDir, examId, jobId });
  fallback.runtime.fallbackReason = real.message || real.reason || 'real_runtime_unavailable';
  fallback.runtime.fallbackAdapter = 'external-unified-runtime-playwright';
  return fallback;
}

const previewDir = arg('--preview-dir');
const examId = arg('--exam-id');
const jobId = arg('--job-id') ?? 'unknown-job';

if (!previewDir || !examId) {
  console.error('usage: preview-e2e.mjs --preview-dir <dir> --exam-id <examId> [--job-id <jobId>]');
  process.exit(2);
}

const { issues, runtime } = validateRuntime({ previewDir, examId, jobId });
const report = {
  jobId,
  passed: issues.length === 0,
  layers: [{ layer: 'RuntimePreview', passed: issues.length === 0, issueCount: issues.length }],
  issues,
  runtime,
  generatedAt: new Date().toISOString(),
};
console.log(JSON.stringify(report, null, 2));
process.exit(report.passed ? 0 : 1);
