//! Shared helpers for tests that depend on the private, intentionally-uncommitted
//! regression corpus.
//!
//! `fixtures/golden/private-real/README.md` states the PDFs there are git-ignored because they
//! are private/copyrighted regression inputs. Tests that hard-failed on their absence made
//! `cargo test` permanently red on every clean checkout and in CI, which hides real regressions
//! in the noise instead of catching them. These helpers turn "corpus absent" into a visible skip
//! by default, and keep it a hard failure on machines that do mount the corpus.

use std::path::{Path, PathBuf};

/// Env var that turns a missing private fixture back into a hard failure.
pub(crate) const REQUIRE_PRIVATE_CORPUS_ENV: &str = "EPIC8_REQUIRE_PRIVATE_CORPUS";

/// Resolve a workspace-relative path (the crate lives in `src-tauri/`).
pub(crate) fn workspace_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(relative)
}

fn require_private_corpus() -> bool {
    std::env::var(REQUIRE_PRIVATE_CORPUS_ENV).is_ok_and(|value| value == "1")
}

/// True when every path exists. When some are missing this either panics (strict mode) or
/// prints a skip notice and returns false, so the caller can return early.
pub(crate) fn private_corpus_ready(test_name: &str, paths: &[PathBuf]) -> bool {
    let missing = paths
        .iter()
        .filter(|path| !path.exists())
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return true;
    }
    if require_private_corpus() {
        panic!(
            "{test_name}: {} required private fixture(s) missing: {}",
            missing.len(),
            missing.join(", ")
        );
    }
    eprintln!(
        "SKIP {test_name}: private regression corpus not present ({} missing, first: {}). Set {}=1 to make this a hard failure.",
        missing.len(),
        missing.first().map(String::as_str).unwrap_or("<none>"),
        REQUIRE_PRIVATE_CORPUS_ENV
    );
    false
}

/// Convenience form for a single workspace-relative fixture.
pub(crate) fn private_fixture_ready(test_name: &str, relative: &str) -> bool {
    private_corpus_ready(test_name, &[workspace_path(relative)])
}

/// True when the Python parser sidecar has the optional `pypdf` dependency available.
/// Image extraction tests skip without it rather than reporting a product failure.
pub(crate) fn python_pypdf_available() -> bool {
    for (command, args) in [("python", vec![]), ("python3", vec![]), ("py", vec!["-3"])] {
        let mut invocation = std::process::Command::new(command);
        invocation.args(args);
        invocation.args(["-c", "import pypdf"]);
        if let Ok(status) = invocation.status() {
            if status.success() {
                return true;
            }
        }
    }
    eprintln!("SKIP: python sidecar dependency pypdf is unavailable in this environment.");
    false
}

/// The authoritative eight-PDF private corpus selected by
/// `fixtures/golden/manifest.json#requiredPrivateCorpus`.
pub(crate) fn golden_private_corpus_paths() -> Vec<PathBuf> {
    let manifest_path = workspace_path("fixtures/golden/manifest.json");
    let Ok(bytes) = std::fs::read(&manifest_path) else {
        return Vec::new();
    };
    let Ok(manifest) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Vec::new();
    };
    manifest
        .get("requiredPrivateCorpus")
        .and_then(serde_json::Value::as_array)
        .map(|fixtures| {
            fixtures
                .iter()
                .filter_map(|fixture| {
                    fixture
                        .get("sourcePath")
                        .and_then(serde_json::Value::as_str)
                })
                .map(workspace_path)
                .collect()
        })
        .unwrap_or_default()
}

/// Guard for the whole eight-PDF corpus. Returns false (after printing a skip notice) when any
/// selected fixture is absent, unless strict mode is requested.
pub(crate) fn golden_private_corpus_ready(test_name: &str) -> bool {
    let paths = golden_private_corpus_paths();
    if paths.is_empty() {
        if require_private_corpus() {
            panic!("{test_name}: golden manifest declares no requiredPrivateCorpus");
        }
        eprintln!("SKIP {test_name}: golden manifest declares no requiredPrivateCorpus.");
        return false;
    }
    private_corpus_ready(test_name, &paths)
}
