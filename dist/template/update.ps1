# World of Kartcraft's updater, run by "Play Kartcraft.bat" before the game starts: it asks the
# server's update host for its manifest (every file of the current client with its SHA-256) and
# downloads only the files that differ here, gzip-compressed, checking each one's hash before it
# replaces the old. A new launcher is written beside the running one as .new for the launcher to
# swap in. When the disc converter changed, the kart data is converted again from the player's
# disc. With no update host reachable, the game simply starts as it is.
param(
    [Parameter(Mandatory)] [string]$Base,
    [Parameter(Mandatory)] [string]$Root,
    [string]$Disc = ""
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Add-Type -AssemblyName System.Net.Http
# Azure and most hosts accept only TLS 1.2 and up.
[Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12

$Root = [IO.Path]::GetFullPath($Root).TrimEnd('\')
Get-ChildItem -LiteralPath $Root -Recurse -Filter *.download -ErrorAction SilentlyContinue | Remove-Item -Force
$probe = New-Object System.Net.Http.HttpClient
$probe.Timeout = [TimeSpan]::FromSeconds(8)

function Get-Sha256($path) {
    if (-not (Test-Path -LiteralPath $path)) { return '' }
    (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLower()
}

try {
    $manifest = $probe.GetStringAsync("$Base/manifest.json").GetAwaiter().GetResult() | ConvertFrom-Json
} catch {
    Write-Host "  (no update server at $Base; starting as is)"
    exit 0
}
$client = New-Object System.Net.Http.HttpClient
$client.Timeout = [TimeSpan]::FromMinutes(30)

$local = Join-Path $Root 'version.txt'
$current = if (Test-Path $local) { (Get-Content -Raw $local).Trim() } else { '' }
$stale = @($manifest.files | Where-Object { (Get-Sha256 (Join-Path $Root $_.path)) -ne $_.sha256 })
if ($stale.Count -gt 0) {
    $bytes = ($stale | Measure-Object -Property gz_size -Sum).Sum
    $size = if ($bytes -ge 1MB) { '{0:N1} MB' -f ($bytes / 1MB) } else { '{0:N0} KB' -f [math]::Ceiling($bytes / 1KB) }
    Write-Host "Updating World of Kartcraft $current -> $($manifest.version): $($stale.Count) file(s), $size"
    foreach ($f in $stale) {
        $target = Join-Path $Root $f.path
        $part = "$target.download"
        Write-Host "  $($f.path)"
        $response = $client.GetAsync("$Base/$($f.gz)", [System.Net.Http.HttpCompletionOption]::ResponseHeadersRead).GetAwaiter().GetResult()
        $response.EnsureSuccessStatusCode() | Out-Null
        $in = New-Object System.IO.Compression.GZipStream($response.Content.ReadAsStreamAsync().GetAwaiter().GetResult(), [System.IO.Compression.CompressionMode]::Decompress)
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
        $out = [System.IO.File]::Create($part)
        try { $in.CopyTo($out) } finally { $out.Close(); $in.Close() }
        if ((Get-Sha256 $part) -ne $f.sha256) {
            Remove-Item -LiteralPath $part -Force
            throw "the download of $($f.path) is damaged; try again"
        }
        # The launcher is running: it swaps its .new in itself when this script returns.
        if ($f.path -eq 'Play Kartcraft.bat') {
            Move-Item -LiteralPath $part -Destination "$target.new" -Force
        } else {
            Move-Item -LiteralPath $part -Destination $target -Force
        }
    }
    Set-Content -LiteralPath $local $manifest.version
    Write-Host "  up to date ($($manifest.version))"
} elseif ($current -ne $manifest.version) {
    Set-Content -LiteralPath $local $manifest.version
}

# The kart data comes from the player's disc: convert it again when the converter changed.
$stamp = Join-Path $Root 'kart-data\.converter'
$converted = if (Test-Path $stamp) { (Get-Content -Raw $stamp).Trim() } else { '' }
# Kart data from before the launcher kept the stamp is taken as current.
if (-not $converted -and $manifest.converter -and (Test-Path (Join-Path $Root 'kart-data\kart.json'))) {
    Set-Content -LiteralPath $stamp $manifest.converter
    $converted = $manifest.converter
}
if ($manifest.converter -and $converted -ne $manifest.converter -and (Test-Path (Join-Path $Root 'kart-data\kart.json'))) {
    if ($Disc -and (Test-Path -LiteralPath $Disc)) {
        Write-Host "Converting the kart data from your disc again (the converter was updated)..."
        & (Join-Path $Root 'ctr-extract.exe') $Disc (Join-Path $Root 'kart-data') --all
        if ($LASTEXITCODE -eq 0) { Set-Content -LiteralPath $stamp $manifest.converter }
    } else {
        Write-Host "  (the disc converter was updated; delete kart-data to convert your disc again)"
    }
}
exit 0
