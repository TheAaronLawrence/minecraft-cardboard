# Updates this folder to the newest minecraft-cardboard release on GitHub. It downloads
# just the exe (a few MB) unless the release changed the game's resources, in which case
# it downloads the full zip once. Run it with update.cmd, or from PowerShell.
#
# The repository is private, so this needs GitHub access: the GitHub CLI logged in
# (`gh auth login`, once), or GITHUB_TOKEN set to a token that can read the repository.
# Written for Windows PowerShell 5.1 and PowerShell 7.

$ErrorActionPreference = 'Stop'
# Windows PowerShell's download progress bar slows downloads to a crawl.
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Repo = 'TheAaronLawrence/minecraft-cardboard'
$Here = $PSScriptRoot

function Get-Token {
    if ($env:GITHUB_TOKEN) { return $env:GITHUB_TOKEN }
    if ($env:GH_TOKEN) { return $env:GH_TOKEN }
    if (Get-Command gh -ErrorAction SilentlyContinue) {
        $token = & gh auth token 2>$null
        if ($LASTEXITCODE -eq 0 -and $token) { return "$token".Trim() }
    }
    throw ("The game's repository is private, so the updater needs your GitHub login. " +
        "Install the GitHub CLI from https://cli.github.com, run 'gh auth login' once, " +
        "then run this again. (Or set GITHUB_TOKEN to a token that can read $Repo.)")
}

function Read-Text($path) {
    if (Test-Path $path) { return ([IO.File]::ReadAllText($path)).Trim() }
    return ''
}

function Save-Asset($release, $suffix, $path) {
    $asset = $release.assets | Where-Object { $_.name.EndsWith($suffix) } | Select-Object -First 1
    if (-not $asset) { throw "Release $($release.tag_name) has no file ending in $suffix." }
    $headers = $Headers.Clone()
    $headers.Accept = 'application/octet-stream'
    Invoke-WebRequest -Uri $asset.url -Headers $headers -OutFile $path -UseBasicParsing
}

try {
    $Headers = @{
        Authorization          = "Bearer $(Get-Token)"
        'User-Agent'           = 'minecraft-cardboard-updater'
        'X-GitHub-Api-Version' = '2022-11-28'
    }
    # Newest first; assigning before piping makes Windows PowerShell list the array's items.
    $releases = Invoke-RestMethod -Uri "https://api.github.com/repos/$Repo/releases?per_page=20" -Headers $Headers
    $latest = $releases | Where-Object { -not $_.draft } | Select-Object -First 1
    if (-not $latest) { throw "No releases found in $Repo." }

    $current = Read-Text (Join-Path $Here 'version.txt')
    if ($current -eq $latest.tag_name) {
        Write-Host "Already up to date ($current)."
        return
    }
    if (Get-Process -Name 'minecraft-cardboard' -ErrorAction SilentlyContinue) {
        throw 'minecraft-cardboard is running. Close it, then run the update again.'
    }
    $from = if ($current) { $current } else { 'an unknown version' }
    Write-Host "Updating from $from to $($latest.tag_name)..."

    $temp = Join-Path ([IO.Path]::GetTempPath()) ('cardboard-update-' + [guid]::NewGuid())
    New-Item -ItemType Directory -Path $temp | Out-Null
    try {
        $remoteAssets = Join-Path $temp 'assets.txt'
        Save-Asset $latest 'assets.txt' $remoteAssets
        $full = (Read-Text $remoteAssets) -ne (Read-Text (Join-Path $Here 'assets.txt'))
        $zip = Join-Path $temp 'update.zip'
        if ($full) {
            Write-Host 'The resources changed, so this downloads the full game (about 25 MB).'
            Save-Asset $latest '-windows-x64.zip' $zip
        } else {
            Write-Host 'Downloading the new exe...'
            Save-Asset $latest '-windows-x64-exe-only.zip' $zip
        }
        $unpacked = Join-Path $temp 'unpacked'
        Expand-Archive -Path $zip -DestinationPath $unpacked
        $inner = Get-ChildItem -Path $unpacked -Directory | Select-Object -First 1
        if ($full) {
            # Replace the folders whole, so files a release dropped don't linger.
            foreach ($dir in 'resources', 'fixtures') {
                $old = Join-Path $Here $dir
                if (Test-Path $old) { Remove-Item -Path $old -Recurse -Force }
            }
        }
        Copy-Item -Path (Join-Path $inner.FullName '*') -Destination $Here -Recurse -Force
        [IO.File]::WriteAllText((Join-Path $Here 'version.txt'), $latest.tag_name)
        Write-Host "Updated to $($latest.tag_name)."
    } finally {
        Remove-Item -Path $temp -Recurse -Force -ErrorAction SilentlyContinue
    }
} catch {
    Write-Host "Update failed: $($_.Exception.Message)" -ForegroundColor Red
    exit 1
}
