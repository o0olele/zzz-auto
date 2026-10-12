param([string]$ArchivePath)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$lock = Get-Content -Raw -LiteralPath (Join-Path $projectRoot 'sdk.lock.json') | ConvertFrom-Json
$cacheDirectory = Join-Path $projectRoot 'sdk'
New-Item -ItemType Directory -Force -Path $cacheDirectory | Out-Null
if (-not $ArchivePath) {
    $ArchivePath = Join-Path $cacheDirectory $lock.archive
    if (-not (Test-Path -LiteralPath $ArchivePath)) {
        Invoke-WebRequest -Uri $lock.url -OutFile "$ArchivePath.partial"
        Move-Item -LiteralPath "$ArchivePath.partial" -Destination $ArchivePath
    }
}
$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $ArchivePath).Hash.ToLowerInvariant()
if ($hash -ne $lock.sha256) { throw 'Maa SDK 压缩包校验失败，请检查下载来源。' }
$destination = Join-Path $cacheDirectory 'maa'
Expand-Archive -LiteralPath $ArchivePath -DestinationPath $destination -Force
if (-not (Test-Path -LiteralPath (Join-Path $destination 'bin/MaaFramework.dll'))) {
    throw 'SDK 缺少 bin/MaaFramework.dll，目录结构与固定版本不符。'
}
Write-Output "Maa SDK $($lock.sdk_version) 已校验并解压到 $destination"
