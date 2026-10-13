# Builds the World of Kartcraft client for friends and publishes it as an update.
#
#   powershell -ExecutionPolicy Bypass -File dist\publish.ps1          build, package, publish
#   powershell -ExecutionPolicy Bypass -File dist\publish.ps1 -NoBuild  reuse target-dist's build
#
# It leaves three things in dist\:
#   WorldOfKartcraft\      the client folder (the binaries, launcher, updater, docs, source.zip)
#   WorldOfKartcraft.zip   that folder zipped: send it once to each new friend
#   updates\               what the friends' launchers download from: manifest.json (each file's
#                          SHA-256) and the files gzip-compressed, named by their hash. Serve it
#                          with dist\serve-updates.bat; a launcher fetches only what changed.
# No game data goes into any of them: the kart data is converted on each player's machine.
#
# .setup\publish.cfg (yours, never committed) presets the package, one KEY=value per line:
#   SERVER=host           the game server friends connect to (they are not asked for it)
#   UPDATE_URL=https://... where launchers look for updates (default http://SERVER:8080)
#   REGISTER_URL=https://... the account sign-up page, shown to new players
#   AZURE_STORAGE=name    also upload dist\updates to that storage account's static website
param([switch]$NoBuild)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$repo = Split-Path -Parent $PSScriptRoot
$dist = $PSScriptRoot
$template = Join-Path $dist 'template'
$client = Join-Path $dist 'WorldOfKartcraft'
$updates = Join-Path $dist 'updates'
$bin = Join-Path $repo 'target-dist\release'
Add-Type -AssemblyName System.IO.Compression, System.IO.Compression.FileSystem

$settings = @{}
$settingsFile = Join-Path $repo '.setup\publish.cfg'
if (Test-Path $settingsFile) {
    foreach ($line in Get-Content $settingsFile) {
        if ($line -match '^\s*([A-Z_]+)\s*=\s*(.*?)\s*$') { $settings[$Matches[1]] = $Matches[2] }
    }
}

# Zips a folder's contents with '/' in the entry names: Windows PowerShell's .NET writes '\',
# which other unzippers take for part of a file name.
function New-Zip($from, $to) {
    $from = (Resolve-Path $from).Path.TrimEnd('\')
    $archive = [IO.Compression.ZipFile]::Open($to, [IO.Compression.ZipArchiveMode]::Create)
    try {
        foreach ($file in Get-ChildItem -LiteralPath $from -Recurse -File -Force) {
            $name = $file.FullName.Substring($from.Length + 1).Replace('\', '/')
            [IO.Compression.ZipFileExtensions]::CreateEntryFromFile($archive, $file.FullName, $name, [IO.Compression.CompressionLevel]::Optimal) | Out-Null
        }
    } finally { $archive.Dispose() }
}

if (-not $NoBuild) {
    Write-Host '== Building (static CRT, player build) =='
    $env:CARGO_TARGET_DIR = Join-Path $repo 'target-dist'
    $env:RUSTFLAGS = '-C target-feature=+crt-static'
    Push-Location $repo
    try {
        cargo build --release -p benilla --no-default-features
        if ($LASTEXITCODE) { throw 'building the client failed' }
        cargo build --release -p kart-demo -p ctr-assets
        if ($LASTEXITCODE) { throw 'building the demo and converter failed' }
        # The launcher carries the presets in itself: it is one file, with no defaults.cfg beside it.
        $env:KARTCRAFT_SERVER = $settings['SERVER']
        $env:KARTCRAFT_UPDATE_URL = $settings['UPDATE_URL']
        $env:KARTCRAFT_REGISTER_URL = $settings['REGISTER_URL']
        cargo build --release -p kartcraft-launcher
        if ($LASTEXITCODE) { throw 'building the launcher failed' }
    } finally { Pop-Location }
}

Write-Host '== Assembling dist\WorldOfKartcraft =='
New-Item -ItemType Directory -Force -Path $client | Out-Null
# Only the shipped files are replaced: the folder may also be someone's test install, with its
# kartcraft.cfg and kart-data.
$shipped = [ordered]@{
    'WorldOfKartcraft.exe' = Join-Path $bin 'benilla.exe'
    'kartcraft-demo.exe'   = Join-Path $bin 'kartcraft-demo.exe'
    'ctr-extract.exe'      = Join-Path $bin 'ctr-extract.exe'
    'Kartcraft Launcher.exe' = Join-Path $bin 'kartcraft-launcher.exe'
    'LICENSE-MIT.txt'      = Join-Path $repo 'LICENSE-MIT'
    'LICENSE-APACHE.txt'   = Join-Path $repo 'LICENSE-APACHE'
    'LICENSE-GPL-3.0.txt'  = Join-Path $repo 'LICENSE-GPL-3.0'
    'NOTICE.txt'           = Join-Path $repo 'NOTICE'
}
foreach ($name in $shipped.Keys) { Copy-Item -LiteralPath $shipped[$name] -Destination (Join-Path $client $name) -Force }
# The scripts and readme with Windows line endings, which cmd needs.
foreach ($name in 'Play Kartcraft.bat', 'Demo.bat', 'README.txt', 'update.ps1') {
    $text = [IO.File]::ReadAllText((Join-Path $template $name)) -replace "`r?`n", "`r`n"
    [IO.File]::WriteAllText((Join-Path $client $name), $text, (New-Object Text.UTF8Encoding $false))
}
# The Linux and Steam Deck launcher with Unix line endings, which bash needs.
$text = [IO.File]::ReadAllText((Join-Path $template 'play-proton.sh')) -replace "`r`n", "`n"
[IO.File]::WriteAllText((Join-Path $client 'play-proton.sh'), $text, (New-Object Text.UTF8Encoding $false))
# The launcher's presets; a player's own kartcraft.cfg overrides them.
$defaults = (@('SERVER', 'UPDATE_URL', 'REGISTER_URL') | Where-Object { $settings[$_] } | ForEach-Object { "$_=$($settings[$_])" }) -join "`r`n"
[IO.File]::WriteAllText((Join-Path $client 'defaults.cfg'), "$defaults`r`n")

# GPL: the complete source of this build ships beside it. git when the checkout is one, else the
# tree minus build output and local state.
Write-Host '== source.zip =='
$src = Join-Path $env:TEMP "kartcraft-source-$PID"
Remove-Item -Recurse -Force $src -ErrorAction SilentlyContinue
$excluded = '^(target[^\\/]*|dist|server|kart-data|benilla-config|WoW|extracted|out|\.setup|\.claude|\.git|wow-addons-vanilla)([\\/]|$)|__pycache__|\.probe-identity$|^setup\.log$|^trace-.*\.json$'
Push-Location $repo
try {
    $files = if (Test-Path .git) {
        git ls-files -co --exclude-standard
    } else {
        Get-ChildItem -Recurse -File -Force | ForEach-Object { $_.FullName.Substring($repo.Length + 1) }
    }
    foreach ($f in $files) {
        if ($f -match $excluded) { continue }
        $to = Join-Path $src $f
        New-Item -ItemType Directory -Force -Path (Split-Path -Parent $to) | Out-Null
        Copy-Item -LiteralPath $f -Destination $to
    }
} finally { Pop-Location }
$zip = Join-Path $client 'source.zip'
Remove-Item -Force $zip -ErrorAction SilentlyContinue
New-Zip $src $zip
Remove-Item -Recurse -Force $src

Write-Host '== dist\updates =='
$version = Get-Date -Format 'yyyy.MM.dd-HHmm'
$objects = Join-Path $updates 'files'
New-Item -ItemType Directory -Force -Path $objects | Out-Null
$names = @($shipped.Keys) + 'Play Kartcraft.bat', 'Demo.bat', 'play-proton.sh', 'README.txt', 'update.ps1', 'defaults.cfg', 'source.zip'
$entries = foreach ($name in $names) {
    $path = Join-Path $client $name
    $sha = (Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLower()
    $gz = Join-Path $objects "$sha.gz"
    if (-not (Test-Path $gz)) {
        $in = [IO.File]::OpenRead($path)
        $out = New-Object IO.Compression.GZipStream([IO.File]::Create($gz), [IO.Compression.CompressionLevel]::Optimal)
        try { $in.CopyTo($out) } finally { $out.Close(); $in.Close() }
    }
    [ordered]@{ path = $name; sha256 = $sha; size = (Get-Item $path).Length; gz = "files/$sha.gz"; gz_size = (Get-Item $gz).Length }
}
$converter = ($entries | Where-Object { $_.path -eq 'ctr-extract.exe' }).sha256
$manifest = [ordered]@{ version = $version; converter = $converter; files = @($entries) }
# Written last and whole, so a launcher never reads a manifest naming files not there yet.
$json = $manifest | ConvertTo-Json -Depth 4
[IO.File]::WriteAllText((Join-Path $updates 'manifest.json.tmp'), $json, (New-Object Text.UTF8Encoding $false))
Move-Item -Force (Join-Path $updates 'manifest.json.tmp') (Join-Path $updates 'manifest.json')
# Objects no manifest names any more go; a launcher mid-download of one still has the old manifest,
# so the previous publish's are kept until the next.
$keep = @($entries | ForEach-Object { "$($_.sha256).gz" })
$previous = Join-Path $updates 'previous.txt'
if (Test-Path $previous) { $keep += Get-Content $previous }
Get-ChildItem $objects | Where-Object { $keep -notcontains $_.Name } | Remove-Item -Force
Set-Content $previous ($entries | ForEach-Object { "$($_.sha256).gz" })
[IO.File]::WriteAllText((Join-Path $client 'version.txt'), $version)

# Azure: the storage account's static website ($web) mirrors dist\updates. New objects first, the
# manifest after them, stale objects last, for the same reason as above.
if ($settings['AZURE_STORAGE']) {
    $account = $settings['AZURE_STORAGE']
    Write-Host "== Uploading to Azure storage $account =="
    $key = az storage account keys list -n $account --query '[0].value' -o tsv
    if ($LASTEXITCODE) { throw "no access to storage account $account (az login?)" }
    $auth = @('--account-name', $account, '--account-key', $key, '--only-show-errors')
    $remote = @(az storage blob list -c '$web' --prefix files/ --query '[].name' -o tsv @auth)
    foreach ($e in $entries) {
        if ($remote -contains $e.gz) { continue }
        Write-Host "  $($e.gz)"
        az storage blob upload -c '$web' -n $e.gz -f (Join-Path $updates $e.gz) --content-type application/gzip --overwrite --no-progress @auth -o none
        if ($LASTEXITCODE) { throw "uploading $($e.gz) failed" }
    }
    az storage blob upload -c '$web' -n manifest.json -f (Join-Path $updates 'manifest.json') --content-type application/json --content-cache-control 'no-cache' --overwrite --no-progress @auth -o none
    if ($LASTEXITCODE) { throw 'uploading the manifest failed' }
    foreach ($name in $remote) {
        if ($keep -notcontains ($name -replace '^files/', '')) { az storage blob delete -c '$web' -n $name @auth -o none }
    }
    # The one file new players download (worldofkartcraft.com/download leads here): the launcher,
    # which fetches the rest.
    az storage blob upload -c '$web' -n 'KartcraftLauncher.exe' -f (Join-Path $client 'Kartcraft Launcher.exe') --content-type application/vnd.microsoft.portable-executable --content-disposition 'attachment; filename="Kartcraft Launcher.exe"' --content-cache-control 'no-cache' --overwrite --no-progress @auth -o none
    if ($LASTEXITCODE) { throw 'uploading the launcher failed' }
}

Write-Host '== WorldOfKartcraft.zip =='
$full = Join-Path $dist 'WorldOfKartcraft.zip'
Remove-Item -Force $full -ErrorAction SilentlyContinue
$stage = Join-Path $env:TEMP "kartcraft-zip-$PID"
Remove-Item -Recurse -Force $stage -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force -Path (Join-Path $stage 'WorldOfKartcraft') | Out-Null
foreach ($name in $names + 'version.txt') { Copy-Item -LiteralPath (Join-Path $client $name) -Destination (Join-Path $stage "WorldOfKartcraft\$name") }
New-Zip $stage $full
Remove-Item -Recurse -Force $stage

$mb = [math]::Round((Get-Item $full).Length / 1MB, 1)
Write-Host ''
Write-Host "Published $version."
Write-Host "  New friends: send dist\WorldOfKartcraft\Kartcraft Launcher.exe, or the zip ($mb MB)."
if ($settings['AZURE_STORAGE']) {
    Write-Host "  Everyone else updates the next time they start the game, from Azure storage."
} else {
    Write-Host "  Everyone else updates the next time they start the game, from dist\updates"
    Write-Host "  (keep dist\serve-updates.bat running, port 8080 forwarded)."
}
