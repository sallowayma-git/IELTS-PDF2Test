use super::model::{DocxDocumentModel, DocxIssue};
use serde_json::{json, Value};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(crate) const DOCX_RENDER_PROVIDER_UNAVAILABLE: &str = "DOCX_RENDER_PROVIDER_UNAVAILABLE";
pub(crate) const DOCX_RENDER_FAILED: &str = "DOCX_RENDER_FAILED";

#[derive(Debug, Clone, Default)]
pub(crate) struct DocxRenderAssistStatus {
    pub(crate) requested: bool,
    pub(crate) provider: Option<String>,
    pub(crate) available: bool,
    pub(crate) reason: Option<String>,
}

#[derive(Debug)]
pub(crate) struct DocxRenderAssistResult {
    status: DocxRenderAssistStatus,
    rendered_pdf: Option<PathBuf>,
    temporary_dir: Option<PathBuf>,
}

impl DocxRenderAssistResult {
    pub(crate) fn from_rendered_pdf(
        provider: impl Into<String>,
        rendered_pdf: impl Into<PathBuf>,
    ) -> Result<Self, String> {
        let rendered_pdf = rendered_pdf.into();
        if !has_pdf_magic(&rendered_pdf) {
            return Err(format!(
                "render provider output is not a PDF: {}",
                rendered_pdf.display()
            ));
        }
        Ok(Self {
            status: DocxRenderAssistStatus {
                requested: true,
                provider: Some(provider.into()),
                available: true,
                reason: None,
            },
            rendered_pdf: Some(rendered_pdf),
            temporary_dir: None,
        })
    }

    pub(crate) fn rendered_pdf(&self) -> Option<&Path> {
        self.rendered_pdf.as_deref()
    }

    pub(crate) fn metadata(&self) -> Value {
        json!({
            "mode": if self.status.requested { "render-assisted" } else { "semantic-only" },
            "requested": self.status.requested,
            "provider": self.status.provider,
            "available": self.status.available,
            "geometryAuthority": if self.rendered_pdf.is_some() { "render-assisted" } else { "ooxml-semantic-only" },
            "reason": self.status.reason
        })
    }

    pub(crate) fn mark_geometry_failure(&mut self, reason: impl Into<String>) {
        self.rendered_pdf = None;
        self.status.available = false;
        self.status.reason = Some(reason.into());
    }

    pub(crate) fn record_issue(&self, model: &mut DocxDocumentModel) {
        if !self.status.requested || self.rendered_pdf.is_some() {
            return;
        }
        let code = if self.status.provider.is_some() {
            DOCX_RENDER_FAILED
        } else {
            DOCX_RENDER_PROVIDER_UNAVAILABLE
        };
        model.issues.push(DocxIssue::warning(
            code,
            self.status
                .reason
                .clone()
                .unwrap_or_else(|| "DOCX render assist failed".to_string()),
            None,
        ));
    }
}

impl Drop for DocxRenderAssistResult {
    fn drop(&mut self) {
        if let Some(path) = self.temporary_dir.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

pub(crate) fn requested_from_environment() -> bool {
    std::env::var("EPIC8_DOCX_RENDER_ASSIST")
        .ok()
        .is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
}

pub(crate) fn inspect_render_provider(requested: bool) -> DocxRenderAssistStatus {
    let candidates = render_candidates();
    inspect_render_provider_from_candidates(requested, &candidates)
}

fn inspect_render_provider_from_candidates(
    requested: bool,
    candidates: &[String],
) -> DocxRenderAssistStatus {
    if !requested {
        return DocxRenderAssistStatus {
            requested: false,
            reason: Some("semantic-only mode is the default".to_string()),
            ..DocxRenderAssistStatus::default()
        };
    }
    for candidate in candidates {
        let mut command = provider_command(candidate);
        command.arg("--headless").arg("--version");
        if run_with_timeout(&mut command, Duration::from_secs(15)).is_ok() {
            return DocxRenderAssistStatus {
                requested: true,
                provider: Some(candidate.clone()),
                available: true,
                reason: None,
            };
        }
    }
    DocxRenderAssistStatus {
        requested: true,
        provider: None,
        available: false,
        reason: Some("no configured LibreOffice-compatible renderer was found".to_string()),
    }
}

pub(crate) fn render_docx(input_path: &Path, requested: bool) -> DocxRenderAssistResult {
    let candidates = render_candidates();
    render_docx_with_candidates(input_path, requested, &candidates)
}

fn render_docx_with_candidates(
    input_path: &Path,
    requested: bool,
    candidates: &[String],
) -> DocxRenderAssistResult {
    let mut status = inspect_render_provider_from_candidates(requested, candidates);
    if !status.requested || !status.available {
        return DocxRenderAssistResult {
            status,
            rendered_pdf: None,
            temporary_dir: None,
        };
    }
    let Some(provider) = status.provider.clone() else {
        status.available = false;
        status.reason = Some("render provider selection was empty".to_string());
        return DocxRenderAssistResult {
            status,
            rendered_pdf: None,
            temporary_dir: None,
        };
    };
    let temporary_dir = std::env::temp_dir().join(format!(
        "epic8-docx-render-{}",
        uuid::Uuid::new_v4().simple()
    ));
    if let Err(error) = fs::create_dir_all(&temporary_dir) {
        status.available = false;
        status.reason = Some(format!("render temp directory failed: {error}"));
        return DocxRenderAssistResult {
            status,
            rendered_pdf: None,
            temporary_dir: None,
        };
    }

    let mut command = provider_command(&provider);
    command
        .arg("--headless")
        .arg("--convert-to")
        .arg("pdf")
        .arg("--outdir")
        .arg(&temporary_dir)
        .arg(input_path);
    let timeout = std::env::var("EPIC8_DOCX_RENDER_TIMEOUT_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(120)
        .clamp(5, 600);
    if let Err(error) = run_with_timeout(&mut command, Duration::from_secs(timeout)) {
        status.available = false;
        status.reason = Some(format!("DOCX to PDF conversion failed: {error}"));
        return DocxRenderAssistResult {
            status,
            rendered_pdf: None,
            temporary_dir: Some(temporary_dir),
        };
    }
    let expected = input_path
        .file_stem()
        .map(|stem| temporary_dir.join(stem).with_extension("pdf"));
    let rendered_pdf = expected.filter(|path| path.is_file()).or_else(|| {
        fs::read_dir(&temporary_dir)
            .ok()?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"))
            })
    });
    let Some(rendered_pdf) = rendered_pdf else {
        status.available = false;
        status.reason = Some("renderer exited successfully but produced no PDF".to_string());
        return DocxRenderAssistResult {
            status,
            rendered_pdf: None,
            temporary_dir: Some(temporary_dir),
        };
    };
    if !has_pdf_magic(&rendered_pdf) {
        status.available = false;
        status.reason = Some("renderer output is not a PDF".to_string());
        return DocxRenderAssistResult {
            status,
            rendered_pdf: None,
            temporary_dir: Some(temporary_dir),
        };
    }
    status.available = true;
    status.reason = None;
    DocxRenderAssistResult {
        status,
        rendered_pdf: Some(rendered_pdf),
        temporary_dir: Some(temporary_dir),
    }
}

fn render_candidates() -> Vec<String> {
    let mut candidates = Vec::new();
    if let Ok(configured) = std::env::var("EPIC8_DOCX_RENDER_PROVIDER") {
        if !configured.trim().is_empty() {
            candidates.push(configured);
        }
    }
    for candidate in ["soffice", "libreoffice"] {
        if !candidates.iter().any(|item| item == candidate) {
            candidates.push(candidate.to_string());
        }
    }
    candidates
}

fn provider_command(provider: &str) -> Command {
    if provider.to_ascii_lowercase().ends_with(".ps1") {
        let mut command = Command::new("powershell.exe");
        command
            .arg("-NoProfile")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-File")
            .arg(provider);
        command
    } else {
        Command::new(provider)
    }
}

fn run_with_timeout(command: &mut Command, timeout: Duration) -> Result<(), String> {
    command.stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let started = Instant::now();
    loop {
        match child.try_wait().map_err(|error| error.to_string())? {
            Some(status) if status.success() => return Ok(()),
            Some(status) => return Err(format!("provider exited with {status}")),
            None if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("provider timed out after {}s", timeout.as_secs()));
            }
            None => thread::sleep(Duration::from_millis(50)),
        }
    }
}

fn has_pdf_magic(path: &Path) -> bool {
    let mut magic = [0_u8; 5];
    fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && &magic == b"%PDF-"
}

#[cfg(test)]
mod tests {
    use super::{
        inspect_render_provider, render_docx_with_candidates, DOCX_RENDER_PROVIDER_UNAVAILABLE,
    };
    #[cfg(windows)]
    use std::{fs, path::PathBuf};
    #[cfg(windows)]
    use uuid::Uuid;

    #[test]
    fn semantic_only_mode_is_default_and_does_not_probe_external_tools() {
        let status = inspect_render_provider(false);
        assert!(!status.requested);
        assert!(!status.available);
        assert_eq!(
            DOCX_RENDER_PROVIDER_UNAVAILABLE,
            "DOCX_RENDER_PROVIDER_UNAVAILABLE"
        );
    }

    #[cfg(windows)]
    #[test]
    fn render_assist_only_trusts_a_real_pdf_output() {
        let temp = make_temp_dir();
        let input = temp.join("fixture.docx");
        fs::write(&input, b"fixture").unwrap();

        let success_provider = temp.join("success-provider.ps1");
        fs::write(
            &success_provider,
            r#"
if ($args -contains "--version") { exit 0 }
$outIndex = [Array]::IndexOf($args, "--outdir")
$outDir = $args[$outIndex + 1]
$inputPath = $args[$args.Count - 1]
$outputPath = Join-Path $outDir (([IO.Path]::GetFileNameWithoutExtension($inputPath)) + ".pdf")
[IO.File]::WriteAllBytes($outputPath, [Text.Encoding]::ASCII.GetBytes("%PDF-1.4"))
exit 0
"#,
        )
        .unwrap();
        let success = render_docx_with_candidates(
            &input,
            true,
            &[success_provider.to_string_lossy().into_owned()],
        );
        assert!(success.rendered_pdf().is_some());
        assert_eq!(success.metadata()["geometryAuthority"], "render-assisted");
        drop(success);

        let no_output_provider = temp.join("no-output-provider.ps1");
        fs::write(
            &no_output_provider,
            r#"
if ($args -contains "--version") { exit 0 }
exit 0
"#,
        )
        .unwrap();
        let no_output = render_docx_with_candidates(
            &input,
            true,
            &[no_output_provider.to_string_lossy().into_owned()],
        );
        assert!(no_output.rendered_pdf().is_none());
        assert_eq!(
            no_output.metadata()["geometryAuthority"],
            "ooxml-semantic-only"
        );
        assert!(no_output.metadata()["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("produced no PDF")));
        drop(no_output);

        let invalid_provider = temp.join("invalid-provider.ps1");
        fs::write(
            &invalid_provider,
            r#"
if ($args -contains "--version") { exit 0 }
$outIndex = [Array]::IndexOf($args, "--outdir")
$outDir = $args[$outIndex + 1]
$inputPath = $args[$args.Count - 1]
$outputPath = Join-Path $outDir (([IO.Path]::GetFileNameWithoutExtension($inputPath)) + ".pdf")
[IO.File]::WriteAllBytes($outputPath, [Text.Encoding]::ASCII.GetBytes("NOPE!"))
exit 0
"#,
        )
        .unwrap();
        let invalid = render_docx_with_candidates(
            &input,
            true,
            &[invalid_provider.to_string_lossy().into_owned()],
        );
        assert!(invalid.rendered_pdf().is_none());
        assert_eq!(
            invalid.metadata()["geometryAuthority"],
            "ooxml-semantic-only"
        );
        assert!(invalid.metadata()["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("not a PDF")));
        drop(invalid);

        let _ = fs::remove_dir_all(temp);
    }

    #[cfg(windows)]
    fn make_temp_dir() -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "phase3-docx-render-test-{}",
            Uuid::new_v4().simple()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }
}
