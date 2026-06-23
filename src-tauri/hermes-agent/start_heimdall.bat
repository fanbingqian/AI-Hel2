@echo off
REM HEIMDALL Web Console — startup script (Windows)
cd /d "%~dp0"
echo Starting HEIMDALL Web Console...
echo Open http://localhost:8765 in browser
echo.
wsl.exe -d Ubuntu -- bash -c "cd /d/hermes-upone/hermes-agent-main && python3 -m uvicorn heimdall.web.api:app --host 0.0.0.0 --port 8765"
pause
