# Copy hermes-agent to target/release before NSIS packaging
$src = "$PSScriptRoot\hermes-agent"
$dst = "$PSScriptRoot\target\release\hermes-agent"
if (Test-Path $src) {
    robocopy $src $dst /E /XD __pycache__ .git /XF *.pyc .gitignore /NFL /NDL /NJH /NJS
    Write-Output "Copied hermes-agent to bundle output"
} else {
    Write-Warning "hermes-agent source not found at $src"
}
