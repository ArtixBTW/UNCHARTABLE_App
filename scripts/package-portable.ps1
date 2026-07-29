param(
  [string]$Version
)

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$packageJson = Get-Content -LiteralPath (Join-Path $root "package.json") -Raw | ConvertFrom-Json
if ([string]::IsNullOrWhiteSpace($Version)) {
  $Version = [string]$packageJson.version
}
$source = Join-Path $root "src-tauri\target\release\unchartable-app.exe"
$output = Join-Path $root "output\release"
$executableName = "UNCHARTABLE-v$Version.exe"
$executable = Join-Path $output $executableName
$checksum = "$executable.sha256"

if (-not (Test-Path -LiteralPath $source)) {
  throw "Portable executable not found. Run npm run build:portable first."
}

New-Item -ItemType Directory -Path $output -Force | Out-Null

if (Test-Path -LiteralPath $executable) {
  Remove-Item -LiteralPath $executable -Force
}

Copy-Item -LiteralPath $source -Destination $executable
$hash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $executableName" | Set-Content -LiteralPath $checksum -Encoding ascii

Write-Output $executable
Write-Output $checksum
