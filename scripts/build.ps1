# Windows Viewer build (release)
$ErrorActionPreference = "Stop"
Set-Location (Split-Path $PSScriptRoot -Parent)
cargo test --workspace
cargo build --release -p viewer-windows
Write-Host "Tayyor: target\release\viewer-windows.exe"
