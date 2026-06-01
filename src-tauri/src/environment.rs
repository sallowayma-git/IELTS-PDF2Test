use crate::llm_profiles::{os_secret_backend, plaintext_secret_fallback_allowed};
use chrono::Utc;
use serde_json::{json, Value};
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

fn sidecar_candidates(relative: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(cwd) = env::current_dir() {
        candidates.push(cwd.join(relative));
        candidates.push(cwd.join("..").join(relative));
    }
    if let Ok(exe) = env::current_exe() {
        if let Some(parent) = exe.parent() {
            candidates.push(parent.join(relative));
            candidates.push(parent.join("..").join(relative));
            candidates.push(parent.join("resources").join(relative));
            candidates.push(parent.join("..").join("Resources").join(relative));
            candidates.push(parent.join("..").join("resources").join(relative));
            if let Some(resource_name) = Path::new(relative).file_name() {
                candidates.push(
                    parent
                        .join("resources")
                        .join("sidecars")
                        .join(resource_name),
                );
                candidates.push(
                    parent
                        .join("..")
                        .join("Resources")
                        .join("sidecars")
                        .join(resource_name),
                );
            }
        }
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(relative),
    );
    candidates
}

pub(crate) fn find_sidecar(relative: &str) -> Option<PathBuf> {
    sidecar_candidates(relative)
        .into_iter()
        .find(|path| path.exists())
}

pub(crate) fn command_failure(command_name: &str, output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!(
        "{} exited with {:?}; stdout={}; stderr={}",
        command_name,
        output.status.code(),
        stdout.trim(),
        stderr.trim()
    )
}

pub(crate) fn command_probe(program: &str, args: &[&str]) -> Value {
    match Command::new(program).args(args).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            json!({
                "program": program,
                "ok": output.status.success(),
                "status": output.status.code(),
                "stdout": stdout,
                "stderr": stderr
            })
        }
        Err(error) => json!({
            "program": program,
            "ok": false,
            "status": null,
            "stdout": "",
            "stderr": error.to_string()
        }),
    }
}

fn preflight_item(name: &str, ok: bool, severity: &str, message: String, details: Value) -> Value {
    json!({
        "name": name,
        "ok": ok,
        "severity": severity,
        "message": message,
        "details": details
    })
}

pub(crate) fn resolve_external_unified_html() -> Option<PathBuf> {
    if let Ok(value) = env::var("EPIC8_UNIFIED_HTML_PATH") {
        let path = PathBuf::from(value);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

pub(crate) fn resolve_external_unified_python() -> Option<PathBuf> {
    if let Ok(value) = env::var("EPIC8_UNIFIED_PYTHON") {
        let path = PathBuf::from(value);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

pub(crate) fn runtime_gate_strict_mode() -> bool {
    env::var("EPIC8_RUNTIME_GATE_STRICT")
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            !matches!(normalized.as_str(), "0" | "false" | "no" | "off")
        })
        .unwrap_or(true)
}

pub(crate) fn node_validator_diagnostics_enabled() -> bool {
    env::var("EPIC8_NODE_VALIDATOR_DIAGNOSTICS")
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

pub(crate) fn environment_preflight_report() -> Value {
    let mut checks = Vec::<Value>::new();

    let node = command_probe("node", &["--version"]);
    let node_ok = node.get("ok").and_then(Value::as_bool).unwrap_or(false);
    checks.push(preflight_item(
        "node",
        node_ok,
        "warning",
        if node_ok {
            format!(
                "Node.js available for optional developer/CI diagnostics: {}",
                node.get("stdout")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            )
        } else {
            "Node.js is optional for production. It is only needed for development parity checks and explicit preview E2E diagnostics.".to_string()
        },
        node,
    ));

    checks.push(preflight_item(
        "rust:text-parser",
        true,
        "info",
        "Built-in Rust TXT/MD parsing is available for plain text and Markdown sources."
            .to_string(),
        json!({"provider": ["rust-parser:text:plain", "rust-parser:text:markdown"]}),
    ));

    checks.push(preflight_item(
        "rust:pdf-extract",
        true,
        "info",
        "Built-in Rust PDF text-layer extraction is available for clear text PDFs.".to_string(),
        json!({"crate": "pdf-extract", "version": "0.10"}),
    ));

    checks.push(preflight_item(
        "rust:docx-ooxml",
        true,
        "info",
        "Built-in Rust DOCX OOXML extraction is available for clear text DOCX files.".to_string(),
        json!({"crates": {"quick-xml": "0.39", "zip": "2.4.2"}}),
    ));

    let python = command_probe("python3", &["--version"]);
    let python_ok = python.get("ok").and_then(Value::as_bool).unwrap_or(false);
    checks.push(preflight_item(
        "python3",
        python_ok,
        "warning",
        if python_ok {
            format!(
                "Python available: {}{}",
                python
                    .get("stdout")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                python
                    .get("stderr")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(|value| format!(" {}", value))
                    .unwrap_or_default()
            )
        } else {
            "python3 is optional for TXT/MD and clear text PDF/DOCX parsing because Rust parsers are primary. It is still required for embedded PDF image extraction and legacy parser fallback; Rust can use macOS sips as a rendered-page fallback for vision transcription when Python/pypdf is unavailable.".to_string()
        },
        python,
    ));

    let pypdf = command_probe("python3", &["-c", "import pypdf; print(pypdf.__version__)"]);
    let pypdf_ok = pypdf.get("ok").and_then(Value::as_bool).unwrap_or(false);
    checks.push(preflight_item(
        "python:pypdf",
        pypdf_ok,
        "warning",
        if pypdf_ok {
            format!(
                "pypdf available: {}",
                pypdf
                    .get("stdout")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
            )
        } else {
            "pypdf is optional for TXT/MD and clear text PDF/DOCX parsing because Rust parsers are primary. It is still required for embedded PDF image extraction and Python legacy fallback; macOS sips can still render a page image for vision transcription without pypdf.".to_string()
        },
        pypdf,
    ));

    let sips = command_probe("sips", &["--version"]);
    let sips_ok = sips.get("ok").and_then(Value::as_bool).unwrap_or(false);
    checks.push(preflight_item(
        "renderer:macos-sips",
        sips_ok,
        "warning",
        if sips_ok {
            format!(
                "macOS sips renderer available: {}{}",
                sips.get("stdout")
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
                sips.get("stderr")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .map(|value| format!(" {}", value))
                    .unwrap_or_default()
            )
        } else {
            "macOS sips renderer is unavailable; scanned PDFs without embedded images will require manual transcription or a future PDFium adapter.".to_string()
        },
        sips,
    ));

    for (name, relative, severity, missing_message) in [
        (
            "sidecar:python-parser",
            "sidecars/python-parser/parser.py",
            "warning",
            Some("Legacy Python parser sidecar is missing. Production TXT/MD/PDF/DOCX parsing is Rust-primary; embedded PDF image extraction may fall back to rendered-page/manual transcription."),
        ),
        (
            "sidecar:llm-gateway",
            "sidecars/llm-gateway/gateway.mjs",
            "warning",
            Some("Legacy Node LLM gateway sidecar is missing. Production LLM calls run through the Rust OpenAI-compatible gateway."),
        ),
        (
            "sidecar:node-validator",
            "sidecars/node-validator/validate-reading-source.mjs",
            "warning",
            Some("Supplementary Node validator is missing; Rust built-in ReadingExamSourceV1/DOM validation still runs."),
        ),
        (
            "sidecar:preview-e2e",
            "sidecars/preview-e2e/preview-e2e.mjs",
            "warning",
            Some("Preview E2E sidecar is missing. Production export uses Rust static contract gates; explicit real-runtime diagnostics require this sidecar."),
        ),
    ] {
        let path = find_sidecar(relative);
        let ok = path.is_some();
        checks.push(preflight_item(
            name,
            ok,
            severity,
            if let Some(path) = path.as_ref() {
                format!("Sidecar found at {}", path.display())
            } else {
                missing_message
                    .map(ToString::to_string)
                    .unwrap_or_else(|| format!("Sidecar missing: {}", relative))
            },
            json!({
                "relative": relative,
                "path": path.map(|value| value.to_string_lossy().to_string()),
                "candidateCount": sidecar_candidates(relative).len()
            }),
        ));
    }

    let unified_html = resolve_external_unified_html();
    checks.push(preflight_item(
        "runtime:unified-html",
        unified_html.is_some(),
        "warning",
        if let Some(path) = unified_html.as_ref() {
            format!(
                "Unified runtime HTML configured for optional E2E diagnostics: {}",
                path.display()
            )
        } else {
            "EPIC8_UNIFIED_HTML_PATH is not set or does not exist. Production export can still use Rust static gates; real-runtime E2E diagnostics require this path.".to_string()
        },
        json!({
            "env": env::var("EPIC8_UNIFIED_HTML_PATH").ok(),
            "path": unified_html.map(|value| value.to_string_lossy().to_string())
        }),
    ));

    let unified_python = resolve_external_unified_python();
    checks.push(preflight_item(
        "runtime:unified-python",
        unified_python.is_some(),
        "warning",
        if let Some(path) = unified_python.as_ref() {
            format!(
                "Unified runtime Python configured for optional E2E diagnostics: {}",
                path.display()
            )
        } else {
            "EPIC8_UNIFIED_PYTHON is not set or does not exist. Production export can still use Rust static gates; real-runtime E2E diagnostics require this path.".to_string()
        },
        json!({
            "env": env::var("EPIC8_UNIFIED_PYTHON").ok(),
            "path": unified_python.map(|value| value.to_string_lossy().to_string())
        }),
    ));

    let strict = runtime_gate_strict_mode();
    checks.push(preflight_item(
        "runtime:strict-gate",
        strict,
        "warning",
        if strict {
            "Production Rust static contract gate is enabled. Real-runtime E2E is available as an explicit diagnostic command.".to_string()
        } else {
            "Production static runtime gate is disabled; publish safety is weaker.".to_string()
        },
        json!({"env": env::var("EPIC8_RUNTIME_GATE_STRICT").ok()}),
    ));

    let plaintext_fallback = plaintext_secret_fallback_allowed();
    checks.push(preflight_item(
        "security:os-secret-storage",
        true,
        "info",
        format!(
            "API keys use OS secure storage by default through the cross-platform keyring adapter: {}.",
            os_secret_backend()
        ),
        json!({"backend": os_secret_backend()}),
    ));
    checks.push(preflight_item(
        "security:plaintext-secret-fallback",
        !plaintext_fallback,
        "warning",
        if plaintext_fallback {
            "Plaintext API key file fallback is enabled by EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK. Use only for development or emergency recovery.".to_string()
        } else {
            "Plaintext API key file fallback is disabled; API keys use OS secure storage by default."
                .to_string()
        },
        json!({"env": env::var("EPIC8_ALLOW_PLAINTEXT_SECRET_FALLBACK").ok()}),
    ));

    let errors = checks
        .iter()
        .filter(|check| {
            !check.get("ok").and_then(Value::as_bool).unwrap_or(false)
                && check.get("severity").and_then(Value::as_str) == Some("error")
        })
        .count();
    let warnings = checks
        .iter()
        .filter(|check| {
            !check.get("ok").and_then(Value::as_bool).unwrap_or(false)
                && check.get("severity").and_then(Value::as_str) == Some("warning")
        })
        .count();

    json!({
        "schemaVersion": "EnvironmentPreflightV1",
        "ok": errors == 0,
        "errors": errors,
        "warnings": warnings,
        "checks": checks,
        "generatedAt": Utc::now().to_rfc3339()
    })
}
