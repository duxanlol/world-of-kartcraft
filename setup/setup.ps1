# World of Kartcraft setup (Windows). Double-click setup.bat, or from PowerShell:
#
#   powershell -ExecutionPolicy Bypass -File setup\setup.ps1                 # everything
#   powershell -ExecutionPolicy Bypass -File setup\setup.ps1 -Wow DIR -Ctr DISC.bin
#   powershell -ExecutionPolicy Bypass -File setup\setup.ps1 -Doctor         # only check this PC
#   powershell -ExecutionPolicy Bypass -File setup\setup.ps1 -ClientOnly     # play on a friend's server
#   powershell -ExecutionPolicy Bypass -File setup\setup.ps1 -NewAccount     # an account for a friend
#
# Every step is skipped when its result is already there, so after a failure fix the reported
# problem and run it again. Everything is logged to setup.log.
param(
    [string]$Wow = "",
    [string]$Ctr = "",
    [switch]$Doctor,
    # Only the game: for joining someone else's Kartcraft server (no Docker, no local server).
    [switch]$ClientOnly,
    # Make another game account (for a friend) even though setup already made yours.
    [switch]$NewAccount
)
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'

$Root = Split-Path -Parent $PSScriptRoot
Set-Location $Root
$State = Join-Path $Root '.setup'
New-Item -ItemType Directory -Force -Path $State | Out-Null
Start-Transcript -Path (Join-Path $Root 'setup.log') -Append | Out-Null

# Pinned upstreams, the same as setup/setup.sh.
$ServerUrl = 'https://github.com/mserajnik/vmangos-deploy'
$ServerRev = 'f0692574b7955e78d68fea39372e0045360ce5ee'
$StockImage = 'ghcr.io/mserajnik/vmangos-server:5875'
# The vmangos commit the Kartcraft patch is made against.
$VmangosRev = '0e3ff01e76d4758e8a7c3108b2717cc785ed56fa'
if (-not $env:KARTCRAFT_SERVER_PROJECT) { $env:KARTCRAFT_SERVER_PROJECT = 'world-of-kartcraft' }
if (-not $env:KARTCRAFT_BUILD_JOBS) { $env:KARTCRAFT_BUILD_JOBS = '4' }

function Step($text) { Write-Host ''; Write-Host "==> $text" -ForegroundColor Cyan }
function Stop-Setup($message, $fix = '') {
    Write-Host ''
    Write-Host "SETUP STOPPED: $message" -ForegroundColor Red
    if ($fix) { Write-Host "Fix: $fix" }
    Write-Host 'Then run setup again; finished steps are skipped.'
    Stop-Transcript | Out-Null
    exit 1
}
# Runs a program and stops setup if it fails (PowerShell 5 does not stop on exit codes).
function Invoke-Native($what, [scriptblock]$command) {
    & $command
    if ($LASTEXITCODE -ne 0) { Stop-Setup "$what failed (see the lines above)" }
}
function Test-Native([scriptblock]$command) {
    # PowerShell 5 under `Stop` turns any redirected stderr line (docker's warnings) into an
    # exception; only the exit code decides here.
    $ErrorActionPreference = 'Continue'
    try { & $command *> $null; return $LASTEXITCODE -eq 0 } catch { return $false }
}

# ── Doctor ─────────────────────────────────────────────────────────────────────────────────────
function Test-Machine {
    Step 'Checking this PC'
    $script:bad = $false
    function Check($name, [bool]$ok, $fix) {
        if ($ok) { Write-Host "  ok       $name" }
        else { Write-Host "  MISSING  $name" -ForegroundColor Yellow; Write-Host "           fix: $fix"; $script:bad = $true }
    }
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $msvc = (Test-Path $vswhere) -and
        [bool](& $vswhere -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath)
    Check 'Rust (rustup/cargo)' ([bool](Get-Command cargo -ErrorAction SilentlyContinue)) `
        'winget install -e --id Rustlang.Rustup   (then open a new terminal)'
    Check 'Visual Studio C++ build tools' $msvc `
        'winget install -e --id Microsoft.VisualStudio.2022.BuildTools --override "--quiet --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"'
    Check 'git' ([bool](Get-Command git -ErrorAction SilentlyContinue)) 'winget install -e --id Git.Git'
    if (-not $ClientOnly) {
        Check 'Docker Desktop' ([bool](Get-Command docker -ErrorAction SilentlyContinue)) 'winget install -e --id Docker.DockerDesktop   (then restart Windows)'
        Check 'Docker compose' (Test-Native { docker compose version }) 'update Docker Desktop'
        Check 'Docker Desktop running' (Test-Native { docker info }) 'start Docker Desktop and wait until it says it is running'
    }
    $drive = (Split-Path -Qualifier $Root).TrimEnd(':')
    $free = [math]::Floor((Get-PSDrive $drive).Free / 1GB)
    Check "20 GB free disk ($free GB)" ($free -ge 20) 'free up disk space'
    $long = (Get-ItemProperty 'HKLM:\SYSTEM\CurrentControlSet\Control\FileSystem' -ErrorAction SilentlyContinue).LongPathsEnabled
    if ($long -ne 1 -and $Root.Length -gt 40) {
        Write-Host "  note     the repo path is long and Windows long paths are off; if the build fails with 'path too long', move the repo to something short like C:\kartcraft"
    }
    return -not $script:bad
}

if ($Doctor) {
    if (Test-Machine) { Write-Host 'All good.'; Stop-Transcript | Out-Null; exit 0 }
    Stop-Transcript | Out-Null
    exit 1
}
if (-not (Test-Machine)) { Stop-Setup 'this PC is missing the tools listed above' 'install them, open a new terminal, run setup again' }

function Read-Path($value, $prompt) {
    while (-not $value) { $value = Read-Host $prompt }
    $value = $value.Trim().Trim('"')
    return [System.IO.Path]::GetFullPath($value)
}

# ── WoW client ─────────────────────────────────────────────────────────────────────────────────
$WowLink = Join-Path $Root 'WoW'
if (-not (Test-Path (Join-Path $WowLink 'Data'))) {
    Step 'WoW 1.12.1 client'
    $Wow = Read-Path $Wow 'Path to your WoW 1.12.1 (English, build 5875) folder, the one with Data in it'
    if ((Split-Path -Leaf $Wow) -eq 'Data') { $Wow = Split-Path -Parent $Wow }
    foreach ($mpq in 'dbc.MPQ', 'patch.MPQ', 'model.MPQ', 'terrain.MPQ') {
        if (-not (Test-Path (Join-Path $Wow "Data\$mpq"))) {
            Stop-Setup "$Wow\Data has no $mpq" 'point setup at an untouched English 1.12.1 client folder'
        }
    }
    if (Test-Path $WowLink) { (Get-Item $WowLink).Delete() }
    New-Item -ItemType Junction -Path $WowLink -Target $Wow | Out-Null
    Set-Content (Join-Path $State 'wow-path') $Wow
    Write-Host "  linked WoW -> $Wow"
}

# ── The kart, from the player's CTR disc ───────────────────────────────────────────────────────
$KartData = Join-Path $Root 'kart-data'
if (-not (Test-Path (Join-Path $KartData 'kart.json'))) {
    Step 'Converting the kart from your Crash Team Racing disc'
    $CtrFile = Join-Path $State 'ctr-path'
    if (-not $Ctr -and (Test-Path $CtrFile)) { $Ctr = (Get-Content $CtrFile -Raw).Trim() }
    $Ctr = Read-Path $Ctr 'Path to your Crash Team Racing (PS1, USA, SCUS-94426) disc image, the .bin file'
    if ($Ctr.EndsWith('.cue')) { $Ctr = [System.IO.Path]::ChangeExtension($Ctr, '.bin') }
    if (-not (Test-Path $Ctr)) { Stop-Setup "$Ctr does not exist" 'point setup at the .bin of a raw (2352-byte sector) CTR NTSC-U image' }
    Set-Content $CtrFile $Ctr
    Invoke-Native 'the kart conversion' {
        cargo run --release --quiet -p ctr-assets --bin ctr-extract -- $Ctr $KartData --all
    }
    if (-not (Test-Path (Join-Path $KartData 'kart.json'))) { Stop-Setup 'the conversion finished without kart.json' }
}

if (-not $ClientOnly) {
# ── Local server ───────────────────────────────────────────────────────────────────────────────
$Server = Join-Path $Root 'server'
$Compose = Join-Path $Server 'compose.yaml'
function Invoke-Compose { & docker compose -p $env:KARTCRAFT_SERVER_PROJECT --project-directory $Server -f $Compose @args }
if (-not (Test-Path $Compose)) {
    Step 'Setting up the local vmangos server'
    if (-not (Test-Path (Join-Path $Server '.git'))) { Invoke-Native 'cloning vmangos-deploy' { git clone --quiet -c core.autocrlf=false $ServerUrl $Server } }
    Invoke-Native 'checking out vmangos-deploy' { git -C $Server checkout --quiet $ServerRev }
    Copy-Item (Join-Path $Server 'config\mangosd.conf.example') (Join-Path $Server 'config\mangosd.conf')
    Copy-Item (Join-Path $Server 'config\realmd.conf.example') (Join-Path $Server 'config\realmd.conf')
    # Horde and Alliance race together: a PvP realm lets an account have both only with this; and
    # talk together, in say, whispers and every channel (the World channel the client joins).
    $MangosConf = Join-Path $Server 'config\mangosd.conf'
    [IO.File]::WriteAllText($MangosConf, ([IO.File]::ReadAllText($MangosConf) -replace '(?m)^AllowTwoSide\.Accounts = 0', 'AllowTwoSide.Accounts = 1' -replace '(?m)^AllowTwoSide\.Interaction\.Chat = 0', 'AllowTwoSide.Interaction.Chat = 1' -replace '(?m)^AllowTwoSide\.Interaction\.Channel = 0', 'AllowTwoSide.Interaction.Channel = 1'))
    # The example's `user:` lines only matter on Linux hosts; its time zone stays UTC.
    Copy-Item (Join-Path $Server 'compose.yaml.example') $Compose
}

# Everything in server\ runs inside Linux containers (patches, shell scripts), so its checkout keeps
# LF whatever git's autocrlf says; a clone made with CRLF is converted once.
if ((& git -C $Server config core.autocrlf) -ne 'false') {
    Step 'Converting the server checkout to LF line endings'
    Invoke-Native 'setting autocrlf' { git -C $Server config core.autocrlf false }
    Invoke-Native 'refreshing the index' { git -C $Server rm --cached -r -q . }
    Invoke-Native 'checking out with LF' { git -C $Server reset --hard -q }
}

# The server with the Karting profession: vmangos built with setup\vmangos\kartcraft.patch,
# tagged by the patch's contents so a changed patch builds a new image (as build-server.sh).
$Patch = Join-Path $Root 'setup\vmangos\kartcraft.patch'
$Tag = (Get-FileHash -Algorithm SHA256 $Patch).Hash.Substring(0, 12).ToLower()
$Image = "world-of-kartcraft/vmangos-server:5875-$Tag"
if (-not (Test-Native { docker image inspect $Image })) {
    Step "Building the server with the Karting profession ($env:KARTCRAFT_BUILD_JOBS jobs, several minutes)"
    $Context = Join-Path $State 'server-build'
    if (Test-Path $Context) { Remove-Item -Recurse -Force $Context }
    New-Item -ItemType Directory -Force -Path $Context | Out-Null
    Copy-Item -Recurse (Join-Path $Server 'docker') $Context
    # LF as committed: the build applies it with git inside Linux.
    $bytes = [System.IO.File]::ReadAllBytes($Patch)
    $text = [System.Text.Encoding]::UTF8.GetString($bytes).Replace("`r`n", "`n")
    [System.IO.File]::WriteAllText((Join-Path $Context 'docker\patches\zz-kartcraft.patch'), $text, (New-Object System.Text.UTF8Encoding $false))
    # The frontend named first: older Docker Desktops build without `COPY --exclude` otherwise.
    $dockerfile = "# syntax=docker/dockerfile:1.7-labs`n" + (Get-Content -Raw (Join-Path $Context 'docker\server\Dockerfile')).Replace('make -j"$(nproc)"', "make -j$env:KARTCRAFT_BUILD_JOBS")
    [System.IO.File]::WriteAllText((Join-Path $Context 'Dockerfile.kartcraft'), $dockerfile.Replace("`r`n", "`n"), (New-Object System.Text.UTF8Encoding $false))
    Invoke-Native 'building the server image' {
        docker build -t $Image -f (Join-Path $Context 'Dockerfile.kartcraft') `
            --build-arg VMANGOS_REVISION=$VmangosRev `
            --build-arg VMANGOS_CLIENT_VERSION=5875 `
            --build-arg VMANGOS_FAIL_ON_PATCH_ERROR=1 `
            $Context
    }
}
$composeText = Get-Content -Raw $Compose
$pointed = [regex]::Replace($composeText, 'image: (ghcr\.io/mserajnik/vmangos-server:5875|world-of-(skate|kart)craft/vmangos-server:\S+)', "image: $Image")
# The database image floats on `latest`, which moved on to vmangos-deploy's version 1 (2026-10-04):
# pin it to the build for our vmangos revision and give it the version it checks for.
$pointed = [regex]::Replace($pointed, 'image: ghcr\.io/mserajnik/vmangos-database(:\S+)?', "image: ghcr.io/mserajnik/vmangos-database:$VmangosRev")
if ($pointed -notmatch 'VMANGOS_DEPLOY_VERSION') {
    # The top-level service only (`^  database:`), and once: other services name it too.
    $pointed = ([regex]'(?ms)(^  database:.*?^    environment:\r?\n)').Replace($pointed, "`$1      - VMANGOS_DEPLOY_VERSION=1`n", 1)
}
if ($pointed -ne $composeText) {
    [System.IO.File]::WriteAllText($Compose, $pointed, (New-Object System.Text.UTF8Encoding $false))
    Write-Host "  server image: $Image"
}

if (-not (Test-Path (Join-Path $Server 'storage\mangosd\extracted-data\maps'))) {
    Step 'Extracting server maps from your WoW client (this takes HOURS; leave it running)'
    $WowDir = (Get-Item $WowLink).Target | Select-Object -First 1
    if (-not $WowDir) { $WowDir = (Get-Content (Join-Path $State 'wow-path') -Raw).Trim() }
    $Extracted = Join-Path $Server 'storage\mangosd\extracted-data'
    Invoke-Native 'map extraction' {
        docker run --rm -v "${WowDir}:/opt/vmangos/storage/client-data" `
            -v "${Extracted}:/opt/vmangos/storage/extracted-data" $Image extract-client-data
    }
}
# The Race Marshals (setup\vmangos\sql\kartcraft.sql): the database container runs custom-sql\ on
# every start, so a changed file restarts it, with the world server stopped meanwhile.
$MarshalSql = Join-Path $Root 'setup\vmangos\sql\kartcraft.sql'
$CustomSql = Join-Path $Server 'storage\database\custom-sql\kartcraft.sql'
$sqlText = [System.IO.File]::ReadAllText($MarshalSql).Replace("`r`n", "`n")
if (-not (Test-Path $CustomSql) -or [System.IO.File]::ReadAllText($CustomSql) -ne $sqlText) {
    New-Item -ItemType Directory -Force -Path (Split-Path $CustomSql) | Out-Null
    [System.IO.File]::WriteAllText($CustomSql, $sqlText, (New-Object System.Text.UTF8Encoding $false))
    Write-Host '  Race Marshals: storage\database\custom-sql\kartcraft.sql'
    if (Invoke-Compose ps -q database) {
        Invoke-Native 'stopping the world server' { Invoke-Compose stop mangosd }
        Invoke-Native 'restarting the database' { Invoke-Compose restart database }
    }
}
Step 'Starting the server'
Invoke-Native 'starting the server' { Invoke-Compose up -d }
Write-Host -NoNewline '  waiting for the world server'
function Get-Health { $id = Invoke-Compose ps -q mangosd; if ($id) { docker inspect -f '{{.State.Health.Status}}' $id } }
for ($i = 0; $i -lt 120 -and (Get-Health) -ne 'healthy'; $i++) { Write-Host -NoNewline '.'; Start-Sleep 5 }
Write-Host ''
if ((Get-Health) -ne 'healthy') {
    Stop-Setup 'the world server did not come up within 10 minutes' "look at: docker compose -p $env:KARTCRAFT_SERVER_PROJECT -f server\compose.yaml logs mangosd"
}

# ── Account ────────────────────────────────────────────────────────────────────────────────────
# The first account is yours and a GM; -NewAccount makes a friend's, an ordinary player.
$AccountFile = Join-Path $State 'account'
if ($NewAccount -or -not (Test-Path $AccountFile)) {
    Step 'Game account'
    $first = -not (Test-Path $AccountFile)
    $account = (Read-Host 'Account name (letters and digits, up to 16)').Trim()
    & (Join-Path $PSScriptRoot 'account.ps1') -Account $account -Gm:$first -Local
    if ($first) { Set-Content $AccountFile $account.ToUpper() }
}
}

# ── Build ──────────────────────────────────────────────────────────────────────────────────────
Step 'Building the game (first time: several minutes)'
Invoke-Native 'the build' { cargo build --release -p benilla -p kart-demo }

Step 'Done'
if ($ClientOnly) {
    Write-Host '  Start the game on your friend''s server with:  set WOW_HOST=<their address> & play.bat'
    Write-Host '  Log in with the account they made you, create a character, press J to drive.'
} else {
    Write-Host '  Start the game with:  play.bat      (the offline kart demo, no server: demo.bat)'
    Write-Host "  Log in as $((Get-Content $AccountFile -Raw).Trim()), create a character, press J to drive (PS5, PS4 or Xbox pad, or keyboard)."
}
Stop-Transcript | Out-Null
