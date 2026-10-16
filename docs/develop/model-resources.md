# M3 模型资源下载与桌面管理

固定模型包的下载、缓存校验和解压已迁到 Rust，提供独立引擎 CLI 及桌面“模型资源”页。只处理 `models.lock.json` 中的四个固定模型，不接受外部 URL 或清单，不下载或替换 Maa DLL。完整运行资源包的原生构建、本地导入、桌面版本切换及回滚见[运行资源包](runtime-resources.md)；完整包联网下载与签名发布继续后续实现。

## 桌面使用与任务

打开“模型资源”，先点击“检查缓存”核对文件；选择所需模型后点击“准备所选模型”。勾选“仅使用离线缓存”时不联网。页面只提交固定模型编号、检查 / 准备和离线选项；缓存固定在引擎数据目录的 `model-cache/`，不向界面开放路径、URL 或外部清单。模型缓存与安装目录内的生成资源分开，下载完成不代表模型已被游戏任务采用。

资源任务由专用工作线程执行，启动立即返回任务 ID，状态和最新进度由引擎保存；界面离开页面后重新查询，不因组件卸载重新下载。每个引擎只允许一个资源任务，检查和准备共同互斥。它没有游戏输入，也不占用游戏运行槽，游戏控制和停止接口继续可用。

点击“取消资源任务”时核对任务 ID，先进入取消中，等待网络、文件和锁收尾后才提供终态。原生 HTTPS 等待每 25ms 检查取消标记，退出和失联取消资源任务并等待收尾。正常关闭后不遗留下载线程；强制终止遗留目录继续按已有规则忽略。引擎重启不自动重放资源任务，页面重新检查缓存。当前协议为 v15，桌面与引擎须成套更新。

页面每 750ms 查询保存在引擎中的状态，不在每次查询时重新扫描磁盘。检查或准备后才更新对应模型的校验状态；任务失败后未确认的项目显示“尚未检查”。读取失败暂时禁止新任务，连接恢复后重新读取；离开页面和断线前发出的迟到响应不会恢复旧表单，也不会自动重试下载。

离线准备时，可把固定 ZIP 放入页面显示的缓存目录；也可以在退出桌面后，通过 CLI 的 `--cache` 指向该目录。安装版及开发桌面默认数据目录为 `%APPDATA%/io.onedragon.zzz-auto`，便携版使用程序目录下的 `data`。CLI 的默认 `sdk/models` 与桌面缓存是两个独立来源，不自动复制开发机模型。

## 实施约定

`od-platform` 提供共享模型清单与缓存管理，`od-tools` 继续使用同一清单进行资源转换。`model-resources status` 只读检查，不加载 SDK、创建账号配置或连接游戏；`prepare` 可选择模型，默认准备全部。`--offline` 禁止联网，有效解压文件可直接复用，否则必须有完整且校验通过的 ZIP。

缓存沿用 `sdk/models/<id>` 与 `<id>.zip`，兼容既有开发缓存。每个模型包先在独立临时目录接收，限制下载长度，校验 SHA-256 和大小后才成为完整缓存；只解压清单指定的文件，逐个核对长度与 SHA-256。所有文件通过后才启用该模型目录，已有有效模型直接复用，即使 ZIP 缺失也可以离线使用。失败时保留已有缓存；替换的无效内容改名为 `.invalid-<原名>-<随机编号>` 保留，重试不会把半包当成完成。

同一缓存目录使用文件锁串行准备，磁盘状态查询遇到准备中的目录返回占用错误。拒绝目录联接、符号链接、ZIP 越界路径与重复条目。Ctrl+C 取消在网络等待、下载块、校验块和文件解压间检查；网络请求仍有 15 秒连接超时和 120 秒总超时。启用是每个模型独立完成的，取消不会撤回已经校验并启用的前项。正常错误和取消清理本次临时文件；强制结束进程可能留下 `.prepare-*`，后续不会将其作为完整缓存，也不自动重放下载。

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

发行引擎可以直接执行 `zzz-auto-engine.exe model-resources ...`，这一步无需 Rust、PowerShell、Node 或 Python。当前开发脚本 `scripts/setup-models.ps1` 已改为调用该实现，再运行既有 Rust 模型转换。桌面此页只接通缓存准备；转换后的运行资源使用独立的[运行资源包入口](runtime-resources.md)。

## 验证

独立测试仓覆盖只读查询、编号拒绝、固定包离线解压、文件复用、锁定来源、坏包与同长度错误哈希、旧缓存保留、取消清理、缓存互斥、联接拒绝和无 SDK / 账号数据的引擎 CLI。传输错误用受控流复现，不请求额外网络服务。

桌面接入另覆盖任务互斥、终态查询、取消 ID、退出和最后一个持有者释放、异常传输、无 SDK 的引擎命令、固定缓存目录及隔离本机 TLS 握手阻塞取消；前端覆盖模型选择、离线参数、切页后恢复任务、取消收尾、读写失败、断线迟到响应、明暗主题与较小窗口。

```powershell
cargo build -p zzz-engine -p od-tools --locked
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test model_resources --test regression --test coffee --test map_transport -- --test-threads=1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/setup-models.ps1 -Offline
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test desktop_resources --test model_resources --test regression --test elevation --test input_recovery -- --test-threads=1
npm --prefix zzz-od-test/test/zzz_auto/frontend test -- --reporter=line
```

本机已用新的可取消 HTTPS 路径再次从锁定 GitHub 地址下载闪光模型到隔离缓存，确认包校验、文件校验和解压全过程；四个既有模型离线复用及识别资源转换继续通过。具体测试结果见[验收记录](../migration/acceptance.md)。本机 TLS 测试需要正常 Windows Schannel 凭据，受限沙箱可能在握手前报缺少凭据；测试不关闭证书校验。咖啡素材、地图实机及多应用交接继续待验收。
