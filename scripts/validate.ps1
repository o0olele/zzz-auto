param([switch]$SkipFrontend)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location $projectRoot
try {
    & cargo fmt --all -- --check
    if ($LASTEXITCODE -ne 0) { throw 'Rust 格式检查失败。' }
    & node scripts/prepare-engine.mjs
    if ($LASTEXITCODE -ne 0) { throw '协议生成或引擎构建失败。' }
    & cargo clippy --workspace --all-targets --locked -- -D warnings
    if ($LASTEXITCODE -ne 0) { throw 'Rust 静态检查失败。' }
    $testManifest = Join-Path $projectRoot 'zzz-od-test/test/zzz_auto/rust/Cargo.toml'
    if (-not (Test-Path -LiteralPath $testManifest)) { throw '缺少配套独立测试仓及 Rust 验证工程。' }
    & cargo test --manifest-path $testManifest --target-dir target/validation --locked -- --test-threads=1
    if ($LASTEXITCODE -ne 0) { throw 'Rust 回归测试失败。' }
    if (-not $SkipFrontend) {
        & npm --prefix apps/desktop run build
        if ($LASTEXITCODE -ne 0) { throw '前端构建失败。' }
        Push-Location zzz-od-test/test/zzz_auto/frontend
        try { & npm test; if ($LASTEXITCODE -ne 0) { throw '前端回归测试失败。' } }
        finally { Pop-Location }
    }
}
finally { Pop-Location }
