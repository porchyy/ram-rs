$ErrorActionPreference = 'Stop'
$cargo = Get-Command cargo -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source
if (-not $cargo) { $cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
if (-not (Test-Path -LiteralPath $cargo)) { throw 'Install Rust and Windows build tools first. See README.md.' }
# GNU dlltool cannot handle a target directory containing Thai characters.
$buildDir = Join-Path $env:TEMP 'ram-observer-build'
Push-Location $PSScriptRoot
try {
    & $cargo build --release --locked --target-dir $buildDir
    if ($LASTEXITCODE -ne 0) { throw 'Cargo build failed.' }
    $distDir = Join-Path $PSScriptRoot 'dist'
    New-Item -ItemType Directory -Path $distDir -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $buildDir 'release\ram-observer.exe') -Destination $distDir
    Write-Host "Built: $distDir\ram-observer.exe"
} finally { Pop-Location }
