# M3 运行资源包首批

本批把当前邮件、固定地图试运行、健康检查与应用清单组成独立版本的完整运行资源包。模型下载仍只准备来源缓存；运行资源包使用已转换并校验的资源。咖啡真实执行、战斗模型和其他未迁移应用不因此开放。

## 版本与启用规则

原生引擎 CLI 提供 `runtime-resources pack|check|install|status|activate|rollback`。首批从本地 ZIP 导入，安装必须给出预期 ZIP SHA-256；联网发现版本、下载完整包、签名发布及桌面切换入口后续实现。包记录内容版本、引擎、SDK、Rust 绑定、通信协议和资源格式，当前要求与本引擎精确匹配。逐文件校验之外，继续使用邮件和地图的固定流程及模型校验；健康检查和应用清单与编译时内容一致。

数据目录的 `runtime-resources/versions/<清单哈希>/` 保存完整版本，`selection.json` 单次替换当前与上一版本。安装仅准备版本，不自动启用；相同清单重复安装只校验复用。内容版本只是展示值，目录由清单哈希决定；已存在但损坏的版本报错并保留，不覆盖。ZIP 不允许额外条目、重复路径、目录链接、越界路径或原生二进制。

安装、启用和回滚与账号配置共用引擎所有权锁，必须先退出使用同一数据目录的引擎，包括已暂停的引擎。引擎取得该锁后读取当前选择并固定本次资源路径，直到进程退出；完整文件校验在原生工作线程完成，首个状态快照不等待模型哈希。校验通过前 SDK 保持未就绪，禁止连接游戏或投递输入；失败保留可查询的错误。未选择版本时继续使用随程序附带的资源。回滚交换当前和上一版本，上一项可为随程序资源；每次启用与回滚都先完整校验目标，失败保留原选择。

读取状态不创建目录；损坏的选择或当前版本明确报错，不静默切到其他版本。崩溃留下的临时文件不作为完成版本；只有完整校验的目录才进入 versions，只有完整写入并同步后的选择文件才能生效。程序、Maa DLL、账号 YAML、运行记录和旧配置归档不进入资源包，也不被本功能覆盖。

## 使用

先用既有 `mail-bundle`、`map-bundle` 生成转换资源，再在工程根目录运行：

```powershell
cargo run -p zzz-engine --locked -- runtime-resources pack --source resources --version m3-preview-1 --output artifacts/runtime-m3-preview-1.zip
cargo run -p zzz-engine --locked -- runtime-resources check --archive artifacts/runtime-m3-preview-1.zip --sha256 <打包输出的哈希>
cargo run -p zzz-engine --locked -- --data <绝对数据目录> runtime-resources install --archive artifacts/runtime-m3-preview-1.zip --sha256 <打包输出的哈希>
cargo run -p zzz-engine --locked -- --data <绝对数据目录> runtime-resources activate --id <安装输出的清单哈希>
cargo run -p zzz-engine --locked -- --data <绝对数据目录> runtime-resources status
cargo run -p zzz-engine --locked -- --data <绝对数据目录> runtime-resources rollback
```

发行引擎可直接执行相同子命令，无需 Python、Node 或 Rust。桌面下次启动使用同一数据目录中的选择；开发与安装桌面默认 `%APPDATA%/io.onedragon.zzz-auto`，便携版默认程序目录下 `data`。CLI 默认工程或程序根目录下 `data`，操作桌面数据时须显式指定。开发 CLI 的显式 `--resources` 用于检查指定资源，覆盖自动选择；桌面入口始终按数据目录选择。

## 验证范围

独立测试覆盖真实生成资源打包、ZIP 与逐文件校验、无 SDK 检查、重复安装、版本切换、回滚到随程序资源、损坏与不兼容包、额外或重复条目、目录联接、引擎所有权冲突、保存失败保留选择，以及引擎启动实际选择资源。所有导入在临时目录中进行；不改变用户当前选择或操作真实游戏。

```powershell
cargo build -p zzz-engine --locked
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test runtime_resources -- --test-threads=1
```

本批没有新增 IPC 或修改前端，通信协议保持 v15。完整资源目前指已有执行器需要的 20 个文件，包含两份锁定 OCR；战斗识别生成资源和咖啡只读预览不随本包分发。资源格式与执行器扩展时同步更新文件清单及兼容校验。
