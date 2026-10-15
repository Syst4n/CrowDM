# Pin the same FFmpeg package used for the preview, independently of the
# changing software preinstalled on GitHub-hosted Windows runners.
$ErrorActionPreference = 'Stop'
$archive = Join-Path $env:RUNNER_TEMP 'crowdm-ffmpeg-7.1.1.zip'
$destination = Join-Path $env:RUNNER_TEMP 'crowdm-ffmpeg-7.1.1'
Invoke-WebRequest -Uri 'https://github.com/GyanD/codexffmpeg/releases/download/7.1.1/ffmpeg-7.1.1-essentials_build.zip' -OutFile $archive
$expected = '04861d3339c5ebe38b56c19a15cf2c0cc97f5de4fa8910e4d47e5e6404e4a2d4'
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $expected) { throw 'FFmpeg archive checksum mismatch' }
Expand-Archive -LiteralPath $archive -DestinationPath $destination -Force
$bin = Join-Path $destination 'ffmpeg-7.1.1-essentials_build\bin'
if (!(Test-Path -LiteralPath (Join-Path $bin 'ffprobe.exe'))) { throw 'The verified FFmpeg package is incomplete' }
$bin | Out-File -LiteralPath $env:GITHUB_PATH -Encoding utf8 -Append
