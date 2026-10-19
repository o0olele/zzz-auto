# M3 下载代理设置

通用设置新增下载代理，供模型准备和运行资源包 HTTPS 下载共用。默认跟随系统代理；直连显式关闭代理，自定义模式使用不带认证的 HTTP 或 HTTPS 代理。只接受主机和端口，不接受用户名、密码、路径参数、查询或片段。旧版 GitHub 转发地址和带认证的个人代理只保留归档并提示重新设置，不自动当成普通代理使用。

保存复用账号配置修订、自动备份和运行互斥。下载开始时读取并冻结保存的设置；修改不改变已经运行的下载，下一次下载采用新值。代理错误不自动回退到直连，原有系统证书验证、HTTPS 降级拒绝、超时、取消和完整内容校验继续生效。地址不写入下载进度或任务快照。

桌面资源包下载子进程通过标准输入接收来源与冻结代理，不通过修改进程环境变量传递配置，也不把地址放到命令行。系统模式沿用原有下载客户端的代理发现行为，包括当前进程的代理环境变量。CLI 的模型准备和资源包下载提供 `--proxy system|direct|<HTTP或HTTPS地址>`，默认 system，仅影响该次命令，不改写账号配置。

旧 `env.yml` 中的 `None` 转为直连，可兼容的 `personal` 转为自定义；没有旧代理字段时沿用系统默认。其他模式保留原文并提示，导入预览、备份及离线恢复展示代理选择。协议升级至 v32，导入指纹升级至 v9，既有运行资源内容继续兼容。

## 验证

2026 年 10 月 19 日新增 10 项 Rust 用例，覆盖严格字段及 URL 校验、旧文件缺省读取、修订与备份恢复、旧代理转换和原文保留、HTTP CONNECT 路由、系统环境代理与直连 / 自定义覆盖、网络等待取消，以及两个 CLI 下载入口和桌面下载子进程。模型引擎实际读取保存配置；下载结果和进度不回显来源或代理地址。系统环境测试使用独立子进程，不修改宿主或用户代理。

连同账号配置、备份、离线恢复、模型缓存、桌面模型管理和旧资源兼容，共 68 项 Rust 回归通过。资源兼容逐一验证 schema 1 的协议 15..32 及 schema 2 的协议 16..32，并拒绝未知版本。既有本机 TLS 取消用例在 Windows 沙箱内因凭据限制失败，在沙箱外单独复验通过；没有放宽断言。配置恢复和模型 CLI 用例补齐 `ZZZ_AUTO_TEST_ENGINE`，显式使用配套引擎。

新增 5 项界面用例，连同账号、运行页、备份与离线恢复共 92 项通过，覆盖保存与切页、非法地址、失败及冲突、任务与未保存记录拦截、断线及迟到响应、浏览器禁用。补齐通用设置对未保存运行记录的拦截，与引擎保持一致。明暗主题和 960 像素布局已检查，截图保存在主仓 `artifacts/m3-download-proxy-{light,dark}.png`。

前端生产、配套引擎、包含静态前端的原生 debug 桌面构建、workspace 及相关测试的严格 Clippy、格式与差异检查通过；随附引擎和新构建引擎 SHA-256 一致。前端保留已有大于 500KB 的单块提示。本轮仅使用隔离配置和本机端口，没有修改真实用户配置或运行游戏；真实代理、WebView2 和正式包环境继续待验收。此前独立测试仓全量编译的资源测试接口失配保持待修，本批不声称全量测试通过。

```powershell
$env:ZZZ_AUTO_TEST_ENGINE = (Resolve-Path target/rerun-validation/debug/zzz-engine.exe).Path
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/standalone-recovery --test download_proxy --test account_config --test config_backups --test config_recovery --test model_resources --test desktop_resources -- --test-threads=1
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/standalone-recovery --test runtime_resources shortcut_protocol_preserves_existing_resource_protocols_and_rejects_unknown_versions -- --test-threads=1
# TLS 取消用例需要在可使用 Windows TLS 凭据的环境运行。
$env:PWTEST_CACHE_DIR = "$PWD/target/playwright-proxy-cache"
npm --prefix zzz-od-test/test/zzz_auto/frontend test -- accounts.spec.ts one-dragon-run.spec.ts config-backups.spec.ts offline-config-recovery.spec.ts --reporter=line
npm --prefix apps/desktop run build
cargo clippy --locked --workspace --all-targets -- -D warnings
```
