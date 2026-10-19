# M3 账号游戏启动

按账号保存游戏可执行文件路径、是否使用启动参数、窗口化 / 全屏、无边框和显示器序号。启动参数固定为 1920×1080，与当前引擎运行基准一致。旧配置缺少对象时只读补默认值，新账号路径为空，复制和备份恢复保留设置。

账号与应用组页负责编辑，总览提供当前账号的手动启动入口。必须先保存并确认当前账号与实际路径；启动请求复验账号和配置修订。正在运行、原生请求、未保存结果、输入恢复及结束后动作期间拒绝启动。已存在游戏进程或本引擎提交的游戏仍存活时拒绝重复启动。

原生启动只接受本地绝对路径中的 `ZenlessZoneZero.exe`，重新检查真实文件，工作目录为可执行文件目录；不经过命令解释器，不提供自定义命令或高级参数。返回进程编号仅表示 Windows 接受启动，用户仍需手动登录并连接游戏，应用组不会自动开始。关闭桌面不结束手动启动的游戏。

启动前核对真实文件名与 Windows x64 PE 文件头；同一交互会话的引擎在启动检查期间互斥，进程尚无窗口时也阻止重复启动。只持有进程查询句柄，不提供强制终止。Windows 按程序声明处理授权，等待期间请在 Windows 提示中确认或取消；启动请求最多等待 120 秒，超时后先检查游戏状态再重连，不自动重试。请求启动会取消尚在等待上一任务完成的结束后动作，已经进入倒计时或正在执行时保持拦截。

依据：[ShellExecuteExW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shellexecuteexw)、[SHELLEXECUTEINFOW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-shellexecuteinfow)。

导入转换旧 `game_account.yml` 的路径及账号级 `game.yml` 中的已知选项。其他分辨率、未知枚举、高级参数和 HDR 行为明确提示兼容范围并归档原文，不自动执行。导入不要求旧游戏在本机仍存在，启动时才检查文件。导入、恢复和编辑展示路径及参数，避免恢复后使用未核对路径。

验证使用隔离目录、模拟启动执行器和原生测试程序，不启动用户游戏、不修改用户数据或 HDR。真实游戏启动、Windows 授权、参数实际效果与正式包环境待实机验收。

## 使用

1. 在“账号与应用组”选择编辑账号，在“游戏启动设置”选择实际 `ZenlessZoneZero.exe` 并保存。可开启固定 1080p 参数，设置窗口模式、无边框和显示器。
2. 将对应账号设为当前账号，返回总览核对显示的账号、路径及参数，勾选启动确认后点击“启动当前账号游戏”。
3. 手动完成登录，从总览扫描并连接窗口，再进入“一条龙运行”预览并确认任务。保存、导入和配置恢复都不会自动启动。

配置保存为 `instances[].game_launch`。缺少整个对象时使用空路径、关闭参数、窗口化、非无边框及显示器 1；对象存在时必须包含全部字段。配置校验不依赖游戏安装，启动时才检查文件；清空路径可关闭本账号的启动入口。协议为 v27、导入预览 v7；旧资源 schema 1 / 协议 15..26、schema 2 / 协议 16..26 继续兼容。

## 自动验证

新增 12 项 Rust 回归覆盖缺省及严格字段、路径与参数、账号隔离及修订、复制、备份与离线恢复、旧设置转换及原文保留、启动确认、失败及取消等待后的互斥、真实桌面管道和原生 Windows 启动。原生成功用例复制独立测试程序到带中文、空格及特殊字符的目录，确认参数、工作目录、进程查询、重复启动拦截和自然退出，不调用用户游戏。

新增 7 项界面回归覆盖按账号保存、明确确认、选择取消和错误、冲突重读、草稿隔离、重复点击、运行与记录拦截、结束后倒计时、断线旧响应、路径错误及 960 像素明暗外观；连同相邻账号、运行、备份、离线恢复和桌面共 88 项通过。截图已检查，真实 WebView2 和文件选择器另行验收。

连同相邻配置、账号、咖啡设置、每日计划、结束后动作及资源兼容专项，共 86 项 Rust 回归通过。资源检查覆盖基础包协议 15..27、咖啡包协议 16..27，继续拒绝未知协议及咖啡包协议 15。生产前端、配套引擎及嵌入静态前端的原生 debug 桌面构建、主 workspace 与新增测试严格 Clippy、格式及差异检查通过；保留已有前端大于 500KB 单块提示，未重打包便携 ZIP。

```powershell
node scripts/prepare-engine.mjs
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test game_launch --test after_done --test account_config --test instance_management --test config_backups --test config_recovery --test coffee_settings --test daily_runs -- --test-threads=1
npm --prefix zzz-od-test/test/zzz_auto/frontend test -- game-launch.spec.ts accounts.spec.ts one-dragon-run.spec.ts config-backups.spec.ts offline-config-recovery.spec.ts desktop.spec.ts --reporter=line
npm --prefix apps/desktop run build
cargo build --locked -p zzz-desktop --features custom-protocol
```
