# Ensure config.yaml has correct Hermes Agent settings
$configPath = "$env:USERPROFILE\.ai-hel2\config.yaml"
if (-not (Test-Path $configPath)) {
    Write-Host "[WARN] config.yaml not found at $configPath"
    exit 0
}

$c = Get-Content $configPath -Raw -Encoding UTF8
$changed = $false

# Fix model section
if ($c -match 'provider:\s*hermes-builtin') {
    $c = $c -replace 'provider:\s*hermes-builtin', 'provider: deepseek'
    $changed = $true
    Write-Host "[FIX] model.provider -> deepseek"
}
if ($c -notmatch 'default:\s*deepseek-v4-flash') {
    $c = $c -replace 'default:\s*\S+', 'default: deepseek-v4-flash'
    $changed = $true
    Write-Host "[FIX] model.default -> deepseek-v4-flash"
}
if ($c -notmatch '^\s*name:\s*deepseek-v4-flash') {
    $c = $c -replace '(?m)^\s*name:\s*\S+', '  name: deepseek-v4-flash'
    $changed = $true
    Write-Host "[FIX] model.name -> deepseek-v4-flash"
}

# Ensure platforms section with API key
if ($c -notmatch 'key:\s*"aihel2-local-dev"') {
    if ($c -match 'platforms:') {
        if ($c -notmatch 'model_name:') {
            $c = $c -replace '(key:\s*"\S+")', "`$1`n      model_name: `"deepseek-v4-flash`""
        }
        if ($c -notmatch 'key:\s*"') {
            $c = $c -replace '(host:\s*"127\.0\.0\.1")', "`$1`n      key: `"aihel2-local-dev`""
        }
    } else {
        # Add entire platforms section
        $c += @"

platforms:
  api_server:
    enabled: true
    extra:
      port: 18642
      host: "127.0.0.1"
      key: "aihel2-local-dev"
      model_name: "deepseek-v4-flash"
"@
    }
    $changed = $true
    Write-Host "[FIX] platforms.api_server section added"
}

if ($changed) {
    [IO.File]::WriteAllText($configPath, $c, [Text.Encoding]::UTF8)
    Write-Host "[OK] config.yaml repaired"
} else {
    Write-Host "[OK] config.yaml is correct"
}
