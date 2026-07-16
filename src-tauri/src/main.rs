// Build the release binary as a Windows GUI subsystem executable so launching
// the app does NOT pop up a trailing console window. Debug builds keep the
// console subsystem so `println!`/`eprintln!` output is visible during
// development. CLI mode (`--generate-reading-source` etc.) re-attaches to the
// parent console at runtime in `run_cli_or_app`, so headless CLI output still
// works in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ielts_author_studio_lib::run_cli_or_app();
}
