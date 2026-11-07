<#
  Publica una versión de ggswp en GitHub Releases. La app instalada busca ahí las
  actualizaciones (latest.json).

  Uso:  powershell -File scripts/release.ps1 [-Notes "Qué cambió"] [-Draft]

  Antes: sube la versión en src-tauri/tauri.conf.json (y package.json / Cargo.toml) y haz
  commit. La clave para firmar está en %USERPROFILE%\.tauri\ggswp.key y nunca va al repo.
#>
param(
  [string]$Notes = "",
  [switch]$Draft
)
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

$conf = Get-Content "src-tauri/tauri.conf.json" -Raw | ConvertFrom-Json
$version = $conf.version
$tag = "v$version"
$repo = "randygil/ggswp"
# Siempre con la cuenta dueña del repositorio, aunque gh tenga otra cuenta activa.
$env:GH_TOKEN = (gh auth token --hostname github.com --user randygil)
if (-not $env:GH_TOKEN) { throw "gh no tiene la cuenta randygil (gh auth login)" }

$keyPath = Join-Path $env:USERPROFILE ".tauri\ggswp.key"
if (-not (Test-Path $keyPath)) { throw "No está la clave de firma en $keyPath" }
if (-not (Test-Path "src-tauri/bin/ffmpeg.exe")) { throw "Falta src-tauri/bin/ffmpeg.exe (se incluye en el instalador)" }
if (git status --porcelain) { throw "Hay cambios sin commit: haz commit antes de publicar" }
# En PowerShell 5.1, la salida de error de un programa con "Stop" corta el script: se mira el código.
$ErrorActionPreference = "Continue"
gh release view $tag --repo $repo *> $null
$exists = ($LASTEXITCODE -eq 0)
$ErrorActionPreference = "Stop"
if ($exists) { throw "La versión $tag ya está publicada: sube la versión primero" }

$env:TAURI_SIGNING_PRIVATE_KEY = Get-Content $keyPath -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ""
$ErrorActionPreference = "Continue"
npx tauri build
$ErrorActionPreference = "Stop"
if ($LASTEXITCODE -ne 0) { throw "Falló la compilación" }

$nsis = "src-tauri/target/release/bundle/nsis"
$exeName = "ggswp_${version}_x64-setup.exe"
$exe = Join-Path $nsis $exeName
$sig = "$exe.sig"
if (-not (Test-Path $sig)) { throw "No se generó la firma $sig" }

if (-not $Notes) { $Notes = "ggswp $version" }
$latest = [ordered]@{
  version   = $version
  notes     = $Notes
  pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
  platforms = [ordered]@{
    "windows-x86_64" = [ordered]@{
      signature = (Get-Content $sig -Raw).Trim()
      url       = "https://github.com/$repo/releases/download/$tag/$exeName"
    }
  }
}
$latestPath = Join-Path $nsis "latest.json"
# UTF-8 sin BOM: el actualizador no acepta el BOM que agrega Set-Content en PowerShell 5.1.
[System.IO.File]::WriteAllText((Join-Path $root $latestPath), ($latest | ConvertTo-Json -Depth 5), (New-Object System.Text.UTF8Encoding $false))

$ErrorActionPreference = "Continue"
git push origin HEAD
git tag $tag
git push origin $tag
$ErrorActionPreference = "Stop"
$ghArgs = @("release", "create", $tag, $exe, $sig, $latestPath, "--repo", $repo, "--title", "ggswp $version", "--notes", $Notes)
if ($Draft) { $ghArgs += "--draft" }
gh @ghArgs
if ($LASTEXITCODE -ne 0) { throw "No se pudo crear la release" }
Write-Host "Publicada $tag"
