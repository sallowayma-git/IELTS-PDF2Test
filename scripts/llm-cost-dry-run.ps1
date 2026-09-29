param(
  [Parameter(Mandatory = $true)]
  [string]$JobDirectory,
  [string]$OutputDirectory,
  [string]$Model,
  [string]$BaseUrl
)

$ErrorActionPreference = 'Stop'
$jobPath = (Resolve-Path -LiteralPath $JobDirectory).Path
$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $OutputDirectory) {
  $runId = (Get-Date).ToUniversalTime().ToString('yyyyMMddTHHmmssfffZ')
  $OutputDirectory = Join-Path $jobPath (Join-Path 'llm-dry-run' (Join-Path $runId 'requests'))
}
$outputPath = [System.IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $outputPath) {
  throw "Dry-run output directory already exists: $outputPath"
}
if ([bool]$Model -ne [bool]$BaseUrl) {
  throw 'Model and BaseUrl must be provided together.'
}
New-Item -ItemType Directory -Path $outputPath | Out-Null

$previousJobs = $env:CARGO_BUILD_JOBS
$previousJobDir = $env:PDF2TEST_LLM_DRY_RUN_JOB_DIR
$previousOutputDir = $env:PDF2TEST_LLM_DRY_RUN_OUTPUT_DIR
$previousModel = $env:PDF2TEST_LLM_DRY_RUN_MODEL
$previousBaseUrl = $env:PDF2TEST_LLM_DRY_RUN_BASE_URL
$env:CARGO_BUILD_JOBS = '1'
$env:PDF2TEST_LLM_DRY_RUN_JOB_DIR = $jobPath
$env:PDF2TEST_LLM_DRY_RUN_OUTPUT_DIR = $outputPath
if ($Model) { $env:PDF2TEST_LLM_DRY_RUN_MODEL = $Model }
if ($BaseUrl) { $env:PDF2TEST_LLM_DRY_RUN_BASE_URL = $BaseUrl }
Push-Location $repoRoot
try {
  cargo test --manifest-path src-tauri/Cargo.toml --lib llm_gateway::tests::dry_run_cached_job_requests -- --ignored --nocapture
  if ($LASTEXITCODE -ne 0) {
    throw "Dry-run request assembly failed with exit code $LASTEXITCODE"
  }
}
finally {
  Pop-Location
  $env:CARGO_BUILD_JOBS = $previousJobs
  $env:PDF2TEST_LLM_DRY_RUN_JOB_DIR = $previousJobDir
  $env:PDF2TEST_LLM_DRY_RUN_OUTPUT_DIR = $previousOutputDir
  $env:PDF2TEST_LLM_DRY_RUN_MODEL = $previousModel
  $env:PDF2TEST_LLM_DRY_RUN_BASE_URL = $previousBaseUrl
}

Write-Output "Serialized request bodies: $outputPath"
