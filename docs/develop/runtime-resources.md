# M3 运行资源包首批

本批把当前邮件、固定地图试运行、健康检查与应用清单组成独立版本的完整运行资源包。模型下载仍只准备来源缓存；运行资源包使用已转换并校验的资源。咖啡真实执行、战斗模型和其他未迁移应用不因此开放。

## 版本与启用规则

原生引擎 CLI 提供 `runtime-resources pack|check|install|status|activate|rollback`。首批从本地 ZIP 导入，安装必须给出预期 ZIP SHA-256；桌面版本切换见下方说明，联网发现版本、下载完整包及签名发布后续实现。包记录内容版本、引擎、SDK、Rust 绑定、通信协议和资源格式，当前要求与本引擎精确匹配。逐文件校验之外，继续使用邮件和地图的固定流程及模型校验；健康检查和应用清单与编译时内容一致。

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

首批 CLI 未修改前端；后续桌面管理增加宿主命令，未新增引擎 IPC，通信协议保持 v15。完整资源目前指已有执行器需要的 20 个文件，包含两份锁定 OCR；战斗识别生成资源和咖啡只读预览不随本包分发。资源格式与执行器扩展时同步更新文件清单及兼容校验。

## 桌面版本管理

本轮接通桌面“运行资源”页，查看本机已安装版本、当前和上一选择，启用已安装版本或回滚。ZIP 继续用上面的原生命令安装；页面不接收文件路径、URL 或外部清单。完整包联网下载和签名发布继续待办。

页面使用桌面宿主保存的异步任务状态，离开页面再回来继续查询，不重新执行切换。刷新只读清单，不重复扫描模型；“清单可用”只表示清单与当前引擎兼容，启用前仍须完整核对文件。损坏或不兼容的版本显示原因，不静默忽略当前版本损坏。

切换先核对页面预览的选择修订，检查游戏任务、未保存结果、模型下载和持久输入恢复记录；运行或暂停中拒绝切换。目标完整校验通过后才退出旧引擎，确认退出后再保存选择。保存时再次核对修订，外部修改导致冲突时拒绝覆盖。失败保留原选择，并尝试重新连接引擎；重新连接失败单独说明，由用户手动重连。

切换后的引擎使用普通权限，不自动连接游戏或运行应用，需要时可手动提升权限。切换期间宿主拒绝启动、重连、提权及恢复等并发请求。退出桌面等待当前资源任务收尾，退出已开始时不再启动新引擎；强制关闭后不自动重放任务，下次重新读取磁盘选择。全部命令仍限定本地主窗口，桌面不加载 Maa DLL。

`catalog` 只读清单与选择修订，至多列出 64 个目录，保留缺失、损坏或不兼容的已选择版本。宿主先调用 `preview-change` 完整验证，再在旧引擎退出后调用 `apply-change`；两次均携带同一选择修订。校验期间定期查询旧引擎快照，维持托管连接。普通状态查询只读宿主保存的快照，不反复扫描磁盘。类型由 `od-core` 生成，页面不自行推断资源兼容性。

```powershell
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test desktop_runtime_resources --test runtime_resources -- --test-threads=1
npm --prefix zzz-od-test/test/zzz_auto/frontend test -- runtime-resources.spec.ts --reporter=line
```
