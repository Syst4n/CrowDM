# Prepare a self-contained Windows media-tool folder. Resolve yt-dlp from its
# official release and verify SHA-256 before it can be run. FFmpeg/Node are
# copied from this PC; no global package or PATH changes are made.
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
$mediaDir = Join-Path $projectRoot 'tools\media'
New-Item -ItemType Directory -Path $mediaDir -Force | Out-Null
$toolPaths = @{}
foreach ($name in @('ffmpeg', 'ffprobe', 'node')) {
    $sourcePath = (Get-Command "$name.exe" -ErrorAction Stop).Source
    $destinationPath = Join-Path $mediaDir "$name.exe"
    if ([IO.Path]::GetFullPath($sourcePath) -ne [IO.Path]::GetFullPath($destinationPath)) { Copy-Item -LiteralPath $sourcePath -Destination $destinationPath -Force }
    $toolPaths[$name] = @{ file = "$name.exe"; sha256 = (Get-FileHash -LiteralPath $destinationPath -Algorithm SHA256).Hash.ToLowerInvariant() }
}
$release = Invoke-RestMethod -Uri 'https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest' -Headers @{ 'User-Agent' = 'CrowDM-build' }
$asset = $release.assets | Where-Object name -eq 'yt-dlp.exe' | Select-Object -First 1
if (-not $asset) { throw 'The official yt-dlp release has no Windows executable.' }
$sumsAsset = $release.assets | Where-Object name -eq 'SHA2-256SUMS' | Select-Object -First 1
$checksums = (Invoke-WebRequest -Uri $sumsAsset.browser_download_url).Content
if ($checksums -is [byte[]]) { $checksums = [Text.Encoding]::UTF8.GetString($checksums) }
$match = [regex]::Match($checksums, '(?m)^([a-fA-F0-9]{64})\s+\*?yt-dlp\.exe\s*$')
if (-not $match.Success) { throw 'Cannot verify the yt-dlp release checksum.' }
$expectedHash = $match.Groups[1].Value.ToLowerInvariant()
$binaryPath = Join-Path $mediaDir 'yt-dlp.exe'
if (-not (Test-Path -LiteralPath $binaryPath) -or (Get-FileHash -LiteralPath $binaryPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHash) {
    $temporaryPath = Join-Path $mediaDir 'yt-dlp.download'
    Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $temporaryPath
    if ((Get-FileHash -LiteralPath $temporaryPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expectedHash) {
        Remove-Item -LiteralPath $temporaryPath
        throw 'yt-dlp checksum mismatch. The downloaded executable was removed.'
    }
    Move-Item -LiteralPath $temporaryPath -Destination $binaryPath -Force
}
$inventory = @{
    ytDlpVersion = $release.tag_name
    ytDlpSource = $asset.browser_download_url
    ytDlpSha256 = $expectedHash
    copiedTools = $toolPaths
}
$inventory | ConvertTo-Json -Depth 3 | Set-Content -LiteralPath (Join-Path $mediaDir 'inventory.json') -Encoding utf8
& (Join-Path $mediaDir 'ffmpeg.exe') -L 2>&1 | Out-String | Set-Content -LiteralPath (Join-Path $mediaDir 'FFmpeg-LICENSE.txt') -Encoding utf8
& (Join-Path $mediaDir 'ffmpeg.exe') -version 2>&1 | Out-String | Set-Content -LiteralPath (Join-Path $mediaDir 'FFmpeg-BUILD.txt') -Encoding utf8
$nodeVersion = & (Join-Path $mediaDir 'node.exe') -p 'process.versions.node'
Invoke-WebRequest -Uri "https://raw.githubusercontent.com/yt-dlp/yt-dlp/$($release.tag_name)/LICENSE" -OutFile (Join-Path $mediaDir 'yt-dlp-LICENSE.txt')
Invoke-WebRequest -Uri "https://raw.githubusercontent.com/nodejs/node/v$nodeVersion/LICENSE" -OutFile (Join-Path $mediaDir 'Node-LICENSE.txt')
Invoke-WebRequest -Uri 'https://raw.githubusercontent.com/FFmpeg/FFmpeg/n7.1.1/COPYING.GPLv3' -OutFile (Join-Path $mediaDir 'GPL-3.0.txt')
Write-Output "Media tools ready: yt-dlp $($release.tag_name), FFmpeg, FFprobe and Node ($mediaDir)"
