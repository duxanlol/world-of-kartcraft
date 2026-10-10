# Makes a game account on the Kartcraft server: the local one, or a remote one over SSH.
#
#   powershell -ExecutionPolicy Bypass -File setup\account.ps1                  # asks name and password
#   powershell -ExecutionPolicy Bypass -File setup\account.ps1 -Gm              # a GM account (yours)
#   powershell -ExecutionPolicy Bypass -File setup\account.ps1 -Remote kart@host -Key C:\path\key
#
# With no -Remote, a remote server saved in .setup\remote ("user@host" on the first line, the key
# file on the second) is used, else the local Docker server. An existing account keeps its
# characters and gets the new password.
param(
    [string]$Account = "",
    [string]$Password = "",
    [switch]$Gm,
    [string]$Remote = "",
    [string]$Key = "",
    # The local server even when .setup/remote names a remote one (setup uses this).
    [switch]$Local
)
$ErrorActionPreference = 'Stop'
$Root = Split-Path -Parent $PSScriptRoot
$Project = if ($env:KARTCRAFT_SERVER_PROJECT) { $env:KARTCRAFT_SERVER_PROJECT } else { 'world-of-kartcraft' }
$RootPassword = if ($env:KARTCRAFT_DB_ROOT_PASSWORD) { $env:KARTCRAFT_DB_ROOT_PASSWORD } else { 'password' }

$saved = Join-Path $Root '.setup\remote'
if (-not $Remote -and -not $Local -and (Test-Path $saved)) {
    $lines = @(Get-Content $saved)
    $Remote = $lines[0].Trim()
    if (-not $Key -and $lines.Count -gt 1) { $Key = $lines[1].Trim() }
}

Add-Type -AssemblyName System.Numerics
# vmangos `SRP6::CalculateVerifier` (as tools\make_account.py): x = SHA1(salt_le || SHA1(USER:PASS))
# read little-endian, v = 7^x mod N; salt and verifier stored as big-endian hex.
function Get-Verifier([string]$user, [string]$password, [byte[]]$saltLe) {
    $sha = [System.Security.Cryptography.SHA1]::Create()
    $inner = $sha.ComputeHash([System.Text.Encoding]::ASCII.GetBytes("$($user.ToUpper()):$($password.ToUpper())"))
    $outer = $sha.ComputeHash([byte[]]($saltLe + $inner))
    $x = New-Object System.Numerics.BigInteger (, [byte[]]($outer + [byte]0))
    $n = [System.Numerics.BigInteger]::Parse('0894B645E89E1535BBDAD5B8B290650530801B18EBFBF5E8FAB3C82872A3E9BB7', 'AllowHexSpecifier')
    $v = [System.Numerics.BigInteger]::ModPow(7, $x, $n)
    return $v.ToString('X').TrimStart('0')
}

# The query goes in on stdin, so no shell in between has to quote it.
function Invoke-Sql($query) {
    $mariadb = "mariadb -uroot -p$RootPassword -N -B realmd"
    if ($Remote) {
        $sshArgs = @('-o', 'BatchMode=yes', '-o', 'StrictHostKeyChecking=accept-new')
        if ($Key) { $sshArgs += @('-i', $Key) }
        $query | & ssh @sshArgs $Remote "cd kartcraft && sudo docker compose -p $Project exec -T database $mariadb"
    } else {
        $server = Join-Path $Root 'server'
        $query | & docker compose -p $Project --project-directory $server -f (Join-Path $server 'compose.yaml') exec -T database sh -c $mariadb
    }
    if ($LASTEXITCODE -ne 0) { throw 'the account database refused the account' }
}

$where = if ($Remote) { "the server at $Remote" } else { 'the local server' }
Write-Host "Making an account on $where"
while ($Account -notmatch '^[A-Za-z0-9]{1,16}$') { $Account = (Read-Host 'Account name (letters and digits, up to 16)').Trim() }
$Account = $Account.ToUpper()
while ($Password.Length -lt 1 -or $Password.Length -gt 16) {
    $secret = Read-Host 'Password (up to 16 characters)' -AsSecureString
    $Password = [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($secret))
}
$salt = New-Object byte[] 32
[System.Security.Cryptography.RandomNumberGenerator]::Create().GetBytes($salt)
$salt[31] = $salt[31] -bor 0x80
$saltHex = -join ($salt[31..0] | ForEach-Object { $_.ToString('X2') })
$verifier = Get-Verifier $Account $Password $salt
$level = if ($Gm) { 3 } else { 0 }
Invoke-Sql ("INSERT INTO account (username, s, v, joindate) VALUES ('$Account', '$saltHex', '$verifier', NOW()) " +
    "ON DUPLICATE KEY UPDATE s=VALUES(s), v=VALUES(v);`n" +
    "UPDATE account SET gmlevel=$level WHERE username='$Account';`n" +
    "DELETE account_access FROM account_access JOIN account USING (id) WHERE username='$Account';`n" +
    "INSERT INTO account_access (id, gmlevel, RealmID) SELECT id, $level, -1 FROM account WHERE username='$Account' AND $level > 0;`n" +
    "REPLACE INTO realmcharacters (realmid, acctid, numchars) SELECT realmlist.id, account.id, 0 " +
    "FROM realmlist, account LEFT JOIN realmcharacters ON acctid=account.id WHERE acctid IS NULL;`n")
$what = if ($Gm) { ' (GM)' } else { '' }
Write-Host "  account $Account ready$what"
