# 绝区零一条龙 · Rust 原型

按 [路线图](docs/ROADMAP.md) 迁移到 Rust、Tauri 2 和 MaaFramework。当前交付为 M0 清单与 M1 集成原型：能加载真实 SDK、运行离线原生 Pipeline、连接游戏窗口、截图、查看诊断，并通过独立测试仓验证协议与资源转换。30 个游戏应用仍显示“待迁移”，尚不能代替旧一条龙。

界面采用 React、TypeScript、Vite 与 Fluent UI React v9；自动化运行在独立 Rust 进程中。运行程序不启动 Python 或 Node 服务。前端构建需要 Node，Rust 和 MSVC 用于开发；正式发行的完整验收属于 M7。

## 启动

开发环境：Windows x64、Visual Studio C++ Build Tools、Rust 1.95.0、Node.js 24 和 WebView2。首次 Cargo 命令会按 `rust-toolchain.toml` 获取固定工具链。

在工程根目录执行：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/setup-sdk.ps1
npm --prefix apps/desktop ci
cd apps/desktop
npm run tauri dev
```

首次启动自动构建并启动随附引擎。主窗口关闭后隐藏到托盘；托盘“停止并退出”等待引擎收尾。截图入口只连接 `ZenlessZoneZero.exe`，桌面默认采用 FramePool、前台输入方法与 CPU；游戏客户区需要为 1920×1080。总览与原生诊断提供截图方式选择，支持 FramePool、PrintWindow、DXGI、ScreenDC 和 GDI；诊断页更改方式后点击“获取截图”会先重新连接控制器。疑似黑帧保留预览并提示，同时记录平均亮度和暗像素比例；加载过场可能是真实黑屏，不能据此自动判定失败。DXGI / ScreenDC 需要游戏完整可见且未被遮挡。连接和截图会调用真实 Maa 控制器；健康检查的 Pipeline 不产生游戏按键或点击。

只查看界面可以执行 `npm run dev`。浏览器显示“界面预览”，原生功能保持禁用。

## CLI

下列命令在工程根目录执行，不需要 Tauri。SDK 必须先完成安装。

```powershell
cargo run -p zzz-engine -- doctor
cargo run -p zzz-engine -- applications
cargo run -p zzz-engine -- windows
cargo run -p zzz-engine -- probe --fixture zzz-od-test/screens/邮件/列表.webp
cargo run -p zzz-engine -- probe --fixture zzz-od-test/screens/邮件/列表.webp --cancel-after-ms 100
```

`capture --hwnd <十六进制句柄> --output <PNG>` 获取游戏截图，默认 `frame-pool`，`--method` 可选择 `gdi`、`print-window`、`frame-pool`、`dxgi`、`screen-dc`，结果包含实际方式和黑帧诊断。`recognize` 接收 `--fixture`、`--image`、`--kind` 和 `--params <JSON>`，调用 Maa 独立识别；默认 CPU，`--directml <设备编号>` 仅供受控模型验证。资源包可通过全局 `--resources <绝对目录>` 指定。CLI 文件入口只用于开发者显式调用，桌面不开放任意文件和 Pipeline 执行。

桌面“原生诊断 → 真实输入诊断”提供前台 / 后台键盘和 Xbox / DS4 数字按键。连接游戏后填写按键、按住时间和启动延时，勾选真实输入确认，再点击“开始输入诊断”。修改参数、启动、错误或失联后需重新确认；停止按钮等待原生队列与输入释放后显示终态。默认延时三秒，按住最长两秒；鼠标、摇杆和扳机尚未开放。

CLI `input-probe` 使用同一实现，指定 `--hwnd`、`--key`、`--hold-ms`、`--mode` 和 `--allow-input`。`--start-delay-ms` 默认为 3000；`--cancel-after-ms` 从探针开始计时，包括启动延时。Ctrl+C 请求收尾后退出。程序返回按下投递、按下完成、实际持有、释放和取消收尾耗时；这些是控制器完成数据，游戏响应仍需观察。示例：

```powershell
cargo run -p zzz-engine -- input-probe --hwnd <十六进制句柄> --mode background --key 87 --hold-ms 1000 --start-delay-ms 3000 --cancel-after-ms 3200 --allow-input
```

键盘使用 Win32 VK；手柄参数、计时边界和未完成的实机检查见 [验收记录](docs/migration/acceptance.md)。首轮未提供权限自动提升、声音闪避、暂停恢复、HTTP、MCP、旧配置导入与游戏应用运行。

## 工程与数据

六个 Rust crate 分别负责通用核心、协议、平台、Maa 适配、引擎与迁移工具；桌面位于 `apps/desktop/`。详见 [首轮实现说明](docs/develop/implementation.md)。

开发 CLI 使用工程 `data/`；桌面使用 Tauri 的用户应用数据目录。引擎记录任务终态与有容量限制的日志；截图按需返回 PNG，不自动写用户截图到日志。SDK 缓存位于 `sdk/maa/bin/`，通过固定清单校验所有运行 DLL，并限制加载目录。版本和压缩包 SHA-256 的来源是 [sdk.lock.json](sdk.lock.json)，原生文件清单见 [sdk-manifest.json](resources/sdk-manifest.json)。

## 迁移与验证

应用清单只维护在 [applications.json](resources/catalog/applications.json)；所有 ID、名称和源工厂来自参考项目。界面、插件、资源及公开接口来源见 [inventory.json](docs/migration/inventory.json)。工具只读取参考目录，不导入或执行旧 Python 模块。

```powershell
cargo run -p od-tools -- inventory --reference E:/Code/ZenlessZoneZero-OneDragon
cargo run -p od-tools -- baseline --reference E:/Code/ZenlessZoneZero-OneDragon --test-root zzz-od-test
cargo run -p od-tools -- convert-screen --source E:/Code/ZenlessZoneZero-OneDragon/assets/game_data/screen_info/email.yml --templates E:/Code/ZenlessZoneZero-OneDragon/assets/template --output resources/generated/email
cargo run -p od-tools -- types
```

区域转换保留 1080p 坐标、识别节点与导航元信息，报告 OCR LCS、模板掩码和颜色过滤差异。生成文件不手改，尚未验收的节点不能直接用于领取流程。原生新 Pipeline 放在 `resources/pipeline/`。

测试代码在独立 `zzz-od-test/test/zzz_auto/`，主工程不含测试模块。当前本机已有配套 Rust 与前端测试；新环境需要获取包含这些改动的配套测试分支，不能仅克隆旧测试仓主分支就假定新测试存在。

```powershell
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked -- --test-threads=1
cd zzz-od-test/test/zzz_auto/frontend
npm ci
npm test
```

前端测试使用本机 Edge；会自动启动或复用 Vite。返回根目录后执行 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate.ps1`，检查格式、构建、Clippy 和配套回归。整个新验证链不依赖 Python。

## 构建

跨电脑测试包执行 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package-portable.ps1`。脚本构建 release 桌面与引擎，校验并复制 Maa DLL、VC++ x64 运行库、微软签名的 WebView2 联网安装器、资源与许可证，生成 `artifacts/` 下的 ZIP、文件清单和 SHA-256。`-SkipBuild` 仅用于已经完成同一份源码 release 构建的打包步骤。

解压整个包后运行 `zzz-desktop.exe`。包内 `portable.flag` 使程序从自身目录定位引擎和资源，数据保存在包内 `data/`，不依赖启动时的工作目录。WebView2 缺失时可手动运行包内安装器；测试包不含模型和开发环境，也不带本机用户数据。

在 `apps/desktop/` 执行 `npm run tauri build`，配置会构建 release 引擎、生成协议、构建 Web 资源，再打包 NSIS。`npm run tauri build -- --debug --no-bundle` 用于本机桌面原型构建；普通 debug 版本的引擎与资源定位到当前工程，不能作为发行包。当前已提供便携测试 ZIP；更新签名、模型完整包、便携版自动更新、资源回滚与干净系统验收尚未实现。

代码与参考资源遵循 GPL-3.0-or-later；MaaFramework / 官方 Rust 绑定为 LGPL-3.0。SDK 保留独立动态库，发行时保留其许可证和对应源码来源。依赖许可证与来源见 [第三方说明](docs/migration/third-party.md)。
