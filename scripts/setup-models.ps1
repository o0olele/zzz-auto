param([switch]$Offline)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    $prepareArgs = @('run', '-p', 'zzz-engine', '--locked', '--', 'model-resources', 'prepare')
    if ($Offline) { $prepareArgs += '--offline' }
    & cargo @prepareArgs
    if ($LASTEXITCODE -ne 0) { throw '模型资源准备失败。' }
    & cargo run -p od-tools --locked -- models --source sdk/models --output resources/generated/recognition
    if ($LASTEXITCODE -ne 0) { throw '模型转换失败。' }
}
finally { Pop-Location }
