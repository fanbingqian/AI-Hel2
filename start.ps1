# AI-Hel2 One-Click Startup Script
# Usage: Right-click → Run with PowerShell, or: powershell -ExecutionPolicy Bypass -File start.ps1

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $ScriptDir

Write-Host ""
Write-Host "╔══════════════════════════════════════════╗" -ForegroundColor Green
Write-Host "║         AI-Hel2 一键启动                ║" -ForegroundColor Green
Write-Host "╚══════════════════════════════════════════╝" -ForegroundColor Green
Write-Host ""

# ── Check Node.js ──
$nodeVersion = $null
try { $nodeVersion = node --version 2>$null } catch {}
if (-not $nodeVersion) {
    Write-Host "[ERROR] Node.js not found. Please install Node.js >= 18." -ForegroundColor Red
    Read-Host "Press Enter to exit"
    exit 1
}
Write-Host "[OK] Node.js $nodeVersion" -ForegroundColor Gray

# ── Check npm dependencies ──
if (-not (Test-Path "node_modules")) {
    Write-Host "[INFO] Installing dependencies (npm install)..." -ForegroundColor Yellow
    npm install
    if ($LASTEXITCODE -ne 0) {
        Write-Host "[ERROR] npm install failed." -ForegroundColor Red
        Read-Host "Press Enter to exit"
        exit 1
    }
}
Write-Host "[OK] Dependencies" -ForegroundColor Gray

# ── Check Rust / Cargo ──
$cargoVersion = $null
try { $cargoVersion = cargo --version 2>$null } catch {}
if ($cargoVersion) {
    Write-Host "[OK] $cargoVersion" -ForegroundColor Gray
} else {
    Write-Host "[WARN] Cargo not found. Tauri may fail to compile." -ForegroundColor Yellow
}

# ── Check hermes-agent ──
if (Test-Path "hermes-agent\hermes_cli\main.py") {
    Write-Host "[OK] Hermes Agent (hermes-agent/hermes_cli/main.py)" -ForegroundColor Gray
} else {
    Write-Host "[WARN] hermes-agent not found at expected path." -ForegroundColor Yellow
}

# ── Check config directory ──
$HermesHome = "$env:USERPROFILE\.hermes"
if (-not (Test-Path $HermesHome)) {
    Write-Host "[INFO] Creating config directory: $HermesHome" -ForegroundColor Yellow
    New-Item -ItemType Directory -Force -Path $HermesHome | Out-Null
}
Write-Host "[OK] Config: $HermesHome" -ForegroundColor Gray

Write-Host ""
Write-Host "Starting AI-Hel2..." -ForegroundColor Cyan
Write-Host "  Frontend : http://localhost:1420" -ForegroundColor Gray
Write-Host "  Agent API: http://127.0.0.1:18642" -ForegroundColor Gray
Write-Host ""
Write-Host "Press Ctrl+C to stop all services." -ForegroundColor DarkYellow
Write-Host ""

# ── Start Tauri dev ──
npm run tauri dev
if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "[ERROR] Tauri exited with code $LASTEXITCODE" -ForegroundColor Red
    Read-Host "Press Enter to exit"
    exit $LASTEXITCODE
}
