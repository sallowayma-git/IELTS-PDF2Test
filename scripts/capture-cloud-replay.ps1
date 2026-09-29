<#
实时抓取真实模型导入产物，避开任务结束时对 cache/llm 与 llm-calls.jsonl 的清理。
轮询 jobs 目录，把匹配的中间产物镜像到回放夹具目录（gitignore，不提交）。
不复制 config（密钥所在），不复制 assets 二进制。抓取后须先扫描 sk-/Authorization 再使用。
#>
[CmdletBinding()]
param(
  [string]$JobsDir = (Join-Path $env:APPDATA 'com.ielts.author.studio\jobs'),
  [string]$Dest    = (Join-Path $PSScriptRoot '..\fixtures\golden\private-cloud-replay'),
  [double]$IntervalSec = 0.5,
  [int]$RunMinutes = 30
)

# 需要抓取的 job 内相对路径规则（正斜杠、小写匹配）。刻意宽松以免漏采。
$includeRx = @(
  'cache/llm/.*\.(json|jsonl)$',
  '(^|/)llm-calls\.jsonl$',
  '(^|/)recognition/.*\.json$',
  '(^|/)decision\.json$',
  '.*\.shadow\.json$',                # document-ir-v2 / authoring-ir-v2 等
  '(^|/)question-layout-graph\.json$',
  '(^|/)cloud[-/].*\.json$',          # cloud-authoring-candidate 等
  '.*authoring[-_]candidate.*\.json$',
  '(^|/)job\.json$'                    # 卷面元数据，便于回放识别是哪份卷
)
# 明确排除：assets 二进制、config 密钥（config 不在 jobs 下，双保险）。
$excludeRx = @('(^|/)assets/', '(^|/)config/')

function Test-Match($rel) {
  foreach ($x in $excludeRx) { if ($rel -match $x) { return $false } }
  foreach ($i in $includeRx) { if ($rel -match $i) { return $true } }
  return $false
}

New-Item -ItemType Directory -Force -Path $Dest | Out-Null
$manifest = Join-Path $Dest ('capture-manifest-{0}.log' -f (Get-Date -Format 'yyyyMMdd-HHmmss'))
$seen = @{}   # 源全路径 -> "size|ticks"，变化则重抓（jsonl 追加、shadow 覆盖）
$deadline = (Get-Date).AddMinutes($RunMinutes)

Write-Host "[capture] 监视 $JobsDir"
Write-Host "[capture] 输出 $Dest"
Write-Host "[capture] 每 $IntervalSec 秒轮询，运行 $RunMinutes 分钟。Ctrl+C 结束。"
Add-Content -LiteralPath $manifest -Value ("start $(Get-Date -Format o)  jobs=$JobsDir")

while ((Get-Date) -lt $deadline) {
  if (Test-Path $JobsDir) {
    Get-ChildItem -LiteralPath $JobsDir -Recurse -File -ErrorAction SilentlyContinue | ForEach-Object {
      $full = $_.FullName
      $rel  = $full.Substring($JobsDir.Length).TrimStart('\','/').Replace('\','/')
      if (-not (Test-Match $rel)) { return }
      $sig = "{0}|{1}" -f $_.Length, $_.LastWriteTimeUtc.Ticks
      if ($seen[$full] -eq $sig) { return }
      $target = Join-Path $Dest $rel
      $tdir = Split-Path $target -Parent
      if (-not (Test-Path $tdir)) { New-Item -ItemType Directory -Force -Path $tdir | Out-Null }
      try {
        Copy-Item -LiteralPath $full -Destination $target -Force -ErrorAction Stop
        $seen[$full] = $sig
        $line = "copied  $rel  ($($_.Length) bytes)"
        Write-Host "[capture] $line"
        Add-Content -LiteralPath $manifest -Value ("$(Get-Date -Format o)  $line")
      } catch {
        # 文件可能正被写入或已被清理；下一轮再试
      }
    }
  }
  Start-Sleep -Seconds $IntervalSec
}
Write-Host "[capture] 结束。清单：$manifest"
Add-Content -LiteralPath $manifest -Value ("end $(Get-Date -Format o)")
