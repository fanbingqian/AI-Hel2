# AI-Hel2 打包后验证脚本
# 每次 tauri build 后运行此脚本，检查关键链路是否完整
param([switch]$SkipUnit)

$ErrorActionPreference = "Continue"
$PASS = 0; $FAIL = 0; $WARN = 0

function Check($label, $script) {
    Write-Host -NoNewline "  [$label] ... "
    try {
        $result = & $script
        if ($result -eq $true -or $result -eq 0) {
            Write-Host "PASS" -ForegroundColor Green
            $script:PASS++
        } elseif ($result -eq $null -or $result -eq "") {
            Write-Host "WARN" -ForegroundColor Yellow
            $script:WARN++
        } else {
            Write-Host "FAIL: $result" -ForegroundColor Red
            $script:FAIL++
        }
    } catch {
        Write-Host "FAIL: $_" -ForegroundColor Red
        $script:FAIL++
    }
}

Write-Host "`n=== AI-Hel2 Build Verification ===`n" -ForegroundColor Cyan

# ── 1. Installer file ──
$RELEASE = "src-tauri\target\release"
$BUNDLE = "$RELEASE\bundle\nsis"
$EXE = "$BUNDLE\AI-Hel2_*.exe"

$exeFile = Get-ChildItem $EXE -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
Check "Installer exists" { $exeFile -ne $null }
if ($exeFile) {
    $sizeMB = [math]::Round($exeFile.Length / 1MB, 1)
    Check "Installer size > 80MB" { $exeFile.Length -gt 80MB }
    Check "Signature file exists" { Test-Path "$BUNDLE\AI-Hel2_*.exe.sig" }
}

# ── 2. ZIP bundle ──
$ZIP = "src-tauri\hermes-agent.zip"
Check "hermes-agent.zip exists" { Test-Path $ZIP }
if (Test-Path $ZIP) {
    $zipSize = [math]::Round((Get-Item $ZIP).Length / 1MB, 1)
    Check "ZIP size > 70MB" { (Get-Item $ZIP).Length -gt 70MB }

    # Verify key files inside ZIP
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [System.IO.Compression.ZipFile]::OpenRead((Resolve-Path $ZIP))
    $entries = $zip.Entries | ForEach-Object { $_.FullName }

    Check "ZIP: python.exe" { ($entries | Where-Object { $_ -like "*python/python.exe" }) -ne $null }
    Check "ZIP: main.py" { ($entries | Where-Object { $_ -like "*hermes_cli/main.py" }) -ne $null }
    Check "ZIP: aiohttp" { ($entries | Where-Object { $_ -like "*aiohttp*" }).Count -gt 0 }
    Check "ZIP: openai" { ($entries | Where-Object { $_ -like "*site-packages/openai*" }).Count -gt 0 }
    Check "ZIP: no editable .pth" { ($entries | Where-Object { $_ -like "*__editable__*" }).Count -eq 0 }
    $zip.Dispose()
}

# ── 3. Config template ──
if (-not $SkipUnit) {
    Check "config: toolsets in ensure_api_server_config" {
        $content = Get-Content "src-tauri\src\services\agent_manager.rs" -Raw
        $content -match "toolsets:"
    }
    Check "config: model.default in ensure_api_server_config" {
        $content = Get-Content "src-tauri\src\services\agent_manager.rs" -Raw
        $content -match "model:.*default:"
    }
}

# ── 4. Port consistency ──
Check "Port: DEFAULT_PORT = 18642" {
    $content = Get-Content "src-tauri\src\services\agent_manager.rs" -Raw
    $content -match "DEFAULT_PORT.*18642"
}
Check "Port: connection_service = 18642" {
    $content = Get-Content "src-tauri\src\services\connection_service.rs" -Raw
    $content -match "agent_url.*18642"
}

# ── 5. Key Rust functions exist ──
if (-not $SkipUnit) {
    $funcs = @(
        "fn update_api_key", "fn verify_api_key", "fn copy_agent_config_for_nexus",
        "fn ensure_api_server_config", "fn ensure_dot_env", "fn extract_agent_zip",
        "fn python_path", "fn spawn_agent", "fn health_check", "fn start", "fn restart"
    )
    foreach ($f in $funcs) {
        Check "Rust: $f" {
            $content = Get-Content "src-tauri\src\services\agent_manager.rs", "src-tauri\src\commands\config.rs" -Raw
            $content -match [regex]::Escape($f)
        }
    }
}

# ── 6. Frontend critical paths ──
Check "Frontend: updateApiKey function" {
    (Get-Content "src\services\api.ts" -Raw) -match "updateApiKey"
}
Check "Frontend: verify_api_key invoke" {
    (Get-Content "src\components\settings\AgentSettings.tsx" -Raw) -match "verify_api_key"
}
Check "Frontend: ProviderModelRow exists" {
    (Get-Content "src\components\settings\AgentSettings.tsx" -Raw) -match "ProviderModelRow"
}
Check "Frontend: NexusProviderRow exists" {
    (Get-Content "src\components\settings\SettingsPage.tsx" -Raw) -match "NexusProviderRow"
}
Check "Frontend: physics.ts exists" {
    Test-Path "src\components\sphere\physics.ts"
}
Check "Frontend: no ForceGraph3D import" {
    $content = Get-Content "src\components\sphere\ForceGraph2DWrapper.tsx", "src\components\aihel\MainContent.tsx", "src\components\sphere\FloatingMenu.tsx" -Raw
    $content -notmatch "ForceGraph3D"
}

# ── 7. Permissions / Capabilities ──
Check "Capabilities: set_max_size allowed" {
    (Get-Content "src-tauri\capabilities\default.json" -Raw) -match "set-max-size"
}
Check "Capabilities: pill window permissions" {
    $caps = Get-Content "src-tauri\capabilities\default.json" -Raw
    $caps -match '"windows":\s*\[.*"pill"' -or $caps -match '"windows":\s*\[.*"\*"'
}
Check "Capabilities: updater allowed" {
    (Get-Content "src-tauri\capabilities\default.json" -Raw) -match "updater"
}

# ── 8. .gitignore protection ──
Check ".gitignore: hermes-agent excluded" {
    (Get-Content ".gitignore" -Raw) -match "hermes-agent"
}
Check ".gitignore: updater-key excluded" {
    (Get-Content ".gitignore" -Raw) -match "updater-key"
}
Check ".gitignore: users.json excluded" {
    (Get-Content ".gitignore" -Raw) -match "users.json"
}

# ── Summary ──
Write-Host "`n=== Result: $PASS passed, $WARN warnings, $FAIL failed ===`n" -ForegroundColor $(if ($FAIL -eq 0) { "Green" } else { "Red" })
exit $FAIL
