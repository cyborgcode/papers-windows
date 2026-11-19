# Write latest.json for the updater: version, download URLs, sizes and SHA-256 of the
# installer and the portable zip in OutDir. Papers reads it from
# https://github.com/<repo>/releases/latest/download/latest.json
param(
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$OutDir,
    [Parameter(Mandatory = $true)][string]$Repository
)
$ErrorActionPreference = 'Stop'
$base = "https://github.com/$Repository/releases/download/v$Version"

function Asset([string]$Name) {
    $path = Join-Path $OutDir $Name
    if (-not (Test-Path $path)) { throw "missing $path" }
    [ordered]@{
        name   = $Name
        url    = "$base/$Name"
        size   = (Get-Item $path).Length
        sha256 = (Get-FileHash -Algorithm SHA256 $path).Hash.ToLowerInvariant()
    }
}

$manifest = [ordered]@{
    version   = $Version
    published = (Get-Date).ToUniversalTime().ToString('yyyy-MM-dd')
    notes     = "Papers $Version for Windows x64."
    notes_url = "https://github.com/$Repository/releases/tag/v$Version"
    installer = Asset "Papers-$Version-windows-x64-setup.exe"
    portable  = Asset "Papers-$Version-windows-x64-portable.zip"
}
$json = $manifest | ConvertTo-Json -Depth 4
# UTF-8 without a byte order mark, so any JSON reader accepts it.
[System.IO.File]::WriteAllText((Join-Path $OutDir 'latest.json'), $json, (New-Object System.Text.UTF8Encoding $false))
Write-Host $json
