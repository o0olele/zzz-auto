# M3 运行资源包首批

本批把当前邮件、固定地图试运行、健康检查与应用清单组成独立版本的完整运行资源包。模型下载仍只准备来源缓存；运行资源包使用已转换并校验的资源。咖啡真实执行、战斗模型和其他未迁移应用不因此开放。

## 版本与启用规则

原生引擎 CLI 提供 `runtime-resources pack|download|check|install|status|activate|rollback`。本地 ZIP 可通过 CLI 或桌面导入，安装必须给出预期 ZIP SHA-256；显式 HTTPS 下载见下方说明，联网发现版本、桌面完整包下载及签名发布后续实现。包记录内容版本、引擎、SDK、Rust 绑定、通信协议和资源格式，当前要求与本引擎精确匹配。逐文件校验之外，继续使用邮件和地图的固定流程及模型校验；健康检查和应用清单与编译时内容一致。

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

### 桌面安装本地 ZIP

打开“运行资源”，点击“选择 ZIP 资源包”，填写发布方提供的 64 位小写 SHA-256，再点击“安装资源包并重连引擎”。取消文件选择保留已填内容，更换文件会清空旧校验值。安装前先用同一原生 CLI 完整检查包；无效包不退出正在连接的引擎。检查通过后确认旧引擎退出，再安装到独立版本目录并重连普通权限引擎。安装不修改当前或上一选择，需另行点击启用。安装期间复用版本切换的任务、模型、未保存结果及输入恢复检查，阻止并发启动；退出桌面等待收尾。页面离开后仍能查询终态，失败保留当前资源选择，重新连接失败单独说明。

文件选择器只选本地 ZIP，不改变进程工作目录；取消不创建任务。安装请求校验本机盘符绝对路径、扩展名及 SHA-256，拒绝网络共享、设备路径、相对路径、父目录跳转及备用数据流。仅本地主窗口可调用选择器和安装命令，不接收 URL 或自定义 Pipeline。安装再次核对 ZIP 哈希，检查后文件被替换或已有版本损坏时拒绝覆盖。

桌面“运行资源”页也可查看本机已安装版本、当前和上一选择，启用已安装版本或回滚。桌面完整包联网下载和签名发布继续待办。

页面使用桌面宿主保存的异步任务状态，离开页面再回来继续查询，不重新执行安装或切换。安装完成自动刷新版本列表；刷新只读清单，不重复扫描模型。“清单可用”只表示清单与当前引擎兼容，启用前仍须完整核对文件。损坏或不兼容的版本显示原因，不静默忽略当前版本损坏。

切换先核对页面预览的选择修订，检查游戏任务、未保存结果、模型下载和持久输入恢复记录；运行或暂停中拒绝切换。目标完整校验通过后才退出旧引擎，确认退出后再保存选择。保存时再次核对修订，外部修改导致冲突时拒绝覆盖。失败保留原选择，并尝试重新连接引擎；重新连接失败单独说明，由用户手动重连。

切换后的引擎使用普通权限，不自动连接游戏或运行应用，需要时可手动提升权限。切换期间宿主拒绝启动、重连、提权及恢复等并发请求。退出桌面等待当前资源任务收尾，退出已开始时不再启动新引擎；强制关闭后不自动重放任务，下次重新读取磁盘选择。全部命令仍限定本地主窗口，桌面不加载 Maa DLL。

`catalog` 只读清单与选择修订，至多列出 64 个目录，保留缺失、损坏或不兼容的已选择版本。宿主先调用 `preview-change` 完整验证，再在旧引擎退出后调用 `apply-change`；两次均携带同一选择修订。校验期间定期查询旧引擎快照，维持托管连接。普通状态查询只读宿主保存的快照，不反复扫描磁盘。类型由 `od-core` 生成，页面不自行推断资源兼容性。

```powershell
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test desktop_runtime_resources --test runtime_resources -- --test-threads=1
npm --prefix zzz-od-test/test/zzz_auto/frontend test -- runtime-resources.spec.ts --reporter=line
```

2026 年 10 月 16 日本轮验证使用默认构建目录。重新获取的测试仓主分支不含此前 `test/zzz_auto/` 工程，因此本轮新建配套工程并添加 `desktop_runtime_install` 与 `runtime-install.spec.ts`，没有把旧回归记为重新通过。Rust 用真实生成资源打包和隔离账号目录、无 SDK 的引擎验证两条综合用例，覆盖参数拒绝、坏哈希不退出引擎、安装与重连、重复安装、保持当前选择、恢复记录拦截、安装阶段失败保留旧文件、临时文件清理及退出等待。6 个前端用例覆盖浏览器禁用、文件选择与取消、校验值、安装提交、切页恢复、错误和断线、迟到响应及主题切换。另通过默认引擎构建、前端构建、Rust 格式和全 workspace Clippy。Windows 原生文件对话框的人工操作及发布包中的完整安装仍待实机验收；自动化界面测试模拟选择器返回值。

```powershell
node scripts/prepare-engine.mjs
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target --locked --test desktop_runtime_install -- --test-threads=1
Push-Location zzz-od-test/test/zzz_auto/frontend
npm test -- --reporter=line
Pop-Location
```

新增命令仅作用于桌面宿主，未新增引擎 IPC，协议仍为 v15；生成类型增加本地安装请求。

## 原生 HTTPS 下载

显式 CLI `runtime-resources download --url <HTTPS> --bytes <预期字节数> --sha256 <预期 ZIP 哈希> --output <ZIP>` 复用模型下载的原生传输、系统代理、证书校验和可取消网络等待。输入元数据先校验；文件下载到输出目录内的临时位置，限制长度并核对哈希，再执行完整 ZIP、兼容性、逐文件及固定流程检查，通过后才保存最终 ZIP。不自动安装或启用，不加载 SDK，不占用游戏控制权。预期大小为 1..536870912 字节，哈希须为 64 位小写十六进制；地址拒绝 HTTP、用户名、密码及 URL 片段。HTTPS 重定向至多十次且禁止 HTTP 降级，连接超时为 15 秒，总超时为 120 秒。

已有输出只允许完整校验后复用；无效文件保留并报错，不覆盖。并发下载最终使用不可覆盖的原子发布，取消和失败清理本次临时内容，强制终止遗留内容不作为完整文件。进度写入标准错误，成功结果写入标准输出。正式发布地址、签名清单及桌面联网发现继续待办；本轮不把显式 CLI 地址当作已受信任的发布源。

`pack` 和 `download` 的成功结果增加 `archive_bytes`，前者给出发布 ZIP 的预期字节数；下载者应从发布方取得大小和哈希。`download` 还返回 `reused`、清单哈希、文件路径及包清单，不在结果或进度中回显下载 URL。输出目录可以有空格与中文，已有的完整输出可直接复用且不联网。网络错误也不附带 URL，避免在日志中输出临时下载凭据。以下占位值须替换为实际发布信息：

```powershell
cargo run -p zzz-engine --locked -- runtime-resources download --url <发布方HTTPS地址> --bytes <预期ZIP字节数> --sha256 <预期ZIP哈希> --output artifacts/runtime-download.zip
cargo run -p zzz-engine --locked -- --data <绝对数据目录> runtime-resources install --archive artifacts/runtime-download.zip --sha256 <预期ZIP哈希>
```

Ctrl+C 在网络等待、下载块、ZIP 条目和文件哈希块之间请求取消；已通过检查且保存为最终 ZIP 后视为完成。邮件、地图的固定语义复验在调用前后检查取消，不承诺中断该次复验。下载完成后可在桌面选择此 ZIP，安装后仍另行启用。

本轮独立回归新增 7 个下载用例，覆盖元数据拒绝、超长 / 截断 / 坏哈希 / 坏 ZIP、流异常、取消清理、既有文件保留、真实生成资源的下载流到安装、无 SDK CLI 复用、保存冲突、本机 TLS 等待取消，以及共用传输后的锁定模型下载与离线复用。既有 2 个桌面安装用例继续通过。测试使用受控文件流和本机端口，没有把正式 HTTPS 资源包发布或签名登记为已验收；本机 TLS 用例需要在 Windows 沙箱外运行，否则可能返回安全凭证不可用。

```powershell
cargo build -p zzz-engine --locked
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target --locked --test runtime_download --test desktop_runtime_install -- --test-threads=1
cargo clippy --workspace --all-targets --locked -- -D warnings
```
