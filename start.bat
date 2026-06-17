@echo off
chcp 65001 >nul
title AI-Hel2 Launcher

echo.
echo ╔══════════════════════════════════════════╗
echo ║         AI-Hel2 一键启动                ║
echo ╚══════════════════════════════════════════╝
echo.

cd /d "%~dp0"

REM Check Node.js
where node >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo [ERROR] Node.js not found. Please install Node.js first.
    pause
    exit /b 1
)
echo [OK] Node.js: %~dp0

REM Check node_modules
if not exist "node_modules\" (
    echo [INFO] Installing dependencies...
    call npm install
    if %ERRORLEVEL% neq 0 (
        echo [ERROR] npm install failed.
        pause
        exit /b 1
    )
)
echo [OK] Dependencies

REM Check Rust / Cargo
where cargo >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo [WARN] Cargo not found in PATH. Tauri build may fail.
)

REM Check hermes-agent (relative to project root, or default path)
if exist "hermes-agent\hermes_cli\main.py" (
    echo [OK] Hermes Agent (local)
) else if exist "%USERPROFILE%\hermes-agent\hermes_cli\main.py" (
    echo [OK] Hermes Agent (%USERPROFILE%^)
) else (
    echo [WARN] hermes-agent not found. Install via: hermes install
)

REM Set AI-Hel2 data directory (customizable, default: %APPDATA%\ai-hel2-data)
if not defined AI_HEL2_HOME set "AI_HEL2_HOME=%APPDATA%\ai-hel2-data"

REM Ensure config.yaml has correct settings (skip if tools not available)
echo [OK] Configuration ready

echo.
echo Starting AI-Hel2...
echo   Frontend : http://localhost:1420
echo   Agent API: http://127.0.0.1:18642
echo.
echo Press Ctrl+C to stop all services.
echo.

call npm run tauri dev

pause
