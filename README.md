# 绝区零一条龙 · Rust 原型

按 [路线图](docs/ROADMAP.md) 迁移到 Rust、Tauri 2 和 MaaFramework。M0 清单与基线已完成，M1 集成原型待验证，M2 **主流程通过，边界待验收**，M3 进行中。邮件大世界和菜单起点已获实机测试通过反馈，详见[邮件试运行说明](docs/develop/mail-pipeline.md)；M3 已实现[账号、配置导入与应用组基础](docs/develop/account-config.md)，并接通[应用组邮件试运行与账号记录](docs/develop/group-runs.md)。暂停、异常收尾等场景继续验收，30 个游戏应用的正式可用标记仍待验收，尚不能代替旧一条龙。

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

`capture --hwnd <十六进制句柄> --output <PNG>` 获取游戏截图，默认 `frame-pool`，`--method` 可选择 `gdi`、`print-window`、`frame-pool`、`dxgi`、`screen-dc`，结果包含实际方式和黑帧诊断。`recognize` 接收 `--fixture`、`--image`、`--kind` 和 `--params <JSON>`，调用 Maa 独立识别；默认 CPU，`--directml <设备编号>` 仅供受控模型验证。`--kind OcrText` 提供精确 / 包含 / LCS 文字匹配、单行识别与全图坐标，参数及兼容范围见 [OCR 业务调用适配](docs/develop/recognition-validation.md#ocr-业务调用适配)。资源包可通过全局 `--resources <绝对目录>` 指定。CLI 文件入口只用于开发者显式调用，桌面不开放任意文件和 Pipeline 执行。

桌面“原生诊断 → 真实输入诊断”提供前台 / 后台键鼠、Xbox / DS4 数字按键、左右摇杆和左右扳机。连接游戏后选择输入方式与动作，填写参数、按住时间和启动延时，勾选真实输入确认，再点击“开始输入诊断”。鼠标被游戏锁定时，可先按 Win 或 Alt+Tab 回到工具；启动后在延时期间切回游戏。修改参数、启动、错误或失联后需重新确认；停止按钮等待原生队列与输入释放、摇杆回中或扳机归零后显示终态。默认延时三秒，按住最长两秒。

鼠标可选择左、右、中或两个侧键；坐标以 1920×1080 游戏客户区左上角为原点，X 为 0..1919，Y 为 0..1079。前台方式会移动鼠标到该位置。摇杆 X / Y 为 -32768..32767，X 正值向右、Y 正值向上；扳机力度为 1..255。摇杆和扳机仅用于 Xbox / DS4 方式。鼠标输入前确认截图尺寸，避免截图失败时点到错误位置；数字按键、鼠标与模拟量共用运行槽与停止规则。拖动、滚轮和连续模拟量变化尚未实现。

前台键鼠在延时结束时要求游戏处于前台。工具会检测游戏与引擎权限；游戏权限更高时，先停止任务并等待收尾，再到总览点击“提升引擎权限”，完成 Windows 授权后重新连接游戏。只提升引擎，桌面保持当前权限；取消授权保留当前连接，切换不会自动运行任务。启动器正常启动无法直接证明游戏权限与工具匹配。用户已反馈前台键盘响应、持有期间停止与无残留动作，以及游戏后台时的 Xbox 数字按键正常；两种后台键盘方式均未生效，暂保留为诊断选项。0.1.4 鼠标按键及 Xbox / DS4 摇杆与扳机已有基础实机通过反馈；具体前后台组合、DS4 数字按键、完整任务与异常收尾等场景继续验收，具体范围见[验收记录](docs/migration/acceptance.md)。

CLI `input-probe` 使用同一实现，指定 `--hwnd`、`--key`、`--hold-ms`、`--mode` 和 `--allow-input`。`--start-delay-ms` 默认为 3000；`--cancel-after-ms` 从探针开始计时，包括启动延时。Ctrl+C 请求收尾后退出。程序返回按下投递、按下完成、实际持有、释放和取消收尾耗时；这些是控制器完成数据，游戏响应仍需观察。示例：

```powershell
cargo run -p zzz-engine -- input-probe --hwnd <十六进制句柄> --mode background --key 87 --hold-ms 1000 --start-delay-ms 3000 --cancel-after-ms 3200 --allow-input
```

CLI 键盘使用 Win32 VK，可选 `--mode foreground-legacy` 或 `background-post` 对照 Maa 的兼容方式；手柄参数、计时边界和未完成的实机检查见 [验收记录](docs/migration/acceptance.md)。邮件暂停 / 继续见[试运行说明](docs/develop/mail-pipeline.md)，旧配置的账号与应用组导入见[M3 使用说明](docs/develop/account-config.md#使用)。声音闪避、HTTP、MCP、其余设置转换及多应用执行继续迁移。

新动作使用 `--action mouse|stick|trigger`，其余停止、确认与计时参数相同。示例中的句柄需要替换为 `windows` 命令返回的游戏句柄：

```powershell
cargo run -p zzz-engine -- input-probe --hwnd <十六进制句柄> --action mouse --button left --x 960 --y 540 --hold-ms 500 --allow-input
cargo run -p zzz-engine -- input-probe --hwnd <十六进制句柄> --mode xbox --action stick --side left --x 16000 --y 0 --hold-ms 2000 --cancel-after-ms 4000 --allow-input
cargo run -p zzz-engine -- input-probe --hwnd <十六进制句柄> --mode ds4 --action trigger --side right --value 128 --hold-ms 500 --allow-input
```

桌面“通用设置”可保存默认截图方式、邮件超时和启动 / 继续延时。设置由 Rust 与账号配置一同持久保存，应用组预览显示实际参数，开始时由引擎核对修订并读取。原生诊断可临时调整；完整用法与旧截图配置导入范围见[通用运行设置](docs/develop/general-settings.md)。账号区服、按日跳过及游戏日过期处理见[应用组每日计划](docs/develop/daily-runs.md)。

当前协议为 v17，桌面与引擎必须使用同一版本。托管引擎通过桌面持有的私有管道回连，支持引擎单独提权、受控输入 Pipeline、地图与咖啡点单试运行、应用组邮件试运行、账号历史及模型资源任务。旧数字按键命令继续支持 `--key`，已保存的旧运行记录可以读取；新运行记录使用明确的动作结构。

咖啡已根据澄辉坪真实截图接通“原生诊断 → 咖啡点单试运行”：手动交互到点单选项，选择汀曼特调或浓缩咖啡，自动等待喝完、确认电量和对应增益并返回大世界。先更新桌面与引擎；若当前启用旧邮件地图包，须安装并启用包含咖啡的新资源包。自动测试已通过，实机待验收，结果只保存为诊断记录；应用组仍只运行邮件。步骤与资源校验值见[咖啡点单与顺序调度](docs/develop/coffee-and-groups.md)。

咖啡前置的“澄辉坪 → 汀曼咖啡”已接入“原生诊断 → 地图传送试运行”，2026 年 10 月 17 日获正常传送流程实机通过反馈，暂停、停止及异常恢复等边界继续待验收。先按 N 打开普通地图，确认本次路线后启动，在延时期间切回游戏；暂停保留检查点，继续前重新识别，停止等待输入释放。地图与其他任务共用运行槽及崩溃输入恢复，结果只保存为诊断记录。`map-bundle` 生成 v2 资源，`map-bundle-check` 校验资源，`map-inspect` 只读检查留档；命令、资源准备及实机待验项见[地图传送说明](docs/develop/map-transport.md)。

桌面在发送真实输入探针或输入 Pipeline 前，把恢复计划写入数据目录的 `input-recovery.json`。引擎崩溃、应答丢失或收尾未确认时会阻止重连，请点击“恢复遗留输入”：确认旧进程退出后，由独立进程只发送对应释放；原管理员引擎可能需要再次授权 Windows。恢复后先确认游戏状态，再手动重连，不自动重放任务。桌面重启也保留拦截；失败或取消授权不会清除记录。当前覆盖桌面托管的单次输入、受控 Pipeline、邮件及地图试运行，CLI 和未来业务 Pipeline 须单独接入。真实游戏、跨权限释放与 ViGEm 卸载仍需实机验收，见[实现说明](docs/develop/implementation.md#m1-崩溃后的独立输入恢复)。

实机带输入 Pipeline 在“原生诊断 → 真实输入诊断 → 执行方式”选择 **Maa Pipeline**。支持键盘 / Xbox / DS4 数字键及鼠标按键；设置按住、启动延时和 Pipeline 超时，勾选输入确认后运行，再在延时期间切回游戏。流程按下一个键，经不超过 25ms 的无输入间隔等待后释放；停止或超时会等待原生队列并显式释放。超时不包含启动延时、截图准备和控制器重建，不能作为强制终止。摇杆与扳机使用“直接输入”；游戏响应及停止残留需实机观察。完整边界和复测入口见[输入 Pipeline 说明](docs/develop/input-pipeline.md)。

## 离线模型对照

桌面“模型资源”页可检查固定模型、准备所选项、离线复用、查看进度及取消任务，缓存位于引擎数据目录的 `model-cache/`。任务离开页面后继续，退出或失联时取消；下载完成暂不切换游戏使用的资源包。CLI 的 `cargo run -p zzz-engine --locked -- model-resources status` 只读检查开发缓存，`model-resources prepare` 下载并校验，`--offline` 只用本地缓存，`--model ppocrv5` 可选择模型，`--cache` 可指定与桌面相同的目录。完整用法与范围见[模型资源管理](docs/develop/model-resources.md)。桌面“运行资源”可选择本地 ZIP、填写发布方 SHA-256 后检查并安装，也可填写显式 HTTPS 地址、预期字节数和 SHA-256 下载并完整校验，支持进度、取消、缓存复用和切页恢复。下载后点击安装，再另行启用或回滚；安装与切换会重连普通权限引擎。CLI `runtime-resources download` 保留相同下载能力；联网版本发现与签名发布继续待办，详见[运行资源包](docs/develop/runtime-resources.md)。

准备固定的旧版 PP-OCRv5、闪光及两种 YOLO 模型，再运行同一批固定图的 CPU / DirectML 对照：

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/setup-models.ps1
cargo run -p zzz-engine -- --resources resources/generated/recognition recognition-check --cases zzz-od-test/test/zzz_auto/fixtures/recognition/cases.json --fixture-root zzz-od-test --output artifacts/recognition-report.json --directml 0
```

`models.lock.json` 固定四个官方发布包及使用文件的 SHA-256。模型缓存位于 `sdk/models/`，转换资源位于 `resources/generated/recognition/`；已有 ZIP 缓存时可为安装脚本加 `-Offline`。转换保持字典字节和标签索引，附带已发现的预处理差异，参考仓不会被修改。当前便携测试包仍不包含这些模型和新增命令，需要从本工程构建引擎。

验证仅进行离线识别，先 CPU 后 DirectML，每案默认预热 5 次、正式测量 30 次。省略 `--directml` 时只跑 CPU；`--warmup` 和 `--samples` 可调整轮数。图像准备、首次识别与正式样本分别记录，正式样本使用同一份已解码帧。JSON 报告包含文字 / 类别 / 框 / 分数、预期检查、结果稳定性和 P50 / P95；原生未命中保留为未命中，错误单独记录。能力调用、预期、稳定性、设备对照或 DirectML 配置证据未通过时，报告仍写出，命令以失败状态退出；已登记的旧版兼容性缺口继续标记 pending。

原始闪光 / YOLO 的 CPU 推理默认由 ORT 选择线程数，并保持关闭自旋；DirectML 继续使用原设置。离线诊断可用 `--raw-cpu-threads 0..64` 对照，0 表示 ORT 默认，1 用于复现之前的单线程。新增完整 BGR 帧到标准结果的统一计时；7 个 YOLO 案例的同机 release CPU P95 已在此范围内达标，完整游戏链路仍待验收。命令与范围见[CPU 线程与统一计时说明](docs/develop/recognition-validation.md#原始推理-cpu-线程与统一计时)。

当前对照不代表旧版完全兼容：Maa 闪光分类再次 softmax，YOLO 的补边和插值与旧版不同；旧版实际输出、正样本和同机性能基线还需补齐。报告明确区分设备请求与可取得的 DirectML 配置日志证据。具体约定见[模型对照说明](docs/develop/recognition-validation.md)，阶段结论以[验收记录](docs/migration/acceptance.md)为准。

闪光适配入口 `LegacyFlashClassify` 已补旧版的等比缩放、右下补边和概率判定，继续由 Maa 推理。先重新生成模型资源，再运行原生 / 适配路径的固定图对照：

```powershell
cargo run -p od-tools --locked -- models
cargo run -p zzz-engine --locked -- --resources resources/generated/recognition recognition-check --cases zzz-od-test/test/zzz_auto/fixtures/recognition/flash-cases.json --fixture-root zzz-od-test --output artifacts/flash-report.json --directml 0
```

生成的 logits 模型与原模型分别校验，资源清单升级为 v2；旧生成目录需要重新转换。正式采样中的 `prepare` 包括每轮缩放、补边与 Maa 图像缓存，`total` 包括适配和推理；固定图解码仍单列。Maa 分数的六位小数会使约 0.8999998 变为 0.9，适配入口对距阈值不超过 0.000001 的值返回精度错误。真实红光 / 黄光、旧版实际输出和该精度边界仍未验收，详见[闪光适配方案](docs/develop/recognition-validation.md#闪光分类适配方案)。当前便携包不包含本轮新增实现。

新增 `LegacyFlashRaw` 通过 SDK 内已有 ONNX Runtime 直接读取 float32 概率，补上 JSON 分数舍入缺口。原模型、Maa logits 和原始分数三条路径可用同一批图对照：

```powershell
cargo run -p zzz-engine --locked -- --resources resources/generated/recognition recognition-check --cases zzz-od-test/test/zzz_auto/fixtures/recognition/raw-flash-cases.json --fixture-root zzz-od-test --output artifacts/raw-flash-report.json --directml 0
```

原始入口使用固定原模型名称 `yolov8n-640-flash-20250921.onnx`，原标签顺序及 ROI / 阈值参数与前一入口相同。按参考 NumPy 2 规则在 float32 下比较阈值，支持相等边界；报告保留三类概率、实际阈值、原生库与模型哈希。`session_setup` 只记录首次模型加载，`enqueue` 为 0，正式 `total` 包括预处理与同步推理，不能直接混入 Maa 队列时延。DirectML 注册失败会报错。未新增、替换或下载原生 DLL，旧 Maa 精度错误入口继续保留；语料和停止能力的验收边界见[原始浮点输出方案](docs/develop/recognition-validation.md#原始浮点输出方案)。

YOLO 适配入口 `LegacyYoloDetect` 使用固定原模型和 SDK 原始张量，补上旧版的右下补边、INTER_LINEAR、类别过滤、严格置信度、按类别 NMS、越界框及迷失之地头像遮罩。先重新生成资源，新增的两份标签 CSV 也需要哈希校验，再运行原生 / 适配对照：

```powershell
cargo run -p od-tools --locked -- models
cargo run -p zzz-engine --locked -- --resources resources/generated/recognition recognition-check --cases zzz-od-test/test/zzz_auto/fixtures/recognition/yolo-cases.json --fixture-root zzz-od-test --output artifacts/yolo-report.json --directml 0
```

参数为固定 `model`、默认 `threshold=0.6`、`iou=0.5` 与可选 `label_list` / `category_list`；标签顺序从已校验 CSV 读取，当前只接收完整 1080p 帧。空洞模型实际输入为 736×416，迷失之地为 736×736。分数保留原始 float32，框同时包含旧版 xyxy 和对照用 xywh。相同类别、相同分数的重叠框返回排序歧义错误。此入口仍用于离线诊断，同机旧输出与性能待验收，详见 [YOLO 适配方案](docs/develop/recognition-validation.md#yolo-适配方案)。现有便携包未更新。

CLI `recognize` 的 `LegacyFlashRaw` / `LegacyYoloDetect` 支持 Ctrl+C 和 `--cancel-after-ms`。定时从识别调用开始，包含图像读取和首次原始模型加载；`0` 表示开始前取消。取消先于结果提交时丢弃结果，等待原生调用及数据收尾后输出 `status=cancelled`、`error.code=raw_recognition_cancelled`、停止耗时与原生错误，退出码为 1。DirectML 可能完成已提交的计算再返回，模型加载也不能立即中断。其他识别类型不支持此取消参数，具体边界见[原始推理的取消](docs/develop/recognition-validation.md#原始推理的取消)。例如先把 `{"model":"yolov8n-640-flash-20250921.onnx"}` 保存到 `artifacts/raw-flash-params.json`，再执行：

```powershell
cargo run -p zzz-engine --locked -- --resources resources/generated/recognition recognize --fixture zzz-od-test/screens/战斗画面/默认.webp --image zzz-od-test/screens/战斗画面/默认.webp --kind LegacyFlashRaw --params artifacts/raw-flash-params.json --cancel-after-ms 100
```

## 工程与数据

已转换的邮件、地图、咖啡及基础运行资源可由原生 CLI 打成独立版本包，通过显式 HTTPS 下载或选择本地 ZIP 安装。桌面“运行资源”页可刷新已安装版本，预检后退出并重启引擎以启用或回滚；同样保留离线 CLI 入口。用法见[运行资源包](docs/develop/runtime-resources.md)。联网版本发现与签名发布继续待实现。

已增加旧版闪光、OCR 和 YOLO 实际输出采样及纯 Rust 的 `recognition-compare` 命令，支持核对同一批固定图的概率、候选、文字、框与分数；OCR 同时比较过滤前结果。旧版采样环境单独放在忽略的 SDK 缓存中，仅用于迁移取证；新程序和正式回归不依赖 Python。命令见[闪光旧版输出说明](docs/develop/recognition-validation.md#旧版实际输出与自动比较)及 [OCR / YOLO 对照说明](docs/develop/recognition-validation.md#ocr-与-yolo-旧版对照)。比较结果与人工标注分别报告，不能把新旧共有的漏检当成人工预期通过。

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

带输入 Pipeline 的停止验证使用离线控制器与真实 Maa 队列，覆盖阻塞动作、取消、超时和释放失败重试；探针不连接游戏。输入清单与安全暂停边界见[实现说明](docs/develop/implementation.md#m1-带输入-pipeline-停止验证)。

测试代码在独立 `zzz-od-test/test/zzz_auto/`，主工程不含测试模块。当前本机已有配套 Rust 与前端测试；新环境需要获取包含这些改动的配套测试分支，不能仅克隆旧测试仓主分支就假定新测试存在。

Rust 模型回归需要固定 Maa SDK 和 `sdk/models/` 缓存，先执行 `scripts/setup-sdk.ps1` 与 `scripts/setup-models.ps1`；已有完整 ZIP 缓存时模型脚本可用 `-Offline`。验证脚本不自动下载模型。

```powershell
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked -- --test-threads=1
cd zzz-od-test/test/zzz_auto/frontend
npm ci
npm test
```

前端测试使用本机 Edge；会自动启动或复用 Vite。返回根目录后执行 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/validate.ps1`，检查格式、构建、Clippy 和配套回归。整个新验证链不依赖 Python。

## 构建

跨电脑测试包执行 `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/package-portable.ps1`。脚本构建 release 桌面与引擎，生成并校验邮件资源，复制 Maa DLL、VC++ x64 运行库、微软签名的 WebView2 联网安装器、资源与许可证，生成 `artifacts/` 下的完整客户端 ZIP、邮件资源补丁、文件清单和 SHA-256。构建机须已准备参考仓及固定模型缓存；参考仓不在相邻目录时设置 `ZZZ_AUTO_REFERENCE`。`-SkipBuild` 仅用于已经完成同一份源码 release 构建的打包步骤，仍执行邮件资源校验。

解压整个包后运行 `zzz-desktop.exe`。包内 `portable.flag` 使程序从自身目录定位引擎和资源，数据保存在包内 `data/`，不依赖启动时的工作目录。WebView2 缺失时可手动运行包内安装器；测试包包含邮件必需的 PP-OCRv5 检测、识别模型及字典，不含闪光 / YOLO 模型、开发环境或本机用户数据。已有邮件客户端报资源缺失时，可将 `mail-resources` 补丁直接解压到 exe 所在目录，详见[跨电脑邮件试运行](docs/develop/mail-pipeline.md#桌面试运行)。

在 `apps/desktop/` 执行 `npm run tauri build`，配置会构建 release 引擎、生成协议、构建 Web 资源，再打包 NSIS。`npm run tauri build -- --debug --no-bundle` 用于本机桌面原型构建；普通 debug 版本的引擎与资源定位到当前工程，不能作为发行包。当前已提供便携测试 ZIP；更新签名、模型完整包、便携版自动更新、资源回滚与干净系统验收尚未实现。

代码与参考资源遵循 GPL-3.0-or-later；MaaFramework / 官方 Rust 绑定为 LGPL-3.0。SDK 保留独立动态库，发行时保留其许可证和对应源码来源。依赖许可证与来源见 [第三方说明](docs/migration/third-party.md)。
