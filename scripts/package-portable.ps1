param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    if (-not $SkipBuild) {
        & npm --prefix apps/desktop run tauri -- build --no-bundle
        if ($LASTEXITCODE -ne 0) { throw 'Release 桌面构建失败。' }
    }
    $config = Get-Content -Raw -Encoding UTF8 -LiteralPath apps/desktop/src-tauri/tauri.conf.json | ConvertFrom-Json
    $sdkLock = Get-Content -Raw -Encoding UTF8 -LiteralPath sdk.lock.json | ConvertFrom-Json
    $sdkManifest = Get-Content -Raw -Encoding UTF8 -LiteralPath resources/sdk-manifest.json | ConvertFrom-Json
    # SkipBuild 也必须校验当前引擎和资源，避免生成缺模型或流程版本不匹配的包。
    $mailCheckOutput = & ./target/release/zzz-engine.exe --root $projectRoot mail-bundle-check
    if ($LASTEXITCODE -ne 0) { throw '邮件资源校验失败，请先完成当前源码的 release 构建及 mail-bundle 生成。' }
    $mailCheck = ($mailCheckOutput -join "`n") | ConvertFrom-Json
    if (-not $mailCheck.valid -or $mailCheck.engine_version -ne $config.version) { throw '发布引擎版本或邮件资源无效，请重新构建。' }
    & ./target/release/zzz-engine.exe --root $projectRoot map-bundle-check --bundle resources/generated/map-preview
    if ($LASTEXITCODE -ne 0) { throw '地图资源校验失败，请先完成当前源码的 release 构建及 map-bundle 生成。' }
    & ./target/release/zzz-engine.exe --root $projectRoot coffee-run-bundle-check --bundle resources/generated/coffee-run
    if ($LASTEXITCODE -ne 0) { throw '咖啡资源校验失败，请先完成当前源码的 release 构建及 coffee-run-bundle 生成。' }
    $packageName = "zzz-auto-$($config.version)-windows-x64-prototype"
    $stagingParent = Join-Path $projectRoot ("target/portable-" + [guid]::NewGuid().ToString('N'))
    $staging = Join-Path $stagingParent $packageName
    foreach ($folder in @('', 'sdk/maa/bin', 'resources/pipeline', 'resources/catalog', 'licenses', 'tools')) {
        New-Item -ItemType Directory -Force -Path (Join-Path $staging $folder) | Out-Null
    }
    Copy-Item -LiteralPath target/release/zzz-desktop.exe -Destination (Join-Path $staging 'zzz-desktop.exe')
    Copy-Item -LiteralPath target/release/zzz-engine.exe -Destination (Join-Path $staging 'zzz-auto-engine.exe')
    Copy-Item -LiteralPath resources/sdk-manifest.json -Destination (Join-Path $staging 'resources/sdk-manifest.json')
    Copy-Item -LiteralPath sdk.lock.json -Destination (Join-Path $staging 'sdk.lock.json')
    foreach ($resourceFolder in @('resources/pipeline', 'resources/catalog', 'resources/generated/mail', 'resources/generated/map-preview', 'resources/generated/coffee-run')) {
        $sourceFolder = Join-Path $projectRoot $resourceFolder
        foreach ($file in (Get-ChildItem -LiteralPath $sourceFolder -File -Recurse)) {
            $relative = $file.FullName.Substring($sourceFolder.Length + 1)
            $target = Join-Path (Join-Path $staging $resourceFolder) $relative
            New-Item -ItemType Directory -Force -Path (Split-Path -Parent $target) | Out-Null
            Copy-Item -LiteralPath $file.FullName -Destination $target
        }
    }
    Get-ChildItem -LiteralPath resources/licenses -File | Copy-Item -Destination (Join-Path $staging 'licenses')
    Copy-Item -LiteralPath LICENSE -Destination (Join-Path $staging 'licenses/project.LICENSE')
    Copy-Item -LiteralPath docs/migration/third-party.md -Destination (Join-Path $staging 'licenses/THIRD-PARTY.md')
    Copy-Item -LiteralPath docs/migration/acceptance.md -Destination (Join-Path $staging 'ACCEPTANCE.md')
    Copy-Item -LiteralPath scripts/portable-readme.txt -Destination (Join-Path $staging 'README.txt')
    Copy-Item -LiteralPath scripts/portable-launch.cmd -Destination (Join-Path $staging 'launch.cmd')
    Copy-Item -LiteralPath scripts/portable-check.cmd -Destination (Join-Path $staging 'check-environment.cmd')
    [IO.File]::WriteAllText((Join-Path $staging 'portable.flag'), 'portable-v1', [Text.UTF8Encoding]::new($false))
    & (Join-Path $staging 'zzz-auto-engine.exe') --root $staging mail-bundle-check
    if ($LASTEXITCODE -ne 0) { throw '复制后的邮件资源包校验失败，已停止打包。' }
    & (Join-Path $staging 'zzz-auto-engine.exe') --root $staging map-bundle-check --bundle (Join-Path $staging 'resources/generated/map-preview')
    if ($LASTEXITCODE -ne 0) { throw '复制后的地图资源包校验失败，已停止打包。' }
    & (Join-Path $staging 'zzz-auto-engine.exe') --root $staging coffee-run-bundle-check --bundle (Join-Path $staging 'resources/generated/coffee-run')
    if ($LASTEXITCODE -ne 0) { throw '复制后的咖啡资源包校验失败，已停止打包。' }
    foreach ($binary in $sdkManifest.binaries) {
        $path = Join-Path (Join-Path $projectRoot 'sdk/maa/bin') $binary.path
        if ((Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant() -ne $binary.sha256) { throw "SDK 文件校验失败：$($binary.path)" }
        Copy-Item -LiteralPath $path -Destination (Join-Path (Join-Path $staging 'sdk/maa/bin') $binary.path)
    }
    # 从 Visual Studio 的可再分发目录获取运行库，不能复制 System32 的未知版本。
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (-not (Test-Path -LiteralPath $vswhere)) { throw '缺少 vswhere，无法确定 VC++ 可再分发运行库。' }
    $runtimeFiles = @(& $vswhere -latest -products '*' -find 'VC\Redist\MSVC\*\x64\Microsoft.VC*.CRT\*.dll')
    if ($runtimeFiles.Count -eq 0) { throw '未找到 Visual Studio x64 CRT 可再分发运行库。' }
    $runtimeFiles = @($runtimeFiles | Sort-Object | Select-Object -Unique)
    $runtimeVersions = @()
    foreach ($path in $runtimeFiles) {
        $name = Split-Path -Leaf $path
        $target = Join-Path $staging $name
        if (Test-Path -LiteralPath $target) { throw "发现多个同名 CRT，请选择明确版本：$name" }
        Copy-Item -LiteralPath $path -Destination $target
        # SDK 的依赖搜索从 bin 开始，根目录与 bin 使用同一套相同运行库。
        Copy-Item -LiteralPath $path -Destination (Join-Path (Join-Path $staging 'sdk/maa/bin') $name)
        $runtimeVersions += [ordered]@{ name = $name; version = (Get-Item -LiteralPath $path).VersionInfo.FileVersion }
    }
    foreach ($required in @('msvcp140.dll','vcruntime140.dll','vcruntime140_1.dll')) {
        if (-not (Test-Path -LiteralPath (Join-Path $staging $required))) { throw "缺少运行库 $required" }
    }
    $installer = Join-Path $projectRoot 'sdk/MicrosoftEdgeWebview2Setup.exe'
    if (-not (Test-Path -LiteralPath $installer)) {
        Invoke-WebRequest -Uri 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -OutFile "$installer.partial" -TimeoutSec 120
        Move-Item -LiteralPath "$installer.partial" -Destination $installer
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $installer
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch 'Microsoft Corporation') { throw 'WebView2 安装器未通过微软签名验证。' }
    Copy-Item -LiteralPath $installer -Destination (Join-Path $staging 'tools/MicrosoftEdgeWebview2Setup.exe')
    $manifestFiles = @(Get-ChildItem -LiteralPath $staging -File -Recurse | Sort-Object FullName | ForEach-Object {
        [ordered]@{ path = $_.FullName.Substring($staging.Length + 1).Replace('\','/'); bytes = $_.Length; sha256 = (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256).Hash.ToLowerInvariant() }
    })
    $manifest = [ordered]@{
        schema_version = 1; version = $config.version; target = 'x86_64-pc-windows-msvc'; profile = 'release'; portable = $true
        sdk_version = $sdkLock.sdk_version; binding_version = $sdkLock.binding_version
        created_at_utc = [DateTime]::UtcNow.ToString('o'); vc_runtime = $runtimeVersions
        webview2 = 'Evergreen bootstrapper included; internet needed only if runtime is missing'
        includes_user_data = $false; includes_models = $true; included_models = @('ppocrv5')
        protocol_version = $mailCheck.protocol_version
        mail_resources = [ordered]@{ path = 'resources/generated/mail'; manifest_sha256 = (Get-FileHash -LiteralPath (Join-Path $staging 'resources/generated/mail/mail-manifest.json') -Algorithm SHA256).Hash.ToLowerInvariant(); accepted = $false }
        map_resources = [ordered]@{ path = 'resources/generated/map-preview'; manifest_sha256 = (Get-FileHash -LiteralPath (Join-Path $staging 'resources/generated/map-preview/map-manifest.json') -Algorithm SHA256).Hash.ToLowerInvariant(); accepted = $false }
        coffee_resources = [ordered]@{ path = 'resources/generated/coffee-run'; manifest_sha256 = (Get-FileHash -LiteralPath (Join-Path $staging 'resources/generated/coffee-run/coffee-run-manifest.json') -Algorithm SHA256).Hash.ToLowerInvariant(); accepted = $false }
        files = $manifestFiles
    }
    [IO.File]::WriteAllText((Join-Path $staging 'release-manifest.json'), ($manifest | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    $outputDirectory = Join-Path $projectRoot 'artifacts'
    New-Item -ItemType Directory -Force -Path $outputDirectory | Out-Null
    $clientTimestamp = [TimeZoneInfo]::ConvertTimeBySystemTimeZoneId([DateTime]::UtcNow, 'China Standard Time').ToString('yyyyMMdd-HHmmss')
    $zipPath = Join-Path $outputDirectory "$packageName-$clientTimestamp.zip"
    Add-Type -AssemblyName System.IO.Compression
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    function Write-PortableArchive($path, $base, $files) {
        $zipStream = [IO.File]::Open($path, [IO.FileMode]::CreateNew)
        $archive = [IO.Compression.ZipArchive]::new($zipStream, [IO.Compression.ZipArchiveMode]::Create, $false)
        try {
            foreach ($file in $files) {
                # 显式使用 ZIP 标准分隔符，资源补丁直接按客户端根目录布局。
                $entryName = $file.FullName.Substring($base.Length + 1).Replace('\', '/')
                $entry = $archive.CreateEntry($entryName, [IO.Compression.CompressionLevel]::Optimal)
                $sourceStream = [IO.File]::OpenRead($file.FullName)
                $destinationStream = $entry.Open()
                try { $sourceStream.CopyTo($destinationStream) }
                finally { $destinationStream.Dispose(); $sourceStream.Dispose() }
            }
        }
        finally { $archive.Dispose(); $zipStream.Dispose() }
        $hash = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash.ToLowerInvariant()
        [IO.File]::WriteAllText("$path.sha256", "$hash  $(Split-Path -Leaf $path)`r`n", [Text.UTF8Encoding]::new($false))
        return $hash
    }
    $hash = Write-PortableArchive $zipPath $stagingParent (Get-ChildItem -LiteralPath $staging -Recurse -File | Sort-Object FullName)
    $mailPatchPath = Join-Path $outputDirectory "zzz-auto-$($config.version)-mail-resources-$clientTimestamp.zip"
    $mailPatchHash = Write-PortableArchive $mailPatchPath $staging (Get-ChildItem -LiteralPath (Join-Path $staging 'resources/generated/mail') -Recurse -File | Sort-Object FullName)
    [ordered]@{ zip = $zipPath; sha256 = $hash; bytes = (Get-Item -LiteralPath $zipPath).Length; staging = $staging; files = $manifestFiles.Count; mail_patch = $mailPatchPath; mail_patch_sha256 = $mailPatchHash } | ConvertTo-Json
}
finally { Pop-Location }
