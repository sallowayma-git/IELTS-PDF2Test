param(
  [string]$BundleRoot = "src-tauri/target/release/bundle",
  [switch]$RequireValid
)

$ErrorActionPreference = "Stop"

function Resolve-RepoPath {
  param([string]$PathValue)
  if ([System.IO.Path]::IsPathRooted($PathValue)) {
    return $PathValue
  }
  return Join-Path (Get-Location) $PathValue
}

$bundlePath = Resolve-RepoPath $BundleRoot
$artifactPatterns = @(
  (Join-Path $bundlePath "nsis/*.exe"),
  (Join-Path $bundlePath "msi/*.msi")
)

$artifacts = @()
foreach ($pattern in $artifactPatterns) {
  $artifacts += Get-ChildItem -Path $pattern -File -ErrorAction SilentlyContinue
}

if ($artifacts.Count -eq 0) {
  Write-Error "No Windows installer artifacts found under $bundlePath"
}

$results = @()
$failed = $false
foreach ($artifact in $artifacts) {
  $signature = Get-AuthenticodeSignature -FilePath $artifact.FullName
  $isValid = $signature.Status -eq "Valid"
  if ($RequireValid -and -not $isValid) {
    $failed = $true
  }
  $results += [ordered]@{
    path = $artifact.FullName
    sizeBytes = $artifact.Length
    status = [string]$signature.Status
    statusMessage = $signature.StatusMessage
    signerSubject = if ($signature.SignerCertificate) { $signature.SignerCertificate.Subject } else { $null }
    timestampCertificateSubject = if ($signature.TimeStamperCertificate) { $signature.TimeStamperCertificate.Subject } else { $null }
    valid = $isValid
  }
}

$report = [ordered]@{
  schemaVersion = "Epic8WindowsSignatureAuditV1"
  generatedAt = (Get-Date).ToUniversalTime().ToString("o")
  requireValid = [bool]$RequireValid
  artifactCount = $artifacts.Count
  artifacts = $results
}

$report | ConvertTo-Json -Depth 6

if ($failed) {
  exit 1
}
