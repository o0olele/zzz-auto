# M3 模型资源下载首批

本批把固定模型包的下载、缓存校验和解压迁到 Rust，提供可独立运行的引擎 CLI。只处理 `models.lock.json` 中的四个固定模型，不接受外部 URL 或清单，不下载或替换 Maa DLL。桌面资源页、完整资源包版本切换及回滚继续后续实现。

## 实施约定

`od-platform` 提供共享模型清单与缓存管理，`od-tools` 继续使用同一清单进行资源转换。`model-resources status` 只读检查，不加载 SDK、创建账号配置或连接游戏；`prepare` 可选择模型，默认准备全部。`--offline` 禁止联网，有效解压文件可直接复用，否则必须有完整且校验通过的 ZIP。

缓存沿用 `sdk/models/<id>` 与 `<id>.zip`，兼容既有开发缓存。每个模型包先在独立临时目录接收，限制下载长度，校验 SHA-256 和大小后才成为完整缓存；只解压清单指定的文件，逐个核对长度与 SHA-256。所有文件通过后才启用该模型目录，已有有效模型直接复用，即使 ZIP 缺失也可以离线使用。失败时保留已有缓存；替换的无效内容改名为 `.invalid-<原名>-<随机编号>` 保留，重试不会把半包当成完成。

同一缓存目录使用文件锁串行准备，状态查询遇到准备中的目录返回占用错误。拒绝目录联接、符号链接、ZIP 越界路径与重复条目。Ctrl+C 取消在下载块、校验块和文件解压间检查；网络请求有 15 秒连接超时和 120 秒总超时，取消可能等待网络请求返回后才收尾。启用是每个模型独立完成的，取消不会撤回已经校验并启用的前项。正常错误和取消清理本次临时文件；强制结束进程可能留下 `.prepare-*`，后续不会将其作为完整缓存，也不自动重放下载。

本批操作的是模型来源缓存。正式运行仍使用已有转换及加载校验后的 `resources/generated/`，不会在线覆盖当前引擎已加载的模型、Pipeline 或用户配置。

## 使用

在工程根目录执行：

```powershell
cargo run -p zzz-engine --locked -- model-resources status
cargo run -p zzz-engine --locked -- model-resources prepare --offline
cargo run -p zzz-engine --locked -- model-resources prepare --model ppocrv5
```

`--cache <目录>` 指定缓存，默认是程序或工程根目录下的 `sdk/models`；`--model` 可以重复指定，不传则选择全部模型。未知或重复编号在创建目录前拒绝。缓存检查只读返回 `missing`、`invalid`、`valid`，`ready` 只代表所选模型文件都有效，不代表游戏功能验收通过；它不以缺失作为命令错误。准备失败、取消或占用冲突以非零退出码返回。

准备命令的标准错误按模型和阶段报告 JSON 进度，下载每增加约 1 MiB 更新一次；标准输出只在全部所选项完成后返回结果，包含 `reused` 与 `downloaded`。网络使用 HTTPS、系统代理与证书校验，最多允许十次重定向，不接受 HTTP 降级；不续传半包。

发行引擎可以直接执行 `zzz-auto-engine.exe model-resources ...`，这一步无需 Rust、PowerShell、Node 或 Python。当前开发脚本 `scripts/setup-models.ps1` 已改为调用该实现，再运行既有 Rust 模型转换。它仍是开发入口，转换的正式发布及桌面接入继续按后续阶段推进。

## 验证

独立测试仓覆盖只读查询、编号拒绝、固定包离线解压、文件复用、锁定来源、坏包与同长度错误哈希、旧缓存保留、取消清理、缓存互斥、联接拒绝和无 SDK / 账号数据的引擎 CLI。传输错误用受控流复现，不请求额外网络服务。

```powershell
cargo build -p zzz-engine -p od-tools --locked
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test model_resources --test regression --test coffee --test map_transport -- --test-threads=1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/setup-models.ps1 -Offline
```

本机额外从锁定 GitHub 地址下载闪光模型到隔离缓存，确认 HTTPS、包校验、文件校验和解压全过程；四个既有模型离线复用及识别资源转换也已通过。具体测试结果见[验收记录](../migration/acceptance.md)。本批没有更改 IPC 或界面，协议保持 v14。咖啡素材、地图实机及多应用交接继续待验收。
