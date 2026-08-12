use crate::llm_profiles::{os_secret_backend, plaintext_secret_fallback_allowed};
use chrono::Utc;
use serde_json::{json, Value};
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Clone)]
pub(crate) struct ResolvedCommand {
    pub(crate) program: String,
    pub(crate) args: Vec<String>,
    pub(crate) source: String,
}

impl ResolvedCommand {
    fn new(program: impl Into<String>, args: Vec<String>, source: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args,
            source: source.into(),
        }
    }

    pub(crate) fn display(&self) -> String {
        let mut parts = vec![self.program.clone()];
        parts.extend(self.args.iter().cloned());
        parts.join(" ")
    }
}

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

pub(crate) fn command_probe_resolved(command: &ResolvedCommand, extra_args: &[&str]) -> Value {
    match Command::new(&command.program)
        .args(&command.args)
        .args(extra_args)
        .output()
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            json!({
                "program": command.program.clone(),
                "baseArgs": command.args.clone(),
                "extraArgs": extra_args,
                "commandLine": command.display(),
                "source": command.source.clone(),
                "ok": output.status.success(),
                "status": output.status.code(),
                "stdout": stdout,
                "stderr": stderr
            })
        }
        Err(error) => json!({
            "program": command.program.clone(),
            "baseArgs": command.args.clone(),
            "extraArgs": extra_args,
            "commandLine": command.display(),
            "source": command.source.clone(),
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

fn env_flag_enabled(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .map(|value| {
            let normalized = value.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => true,
                "0" | "false" | "no" | "off" => false,
                _ => default,
            }
        })
        .unwrap_or(default)
}

pub(crate) fn cloud_pdf_vision_enabled() -> bool {
    env_flag_enabled("EPIC8_ENABLE_CLOUD_PDF_VISION", false)
}

pub(crate) fn local_ocr_enabled() -> bool {
    env_flag_enabled("EPIC8_ENABLE_LOCAL_OCR", false)
}

pub(crate) fn document_ir_v2_shadow_enabled() -> bool {
    cfg!(debug_assertions) && env_flag_enabled("EPIC8_DOCUMENT_IR_V2_SHADOW", false)
}

pub(crate) fn authoring_v2_shadow_enabled() -> bool {
    // Production packages may opt into the append-only V2 shadow during the
    // controlled rollout; the default remains off and V1 is untouched.
    env_flag_enabled("EPIC8_AUTHORING_V2_SHADOW", false)
}

pub(crate) fn quality_gate_v2_enabled() -> bool {
    cfg!(debug_assertions) && env_flag_enabled("EPIC8_QUALITY_GATE_V2", false)
}

pub(crate) fn pdf_renderer_setting() -> String {
    env::var("EPIC8_PDF_RENDERER")
        .ok()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| {
            matches!(
                value.as_str(),
                "auto" | "none" | "sips" | "pdfium" | "poppler" | "pymupdf"
            )
        })
        .unwrap_or_else(|| "auto".to_string())
}

fn split_command_spec(spec: &str) -> Vec<String> {
    let mut parts = Vec::<String>::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;

    for ch in spec.trim().chars() {
        match (quote, ch) {
            (Some(active), next) if next == active => quote = None,
            (None, '\'' | '"') => quote = Some(ch),
            (None, next) if next.is_whitespace() => {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            (_, next) => current.push(next),
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

pub(crate) fn python_command_candidates() -> Vec<ResolvedCommand> {
    let mut candidates = Vec::new();
    if let Ok(spec) = env::var("EPIC8_PYTHON") {
        let parts = split_command_spec(&spec);
        if let Some((program, args)) = parts.split_first() {
            candidates.push(ResolvedCommand::new(
                program.clone(),
                args.to_vec(),
                "env:EPIC8_PYTHON",
            ));
        }
    }

    #[cfg(target_os = "windows")]
    {
        candidates.push(ResolvedCommand::new(
            "py",
            vec!["-3".to_string()],
            "platform:windows",
        ));
        candidates.push(ResolvedCommand::new(
            "python",
            Vec::new(),
            "platform:windows",
        ));
    }

    #[cfg(not(target_os = "windows"))]
    {
        candidates.push(ResolvedCommand::new("python3", Vec::new(), "platform:unix"));
        candidates.push(ResolvedCommand::new("python", Vec::new(), "platform:unix"));
    }

    candidates
}

pub(crate) fn resolve_python_command() -> Option<ResolvedCommand> {
    python_command_candidates().into_iter().find(|candidate| {
        command_probe_resolved(candidate, &["--version"])
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    })
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

    let python_candidates = python_command_candidates();
    let python_probes = python_candidates
        .iter()
        .map(|candidate| command_probe_resolved(candidate, &["--version"]))
        .collect::<Vec<_>>();
    let python = python_probes
        .iter()
        .find(|probe| probe.get("ok").and_then(Value::as_bool).unwrap_or(false))
        .cloned();
    let python_ok = python.is_some();
    checks.push(preflight_item(
        "python",
        python_ok,
        "warning",
        if let Some(python) = python.as_ref() {
            format!(
                "Python available for optional legacy parser and PDF image extraction: {}{}",
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
            "Python is optional for production TXT/MD and clear text PDF/DOCX parsing because Rust parsers are primary. Configure EPIC8_PYTHON or install a platform Python only for embedded PDF image extraction and legacy parser fallback.".to_string()
        },
        json!({
            "selected": python,
            "candidates": python_probes,
            "env": env::var("EPIC8_PYTHON").ok()
        }),
    ));

    let pypdf = if let Some(python) = resolve_python_command() {
        command_probe_resolved(&python, &["-c", "import pypdf; print(pypdf.__version__)"])
    } else {
        json!({
            "ok": false,
            "status": null,
            "stdout": "",
            "stderr": "python_unavailable",
            "candidates": python_command_candidates().iter().map(ResolvedCommand::display).collect::<Vec<_>>()
        })
    };
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
            "pypdf is optional for production TXT/MD and clear text PDF/DOCX parsing. Without it, embedded PDF image extraction falls back to the platform PDF renderer or manual review.".to_string()
        },
        pypdf,
    ));

    let renderer = pdf_renderer_setting();
    let platform = env::consts::OS;
    let platform_renderer_name = match platform {
        "macos" => "renderer:macos-sips",
        "windows" => "renderer:windows-pdfium",
        _ => "renderer:unsupported",
    };
    let platform_renderer_available = if platform == "macos" && renderer != "none" {
        command_probe("sips", &["--version"])
            .get("ok")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    } else {
        false
    };
    checks.push(preflight_item(
        "renderer:pdf-page-renderer",
        platform_renderer_available,
        "warning",
        if platform_renderer_available {
            "PDF page rendering is available for vision transcription inputs through the platform adapter.".to_string()
        } else {
            "PDF page rendering is unavailable or disabled on this platform; scanned PDFs without embedded images require cloud PDF vision or manual transcription.".to_string()
        },
        json!({
            "platform": platform,
            "selectedRenderer": renderer,
            "provider": platform_renderer_name,
            "supportedRenderers": ["auto", "none", "sips", "pdfium", "poppler", "pymupdf"],
            "cloudPdfVisionEnabled": cloud_pdf_vision_enabled(),
            "localOcrEnabled": local_ocr_enabled()
        }),
    ));

    if platform == "macos" {
        let sips = command_probe("sips", &["--version"]);
        let sips_ok = sips.get("ok").and_then(Value::as_bool).unwrap_or(false);
        checks.push(preflight_item(
            "renderer:macos-sips",
            sips_ok && renderer != "none",
            "warning",
            if renderer == "none" {
                "macOS sips renderer is disabled by EPIC8_PDF_RENDERER=none; scanned PDFs require cloud PDF vision or manual transcription.".to_string()
            } else if sips_ok {
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
                "macOS sips renderer is unavailable; scanned PDFs without embedded images require cloud PDF vision, manual transcription, or a future PDFium adapter.".to_string()
            },
            sips,
        ));
    } else if platform == "windows" {
        let pdfium_ok = crate::pdf_geometry::pdfium_library_path().is_some();
        checks.push(preflight_item(
            "renderer:windows-pdfium",
            pdfium_ok,
            if pdfium_ok { "info" } else { "warning" },
            if pdfium_ok {
                "Bundled PDFium page renderer is available; scanned PDFs use it for vision transcription input.".to_string()
            } else {
                "PDFium native library is not bundled; scanned PDFs require cloud PDF vision or manual transcription. Text-layer PDF parsing still works via the pdf-extract fallback.".to_string()
            },
            json!({
                "selectedRenderer": renderer,
                "rendererProvider": "windows-pdfium",
                "implemented": true,
                "libraryAvailable": pdfium_ok
            }),
        ));
    }

    let local_ocr = local_ocr_enabled();
    checks.push(preflight_item(
        "ocr:local",
        !local_ocr,
        if local_ocr { "warning" } else { "info" },
        if local_ocr {
            "EPIC8_ENABLE_LOCAL_OCR is enabled, but no local OCR engine is bundled in the default runtime.".to_string()
        } else {
            "Local OCR is disabled by default and not bundled; scanned PDFs use page rendering plus cloud vision or manual transcription.".to_string()
        },
        json!({
            "env": env::var("EPIC8_ENABLE_LOCAL_OCR").ok(),
            "enabled": local_ocr,
            "bundled": false
        }),
    ));

    let cloud_vision = cloud_pdf_vision_enabled();
    let live_base_url = env::var("EPIC8_LIVE_LLM_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let live_model = env::var("EPIC8_LIVE_LLM_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let live_api_key = env::var("EPIC8_LIVE_LLM_API_KEY")
        .ok()
        .filter(|value| !value.trim().is_empty());
    checks.push(preflight_item(
        "vision:cloud",
        !cloud_vision || (live_base_url.is_some() && live_model.is_some()),
        if cloud_vision { "warning" } else { "info" },
        if cloud_vision {
            "Cloud PDF vision is enabled; configure an LLM profile or EPIC8_LIVE_LLM_* environment values that support vision/PDF input.".to_string()
        } else {
            "Cloud PDF vision is disabled by default. Enable EPIC8_ENABLE_CLOUD_PDF_VISION only when a configured profile supports image or PDF input.".to_string()
        },
        json!({
            "env": env::var("EPIC8_ENABLE_CLOUD_PDF_VISION").ok(),
            "enabled": cloud_vision,
            "liveEnv": {
                "baseUrl": live_base_url.is_some(),
                "model": live_model.is_some(),
                "apiKey": live_api_key.is_some()
            },
            "profileCapabilityFields": [
                "supportsVisionImages",
                "supportsPdfFileInput",
                "maxPdfBytes",
                "maxVisionImages"
            ]
        }),
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
