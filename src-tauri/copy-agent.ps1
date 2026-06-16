# Copy hermes-agent to NSIS staging directory before packaging.
# NSIS makensis runs from target/release/bundle/nsis/ — its File command
# resolves relative paths from there.  Without this copy, the installer
# silently omits hermes-agent and the agent won't start.
$src = "$PSScriptRoot\hermes-agent"
$dstNsis = "$PSScriptRoot\target\release\bundle\nsis\hermes-agent"
$dstRelease = "$PSScriptRoot\target\release\hermes-agent"

$targets = @()
if (Test-Path "$PSScriptRoot\target\release\bundle\nsis") { $targets += $dstNsis }
if (Test-Path "$PSScriptRoot\target\release")              { $targets += $dstRelease }

if ($targets.Count -eq 0) {
    Write-Warning "No bundle output directories found — skipping hermes-agent copy"
    exit 0
}

if (Test-Path $src) {
    foreach ($dst in $targets) {
        robocopy $src $dst /E /XD __pycache__ .git /XF *.pyc .gitignore /NFL /NDL /NJH /NJS
        Write-Output "Copied hermes-agent to $dst"
    }
} else {
    Write-Warning "hermes-agent source not found at $src"
}

# Also copy hermes-agent.zip to the NSIS/release root (parent of $dst)
$zipSrc = "$PSScriptRoot\hermes-agent.zip"
if (Test-Path $zipSrc) {
    foreach ($dst in $targets) {
        $parent = Split-Path $dst -Parent
        Copy-Item $zipSrc $parent -Force
        Write-Output "Copied hermes-agent.zip to $parent"
    }
} else {
    Write-Warning "hermes-agent.zip not found at $zipSrc"
}
