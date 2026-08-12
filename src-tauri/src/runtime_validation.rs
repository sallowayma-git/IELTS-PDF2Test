use crate::authoring_review::authoring_review_issues;
use crate::authoring_validation::{
    merge_sidecar_validation, merge_validation_issues, validate_authoring,
};
use crate::diagnostics::load_diagnostics_settings;
use crate::environment::{
    command_failure, find_sidecar, node_validator_diagnostics_enabled, runtime_gate_strict_mode,
};
use crate::export_artifacts::build_reading_asset_bundle;
use crate::reading_source::reading_source;
use crate::source_review::{source_review_issues, source_review_status_for_job};
use crate::util::{job_dir, validate_path_segment, write_json, write_text};
use crate::validator::json_issue;
use crate::CommandResult;
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

fn inline_script(code: &str) -> String {
    code.replace("</script>", "<\\/script>")
}

fn resolve_preview_runtime_html_path() -> Option<PathBuf> {
    let env_path = std::env::var("EPIC8_UNIFIED_RUNTIME_HTML_PATH")
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.exists());
    env_path.or_else(|| {
        let candidate = PathBuf::from(
            "/Users/maziheng/Downloads/0.3.1 working/assets/generated/reading-exams/reading-practice-unified.html",
        );
        candidate.exists().then_some(candidate)
    })
}

fn resolve_preview_runtime_js_path() -> Option<PathBuf> {
    let env_path = std::env::var("EPIC8_UNIFIED_RUNTIME_JS_PATH")
        .ok()
        .map(PathBuf::from)
        .filter(|path| path.exists());
    env_path.or_else(|| {
        let candidate = PathBuf::from(
            "/Users/maziheng/Downloads/0.3.1 working/js/runtime/unifiedReadingPage.js",
        );
        candidate.exists().then_some(candidate)
    })
}

fn preview_bridge_script(exam_id: &str) -> String {
    let exam_id_literal = serde_json::to_string(exam_id).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        r#"(function initAuthorPreviewBridge() {{
  const examId = {exam_id_literal};
  const bridgeSource = "author_preview_bridge";
  const readOnlyInit = () => {{
    try {{
      window.postMessage({{
        type: "INIT_SESSION",
        data: {{
          examId,
          dataKey: examId,
          reviewMode: true,
          readOnly: true
        }}
      }}, "*");
    }} catch (_error) {{
      // ignore preview bridge init errors
    }}
  }};
  const resolveQuestionId = (node) => {{
    const element = node instanceof Element ? node : node && node.parentElement;
    if (!element) return "";
    const holder = element.closest("[data-question-id]");
    if (holder && holder.getAttribute("data-question-id")) return holder.getAttribute("data-question-id") || "";
    const control = element.closest("input[name], textarea[name], select[name], [id$='_input']");
    if (!control) return "";
    const name = control.getAttribute("name");
    if (name) return name;
    const id = control.getAttribute("id") || "";
    return id.endsWith("_input") ? id.slice(0, -6) : id;
  }};
  const highlightQuestion = (questionId) => {{
    if (!questionId) return;
    document.querySelectorAll(".author-preview-selected").forEach((node) => node.classList.remove("author-preview-selected"));
    const selectors = [
      `[data-question-id="${{questionId}}"]`,
      `input[name="${{questionId}}"]`,
      `textarea[name="${{questionId}}"]`,
      `select[name="${{questionId}}"]`,
      `#${{CSS.escape(questionId)}}_input`
    ];
    const target = document.querySelector(selectors.join(", "));
    if (!(target instanceof Element)) return;
    const container = target.closest("[data-question-id], li, tr, .question-item, .match-question-item, .choice-item, .tfng-item") || target;
    if (container instanceof HTMLElement) {{
      container.classList.add("author-preview-selected");
      container.scrollIntoView({{ block: "center", behavior: "smooth" }});
    }}
  }};
  try {{
    const params = new URLSearchParams(window.location.search || "");
    params.set("examId", examId);
    params.set("dataKey", examId);
    window.history.replaceState(null, "", `?${{params.toString()}}`);
  }} catch (_error) {{
    // ignore query-state sync errors
  }}
  document.addEventListener("click", (event) => {{
    const questionId = resolveQuestionId(event.target);
    if (!questionId || !window.parent || window.parent === window) return;
    window.parent.postMessage({{
      source: bridgeSource,
      type: "question-click",
      examId,
      questionId
    }}, "*");
  }}, true);
  window.addEventListener("message", (event) => {{
    const payload = event && event.data;
    if (!payload || typeof payload !== "object" || payload.source !== "author_editor") return;
    if (payload.type === "select-question") {{
      highlightQuestion(typeof payload.questionId === "string" ? payload.questionId : "");
    }}
  }});
  document.addEventListener("DOMContentLoaded", () => {{
    const style = document.createElement("style");
    style.textContent = ".author-preview-selected{{outline:2px solid #d46836;outline-offset:4px;border-radius:8px;background:rgba(212,104,54,.08);}}";
    document.head.appendChild(style);
    window.setTimeout(readOnlyInit, 120);
    window.setTimeout(readOnlyInit, 500);
  }});
}})();"#,
    )
}

fn build_unified_runtime_html(
    exam_id: &str,
    manifest_js: &str,
    wrapper_js: &str,
) -> Option<String> {
    let template_path = resolve_preview_runtime_html_path()?;
    let runtime_js_path = resolve_preview_runtime_js_path()?;
    let runtime_dir = runtime_js_path.parent()?;
    let mut html = fs::read_to_string(template_path).ok()?;
    let registry_js = fs::read_to_string(runtime_dir.join("readingExamRegistry.js")).ok()?;
    let explanation_registry_js =
        fs::read_to_string(runtime_dir.join("readingExplanationRegistry.js")).ok()?;
    let highlight_shared_js =
        fs::read_to_string(runtime_dir.join("readingHighlightShared.js")).ok()?;
    let review_dictionary_js =
        fs::read_to_string(runtime_dir.join("reviewHighlightDictionary.js")).ok()?;
    let runtime_js = fs::read_to_string(runtime_js_path).ok()?;
    html = html.replace(r#"<script src="./manifest.js"></script>"#, "");
    html = html.replace(
        r#"<script src="../../../js/bundles/reading-page.bundle.js"></script>"#,
        "",
    );
    let scripts = format!(
        r#"
    <script>{}</script>
    <script>{}</script>
    <script>{}</script>
    <script>{}</script>
    <script>{}</script>
    <script>{}</script>
    <script>{}</script>
    <script>{}</script>
"#,
        inline_script(&registry_js),
        inline_script(&explanation_registry_js),
        inline_script(&highlight_shared_js),
        inline_script(&review_dictionary_js),
        inline_script(manifest_js),
        inline_script(wrapper_js),
        inline_script(&runtime_js),
        inline_script(&preview_bridge_script(exam_id)),
    );
    if html.contains("</body>") {
        html = html.replace("</body>", &format!("{scripts}\n</body>"));
        Some(html)
    } else {
        None
    }
}

pub(crate) fn validate_with_node_sidecar(
    root: &Path,
    job_id: &str,
    source: &Value,
) -> CommandResult<Value> {
    let script = find_sidecar("sidecars/node-validator/validate-reading-source.mjs")
        .ok_or_else(|| "node_validator_sidecar_missing".to_string())?;
    let input_path = job_dir(root, job_id)
        .join("cache")
        .join("reading-source-for-validation.json");
    write_json(&input_path, source)?;
    let output = Command::new("node")
        .arg(&script)
        .arg(&input_path)
        .output()
        .map_err(|error| format!("node_validator_spawn_failed:{}:{}", script.display(), error))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed = serde_json::from_str::<Value>(&stdout)
        .map_err(|error| format!("node_validator_json_failed:{}:{}", error, stdout.trim()))?;
    if !output.status.success() && parsed.get("passed").and_then(Value::as_bool) != Some(false) {
        return Err(command_failure("node-validator", &output));
    }
    Ok(parsed)
}

pub(crate) fn validate_preview_with_node_sidecar(
    root: &Path,
    job_id: &str,
    preview_dir: &Path,
    exam_id: &str,
    unified_html_path: Option<&Path>,
    unified_python_path: Option<&Path>,
) -> CommandResult<Value> {
    let script = find_sidecar("sidecars/preview-e2e/preview-e2e.mjs")
        .ok_or_else(|| "preview_e2e_sidecar_missing".to_string())?;
    let mut command = Command::new("node");
    command
        .arg(&script)
        .arg("--preview-dir")
        .arg(preview_dir)
        .arg("--exam-id")
        .arg(exam_id)
        .arg("--job-id")
        .arg(job_id);
    if let Some(path) = unified_html_path {
        command.env("EPIC8_UNIFIED_HTML_PATH", path);
    }
    if let Some(path) = unified_python_path {
        command.env("EPIC8_UNIFIED_PYTHON", path);
    }
    let output = command
        .output()
        .map_err(|error| format!("preview_e2e_spawn_failed:{}:{}", script.display(), error))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed = serde_json::from_str::<Value>(&stdout)
        .map_err(|error| format!("preview_e2e_json_failed:{}:{}", error, stdout.trim()))?;
    if !output.status.success() && parsed.get("passed").and_then(Value::as_bool) != Some(false) {
        return Err(command_failure("preview-e2e", &output));
    }
    let output_path = job_dir(root, job_id)
        .join("preview")
        .join("preview-e2e-report.json");
    write_json(&output_path, &parsed)?;
    Ok(parsed)
}

pub(crate) fn preview_assets_for_source(
    root: &Path,
    job_id: &str,
    source: &Value,
) -> CommandResult<(String, PathBuf, String, String, Value)> {
    validate_path_segment("job_id", job_id)?;
    let bundle = build_reading_asset_bundle(source)?;
    let preview_dir = job_dir(root, job_id).join("preview");
    write_text(
        &preview_dir.join(format!("{}.js", bundle.exam_id)),
        &bundle.wrapper_js,
    )?;
    write_text(&preview_dir.join("manifest.js"), &bundle.manifest_js)?;
    let runtime_html =
        build_unified_runtime_html(&bundle.exam_id, &bundle.manifest_js, &bundle.wrapper_js);
    let assets = json!({"examId": bundle.exam_id, "manifestPath": preview_dir.join("manifest.js").to_string_lossy(), "scriptPath": preview_dir.join(format!("{}.js", bundle.exam_id)).to_string_lossy(), "previewUrl": format!("tauri-local://preview/{}", bundle.source.get("examId").and_then(Value::as_str).unwrap_or("local-authoring-exam")), "source": bundle.source, "wrapperJs": bundle.wrapper_js, "manifestJs": bundle.manifest_js, "runtimeHtml": runtime_html});
    write_json(&preview_dir.join("preview-assets.json"), &assets)?;
    Ok((
        bundle.exam_id,
        preview_dir,
        bundle.wrapper_js,
        bundle.manifest_js,
        assets,
    ))
}

pub(crate) fn validate_for_runtime_gate(
    root: &Path,
    job_id: &str,
    ir: &Value,
    require_static_runtime_gate: bool,
) -> CommandResult<Value> {
    let source = reading_source(ir);
    let mut report = validate_authoring(job_id, Some(ir));
    if report
        .get("passed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let _ = preview_assets_for_source(root, job_id, &source)?;
        if require_static_runtime_gate && !runtime_gate_strict_mode() {
            merge_validation_issues(
                &mut report,
                vec![json!({
                    "issueId": format!("issue-{}", Uuid::new_v4().simple()),
                    "severity": "warning",
                    "layer": "RuntimePreview",
                    "path": "runtime.staticGate",
                    "message": "Production static runtime gate was explicitly disabled by EPIC8_RUNTIME_GATE_STRICT.",
                    "fixHint": "Enable EPIC8_RUNTIME_GATE_STRICT for production exports."
                })],
            );
        }
        if let Some(obj) = report.as_object_mut() {
            obj.insert(
                "runtime".to_string(),
                json!({
                    "mode": "static-rust",
                    "adapter": "rust-static-contract",
                    "diagnosticE2e": "not-run",
                    "fallbackReason": null
                }),
            );
        }
    }
    write_json(
        &job_dir(root, job_id).join("validation-report.json"),
        &report,
    )?;
    Ok(report)
}

pub(crate) fn run_node_validator_diagnostic(
    root: &Path,
    job_id: &str,
    report: &mut Value,
    source: &Value,
) {
    if !node_validator_diagnostics_enabled() {
        return;
    }
    match validate_with_node_sidecar(root, job_id, source) {
        Ok(mut sidecar_report) => {
            if let Some(obj) = sidecar_report.as_object_mut() {
                obj.insert("replaceExistingLayers".to_string(), json!(false));
            }
            merge_sidecar_validation(report, sidecar_report);
        }
        Err(error) => {
            merge_validation_issues(
                report,
                vec![json!({
                    "issueId": format!("issue-{}", Uuid::new_v4().simple()),
                    "severity": "warning",
                    "layer": "ReadingExamSourceV1",
                    "path": "$",
                    "message": format!("Node validator diagnostic unavailable; Rust built-in ReadingExamSourceV1/DOM validation was used: {}", error),
                    "fixHint": "Set up Node.js only if development parity diagnostics are needed."
                })],
            );
        }
    }
}

pub(crate) fn publish_readiness_gate(
    root: &Path,
    job_id: &str,
    ir: &Value,
    mut runtime_report: Value,
) -> CommandResult<Value> {
    let dir = job_dir(root, job_id);
    let source_review = source_review_status_for_job(root, job_id)?;
    let human_verified = ir.pointer("/audit/humanVerified").and_then(Value::as_bool) == Some(true);
    let mut issues = Vec::new();

    issues.extend(source_review_issues(&source_review));
    if !human_verified {
        issues.push(json_issue(
            "AuthoringIR",
            "$.audit.humanVerified",
            "All questions must be human verified before publish",
        ));
    }
    issues.extend(authoring_review_issues(ir));

    merge_validation_issues(&mut runtime_report, issues);
    if load_diagnostics_settings(root)
        .map(|settings| settings.keep_full_process_artifacts)
        .unwrap_or(false)
    {
        write_json(&dir.join("publish-readiness-report.json"), &runtime_report)?;
    }
    Ok(runtime_report)
}
