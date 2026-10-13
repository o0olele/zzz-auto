param([switch]$Offline)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
$modelLock = Get-Content -LiteralPath (Join-Path $projectRoot 'models.lock.json') -Raw -Encoding UTF8 | ConvertFrom-Json
$modelCache = Join-Path $projectRoot 'sdk/models'
New-Item -ItemType Directory -Force -Path $modelCache | Out-Null
foreach ($model in $modelLock.models) {
    if ($model.id -notmatch '^[a-zA-Z0-9_-]+$' -or $model.archive -ne "$($model.id).zip") { throw '模型缓存名称无效。' }
    $archivePath = Join-Path $modelCache $model.archive
    if (-not (Test-Path -LiteralPath $archivePath)) {
        if ($Offline) { throw "离线缓存缺少 $($model.archive)。" }
        $partialPath = "$archivePath.partial"
        Invoke-WebRequest -Uri $model.url -OutFile $partialPath -TimeoutSec 120
        if ((Get-FileHash -LiteralPath $partialPath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $model.sha256) { throw "模型下载校验失败：$($model.id)。" }
        Move-Item -LiteralPath $partialPath -Destination $archivePath
    }
    if ((Get-Item -LiteralPath $archivePath).Length -ne $model.bytes -or (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant() -ne $model.sha256) { throw "模型压缩包校验失败：$($model.id)。" }
    # 仅在固定缓存目录解压，拒绝 ZIP 中的绝对路径或目录穿越。
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zipArchive = [IO.Compression.ZipFile]::OpenRead($archivePath)
    $destination = [IO.Path]::GetFullPath((Join-Path $modelCache $model.id))
    try {
        foreach ($entry in $zipArchive.Entries) {
            $entryTarget = [IO.Path]::GetFullPath((Join-Path $destination $entry.FullName))
            if (-not $entryTarget.StartsWith($destination + '\', [StringComparison]::OrdinalIgnoreCase)) { throw '模型 ZIP 路径不在缓存目录内。' }
        }
    }
    finally { $zipArchive.Dispose() }
    Expand-Archive -LiteralPath $archivePath -DestinationPath $destination -Force
    Write-Output "已校验 $($model.id)"
}
Push-Location $projectRoot
try {
    & cargo run -p od-tools --locked -- models --source sdk/models --output resources/generated/recognition
    if ($LASTEXITCODE -ne 0) { throw '模型转换失败。' }
}
finally { Pop-Location }
