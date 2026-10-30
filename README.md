# 绝区零一条龙 · Rust 原型

按 [路线图](docs/ROADMAP.md) 迁移到 Rust、Tauri 2 和 MaaFramework。M0 清单与基线已完成，M1 集成原型待验证，M2 **主流程通过，边界待验收**，M3、M4 **开发完成，待验收**。M3 [首批日常](docs/develop/basic-daily-runs.md)为邮件、咖啡、活跃度奖励、丽都城募和刮刮卡；M4 [Rust 战斗](docs/develop/rust-combat.md)已接通条件脚本、角色与闪光识别、声音闪避、键鼠与手柄、脚本导入和桌面运行入口。账号配置、独立执行、历史、日志及资源版本管理共用现有引擎。真实素材、游戏连续执行和异常边界继续验收，30 个游戏应用的正式可用标记保持待验收，尚不能代替旧一条龙。

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

开发启动直接使用本次生成的工程资源，无需安装资源 ZIP；已有的发行资源版本选择不会覆盖开发资源。

首次启动自动构建并启动随附引擎。主窗口关闭后隐藏到托盘；托盘“停止并退出”等待引擎收尾。截图入口只连接 `ZenlessZoneZero.exe`，桌面默认采用 FramePool、前台输入方法与 CPU；游戏客户区需要为 1920×1080。总览与原生诊断提供截图方式选择，支持 FramePool、PrintWindow、DXGI、ScreenDC 和 GDI；诊断页更改方式后点击“获取截图”会先重新连接控制器。疑似黑帧保留预览并提示，同时记录平均亮度和暗像素比例；加载过场可能是真实黑屏，不能据此自动判定失败。DXGI / ScreenDC 需要游戏完整可见且未被遮挡。连接和截图会调用真实 Maa 控制器；健康检查的 Pipeline 不产生游戏按键或点击。

连接游戏后可从侧栏或总览进入[一条龙运行](docs/develop/one-dragon-run.md)，选择当前账号的应用组，预览计划并确认本次账号和输入后启动；页面集中显示分项进度、暂停 / 继续 / 停止及记录保存重试。账号和应用顺序在“账号与应用组”编辑，完整历史在“运行记录”查看。

侧栏“自动战斗”提供自动战斗与闪避助手。先保存当前账号的脚本、输入方式和识别设置，连接战斗画面后确认输入并启动；F9 暂停 / 继续，F10 停止。旧项目战斗脚本通过只读预览导入独立快照。暂停后同 ID 继续会重新识别，丢弃暂停前动作。实机验收尚未开展，详见[开发说明](docs/develop/rust-combat.md)。

M5 正在开发。[丽都周纪、卦象集录、吼吼饼铺、驱动盘拆解、兑换码和录像店](docs/develop/remaining-game-apps.md)已接入独立应用与应用组试运行，沿用“日常设置”。拆解规则及录像店入口、宣传员和转向系数按账号与组保存，并在计划中确认。兑换码在“通用设置”保存全局码表，按账号保留逐码结果，跳过已处理或过期码。需要重新生成 schema 7 日常执行包和协议 v43 配套引擎；旧包继续支持其原有应用。卦象、饼铺与刮刮卡共享游戏内每日次数，固定传送点需已解锁且在地图可见。实机验收及 M5 其余应用继续待办。

“账号与应用组”可保存 20 个预备编队的名称、战斗脚本和成员，支持旧 `team.yml` 导入、账号复制及备份恢复。通过“添加应用到当前组”加入[预备编队检查](docs/develop/remaining-game-apps.md#预备编队检查)，可独立运行或按组执行：从菜单打开管理页、扫描五页、返回大世界并释放输入后保存可靠成员。暂停后重新扫描，未识别或冲突的编队保留原值；需要 schema 6 以上日常包和协议 v43 配套引擎，实机验收保留。

[情报板完整运行](docs/develop/remaining-game-apps.md#情报板完整运行入口与冻结战斗)已接通独立应用和应用组试运行，沿用按组目标及编队、账号战斗输入与设备。预览和启动共同复验冻结脚本；暂停继续、周记录、选队与战斗交接共用运行槽，桌面预先登记 v10 混合输入恢复。当前无委托会结束本次运行，后续仍可继续周目标；需要 schema 7 日常包和协议 v43 配套桌面 / 引擎，全部实机验收保留。

[随便观完整运行](docs/develop/remaining-game-apps.md#随便观完整运行入口)已接通独立应用和应用组试运行：固定传送、托管或手动经营、饮茶仙补料及可选好物铺 / 邦巢购买，共用总预算、暂停恢复和每日成功证明。需要 schema 8 随便观资源及协议 v44 配套桌面 / 引擎，旧 schema 1..7 子流程包保持兼容；德丰大押仍不执行，完整实机验收保留。

[体力刷本计划](docs/develop/remaining-game-apps.md#体力计划配置与估算)已支持按组编辑五类副本、旧配置导入、复制与恢复，以及桌面和 CLI 只读估算。可比较人工填写的电量、储蓄电量和以太电池，查看循环、每日重置与双倍临时计划；需要协议 v45 配套桌面 / 引擎。副本执行仍在开发，估算不会消耗电量或修改运行次数。

[副本内电量恢复](docs/develop/restore-charge.md)已实现为内部共用子流程，并提供独立资源校验与截图检查命令。副本入口、返回画面识别、战斗结算及体力应用执行继续开发，实机验收保留。

[实战模拟室识别与卡片准备](docs/develop/combat-simulation.md)已提供固定区域只读识别、1..5 张卡片准备、保存后实际电量核对和暂停恢复，使用独立 schema 1 资源。该内部子流程只调整方案；目标副本确认、选队、电量恢复串联、战斗结算与体力应用运行登记继续开发，全部实机验收保留。

[快捷手册到实战模拟室的传送](docs/develop/compendium-transport.md)已实现为内部共用子流程，提供训练选中态、同帧条目与按钮配对、确认和实际到达检查，以及独立资源校验与截图检查命令。目标关卡选择、完整副本运行及实机验收继续推进，协议保持 v45。

[模拟室类型与关卡选择投递](docs/develop/simulation-mission.md)已支持类型重选、完整名称匹配、有界双向翻页和暂停后回读，使用独立 schema 1 资源。结果保留为待验证回执；真实关卡选中态证明及后续卡片、电量和战斗串联继续接入，协议保持 v45。

[模拟室结算与领奖收尾](docs/develop/simulation-settlement.md)提供完成结算、确认实际奖励和返回详情的内部子流程，复用父控制器与预算，暂停后不重复投递。独立 schema 1 资源及只读命令不开放体力应用执行；关卡选中证明、完整串联和全部实机验收继续保留。

[模拟室卡片与电量恢复串联](docs/develop/simulation-preparation.md)复用已有资源和同一父控制器，按实际电量打开恢复弹窗，保留冻结设置、消费意图和暂停预算，并在返回详情后重新核对电量。结果仍不提供关卡选中证明；体力应用运行入口与战斗串联继续开发。

[模拟室选队准备](docs/develop/simulation-teams.md)已提供从出战页打开预备编队、二十槽位清除与全表复核、返回页面复验及暂停恢复的内部接口；保持当前队伍时不发送输入。完整父任务仍须补齐关卡选中证明并采用这些接口，体力应用运行入口与实机验收继续保留。

[模拟室出战与子战斗交接](docs/develop/simulation-deploy.md)已提供出战、低等级确认、前移及冻结脚本交接的内部接口，并可接续既有结算领奖。消费入口要求可靠的关卡选中证明，现有未验证回执不能使用；完整体力应用继续关闭，真实流程及手柄切换仍待父任务接入和验收。

[区域巡防单次挑战](docs/develop/area-patrol.md)已接通本次传送、独立 60 电量检查与恢复、选队、出战、单战、结算返回和一次性计数，并接入[体力父循环](docs/develop/charge-worker.md#区域巡防父循环)的冻结战斗与唯一会话槽位。计数保存后返回大世界并重读下一轮资源；共用出战流程保留原模拟室接口。独立及应用组试运行入口见[入口说明](docs/develop/charge-entry.md)，实际游戏验收继续待办，开发阶段按用户选择使用离线回放，不自动启动游戏。

[专业挑战室](docs/develop/expert-challenge.md)已接入同一内部父循环：燃竭关闭与确认保存独立意图，明确 OFF 且实际需要 40 电量才允许消费，未知模式停在待验证。恢复、战斗、结算与一次性登记已有离线回放，专业后接区域计划分别记账；专业详情及燃竭 OFF / 确认缺少真实留档，实际识别和操作继续保留验收。

[深度追猎](docs/develop/deep-hunt.md)接入同一内部父循环：完整目标、周常耗尽、深度 ON 与实际 60 电量分别核对；周常仍有奖励时跳过且不计成功。出战后复用距离、交互和鸣徽移动，再交接战斗与结算。列表、详情及开启确认采用标注用途的合成画面验证，实机素材和操作验收继续待办。

[深度追猎指定等级](docs/develop/charge-level.md)已接入消费前置：独立菜单意图、完整等级匹配和切换后回读，暂停继续和电量恢复后再次核对。默认等级保持原路径，其他分类仍要求默认等级。[代理人方案](docs/develop/agent-plan.md)已有与参考 OpenCV 对照的标记检测及固定图检查；本阶段继续离线验证并保留实机验收。

[代理人培养导航](docs/develop/agent-transport.md)已加入四类训练的内部传送：同帧配对标记、具体副本与前往，保留原计划，分别保存导航意图并核对实际到达。暂停不重复前往或确认，缺少目标行按钮不借用下一行；有限查找返回独立的未找到终态。[区域、专业和追猎代理人挑战](docs/develop/agent-challenge.md)由本次导航直接交接，结算同帧读取完整“已达成”，每场正常完成只给原计划加一次，达成后跳过本次余项。无目标、不足和周常阻挡不计成功，含糊读数保留单场计数后等待验证。模拟室生产选中证明与完整消费、可靠无活动判断继续开发。

[模拟室培养详情选择](docs/develop/agent-mission.md)已接通原导航到具体类型和关卡投递：第一个标记与同列完整名称配对，有限翻页，暂停后不重复选择；原代理人计划保持。回执仅证明本次投递及详情可读，完整模拟室挑战继续等待生产选中证明，未开放正式消费入口。

[五次双倍余量](docs/develop/charge-double.md)已接入内部父流程：完整标签、独立单行次数及同位置分类选中共同核对，明确 `0/5` 继续普通计划，正余量按当前电量生成一次临时计划。缺少标签、咖啡 `1/1` 和冲突读数保持未知；可靠无活动判断与模拟室完整消费仍待完成。

[体力应用记录](docs/develop/charge-application.md)已接通内部父实例的历史与每日证明：摘要复验原始消费日志，完整完成、返回大世界、输入释放和次数提交共同提供成功；空计划、阻挡、停止或待验证不写每日成功。最终历史失败仅补存，重启后的查询和导出已离线验证；已接入[独立及应用组试运行入口](docs/develop/charge-entry.md)：整组首个输入前冻结资源、预检全部脚本，缺少模拟室生产证明时阻挡，资源不足不记完整成功。入口阶段新增 17 项、连同回归 80 项检查通过。

[当前参考来源兼容](docs/develop/charge-source-compatibility.md)已补齐名册、管理页槽位扫描及[公共战前按槽位选队](docs/develop/team-selection-slots.md)：实际按钮定位和局部反馈保留清除预选、顺序选择与二十槽复核，名称和人数只记录到本次检查点，配置及脚本保持冻结。当前四十条来源全部匹配，可直接生成 schema 10 日常包，要求协议 v51，导入预览 v20；旧资源保持原义。本轮新增 11 项、连同相邻 76 项 Rust 检查、严格检查及前端 / 原生构建通过。模拟室生产选中证明、可靠无活动判断、完整消费及全部实机验收保留，正式可用标记不提前修改。

运行页可开启“仅本次选择应用”，临时勾选邮件、咖啡或其他组内应用，按保存顺序预检及执行，不改变应用组配置；未迁移项被选中时仍会拦截。刷新配置、换组、断线或离开页面后恢复保存的默认启用项。

如需重试本游戏日已完成的应用，可开启“仅本次重新运行已完成应用”，重新预览并确认后运行。可与本次选择组合使用，保留保存的每日策略和已有成功记录；不会增加游戏内领取次数。

一条龙运行页可保存[结束后动作](docs/develop/after-done.md)：无、关闭游戏、关机、休眠或睡眠。整组正常完成并确认保存及输入释放后进入 60 秒倒计时，可在任意页面或托盘取消本次动作；失败、停止和独立诊断不执行。

总览提供[手动启动游戏](docs/develop/game-launch.md)：先在“账号与应用组”按账号保存游戏路径及可选的固定 1080p 参数，再回总览核对并确认启动。启动后手动登录、扫描和连接窗口，随后进入一条龙运行。

“账号与应用组”还可保存[运行结果提醒](docs/develop/run-notifications.md)策略。应用组完成或异常后，可从全局提示、侧栏和托盘进入运行提醒，跨页面及断线后查看并标记已读；默认仅异常。

需要补充奖励等真实画面时，可在总览连接游戏后进入[画面素材](docs/develop/screen-samples.md)，按需采集、核对预览和人工标注，再保存 PNG 与元信息到本机数据目录 `diagnostics/screens/`。

同页的[已保存素材](docs/develop/screen-sample-library.md)支持按状态和备注筛选、分页及图片回看，重启或引擎断线后仍可读取；打开时自动核对文件哈希和实际尺寸。

只查看界面可以执行 `npm run dev`。浏览器显示“界面预览”，原生功能保持禁用。

[锄大地](docs/develop/world-patrol.md)已接入独立应用和应用组试运行：迁移 21 条路线、4 份名单、循环与重试设置、旧配置导入、小地图定位、跑图与战斗交接，以及按游戏日续跑。需要 schema 1 锄大地资源和协议 v46 配套桌面 / 引擎。路线与大地图录制工具、异常后自动重开并登录游戏未迁移；全部实机验收保留。

## CLI

`OcrText` 现支持 [RGB 颜色范围过滤](docs/develop/ocr-color-filter.md)，可用固定截图核对快捷手册当前选中的标签；上下界与旧版 RGB 设置一致，二值化由 Maa 原生完成。

下列命令在工程根目录执行，不需要 Tauri。SDK 必须先完成安装。

[活跃度奖励识别准备](docs/develop/engagement-reward.md)提供 `engagement-reward-bundle`、`engagement-reward-bundle-check` 和 `engagement-reward-inspect`，用于生成、校验只读资源及检查固定 1080p 截图。日常页要求黄色选中页签和正文标题同时命中，报告保留可见完成标记；旧只读包需重新生成为 schema 2。完整领取另由 `basic-daily-bundle` 生成的执行包提供，可从应用组或独立应用试运行，真实奖励后态继续待验收。

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

CLI 键盘使用 Win32 VK，可选 `--mode foreground-legacy` 或 `background-post` 对照 Maa 的兼容方式；手柄参数、计时边界和未完成的实机检查见 [验收记录](docs/migration/acceptance.md)。邮件暂停 / 继续见[试运行说明](docs/develop/mail-pipeline.md)，旧配置的账号与应用组导入见[M3 使用说明](docs/develop/account-config.md#使用)。声音闪避、HTTP、MCP 和其余游戏应用按后续阶段迁移。

新动作使用 `--action mouse|stick|trigger`，其余停止、确认与计时参数相同。示例中的句柄需要替换为 `windows` 命令返回的游戏句柄：

```powershell
cargo run -p zzz-engine -- input-probe --hwnd <十六进制句柄> --action mouse --button left --x 960 --y 540 --hold-ms 500 --allow-input
cargo run -p zzz-engine -- input-probe --hwnd <十六进制句柄> --mode xbox --action stick --side left --x 16000 --y 0 --hold-ms 2000 --cancel-after-ms 4000 --allow-input
cargo run -p zzz-engine -- input-probe --hwnd <十六进制句柄> --mode ds4 --action trigger --side right --value 128 --hold-ms 500 --allow-input
```

桌面“通用设置”可保存默认截图方式、邮件超时和启动 / 继续延时。设置由 Rust 与账号配置一同持久保存，应用组预览显示实际参数，开始时由引擎核对修订并读取。原生诊断可临时调整；完整用法与旧截图配置导入范围见[通用运行设置](docs/develop/general-settings.md)。账号区服、按日跳过及游戏日过期处理见[应用组每日计划](docs/develop/daily-runs.md)。

同页“桌面外观”支持跟随系统、浅色和深色主题，以及 90% / 100% / 110% / 125% 界面缩放。选择后立即应用并独立保存在本机，运行或引擎未连接时也可调整；详见[外观设置](docs/develop/desktop-appearance.md)。

“账号与应用组”支持新建、复制、重命名和删除应用组。新建组的应用全部禁用；复制保留当前组的顺序、启用项及运行策略，按日完成状态仍由同一账号的各组共用。每个账号保留 1..64 组，删除前需确认；完整规则见[应用组管理](docs/develop/account-config.md#应用组管理)。

同页还支持复制账号配置、确认删除非当前账号，以及账号和应用组排序。账号副本使用新编号，默认不参与一条龙；删除保留历史、导入归档和编号，不删除当前或最后一个账号。规则与恢复方式见[账号管理](docs/develop/instance-management.md)。

同页“配置备份与恢复”可读取自动备份，预览账号、应用组和通用设置差异，确认后恢复。恢复前保存当前配置，修订继续递增，已移除账号的编号不分配给新账号。当前配置须能正常读取；用法与范围见[配置备份说明](docs/develop/config-backups.md)。

当前协议为 v45，桌面与引擎必须使用同一版本。托管引擎通过桌面持有的私有管道回连，支持引擎单独提权、受控输入 Pipeline、五项首批日常、应用组及独立运行、账号历史、模型资源及运行资源版本发现。账号与组可保存[咖啡参数](docs/develop/coffee-settings.md)和[日常参数](docs/develop/basic-daily-runs.md)，由引擎读取并冻结；原生咖啡诊断也可选择来源。旧数字按键命令继续支持 `--key`，已保存的旧运行记录可以读取；新运行记录使用明确的动作结构。

“通用设置 → 下载代理”可保存系统代理、直连或自定义 HTTP / HTTPS 代理，供下一次模型准备及运行资源包下载使用。自定义代理暂不支持用户名和密码。CLI 两类下载命令支持 `--proxy system|direct|<地址>`，完整规则见[下载代理说明](docs/develop/download-proxy.md)。

“独立应用运行”可从当前账号的应用组选择邮件或咖啡，单独执行保存为禁用或本游戏日已完成的应用。参数沿用保存配置，成功会更新每日记录，独立运行不会触发一条龙结束后动作；暂停、继续、停止和结果提醒共用现有机制。详见[独立应用运行说明](docs/develop/standalone-run.md)。

“原生诊断 → 咖啡点单试运行”支持已打开点单选项，以及“从大世界传送并点单”两种起点。新起点会按 N 开图、沿澄辉坪 → 汀曼咖啡传送、按 F 交互并推进对话，再接入已有点单、电量确认和返回流程；需要 v2 咖啡执行包。先更新配套桌面、引擎与资源。独立诊断只保存诊断记录；[应用组](docs/develop/coffee-group-runs.md)可顺序试运行邮件和咖啡，写账号分项与每日记录。完整领取与实际游戏串联仍待验收，当前素材缺口和资源包校验值见[咖啡点单与顺序调度](docs/develop/coffee-and-groups.md)。

咖啡前置的“澄辉坪 → 汀曼咖啡”已接入“原生诊断 → 地图传送试运行”，2026 年 10 月 17 日获正常传送流程实机通过反馈，暂停、停止及异常恢复等边界继续待验收。先按 N 打开普通地图，确认本次路线后启动，在延时期间切回游戏；暂停保留检查点，继续前重新识别，停止等待输入释放。地图与其他任务共用运行槽及崩溃输入恢复，结果只保存为诊断记录。`map-bundle` 生成 v2 资源，`map-bundle-check` 校验资源，`map-inspect` 只读检查留档；命令、资源准备及实机待验项见[地图传送说明](docs/develop/map-transport.md)。

桌面在发送真实输入探针、输入 Pipeline 或首批日常前，把恢复计划写入数据目录的 `input-recovery.json`。引擎崩溃、应答丢失或收尾未确认时会阻止重连，请点击“恢复遗留输入”：确认旧进程退出后，由独立进程只发送对应释放；原管理员引擎可能需要再次授权 Windows。恢复后先确认游戏状态，再手动重连，不自动重放任务。桌面重启也保留拦截；失败或取消授权不会清除记录。当前覆盖桌面托管的单次输入、受控 Pipeline、邮件、地图、咖啡及首批五项应用组 / 独立运行；v7 增加 Escape 与 W，旧恢复记录保持原义。CLI 和未来业务 Pipeline 须单独接入。真实游戏、跨权限释放与 ViGEm 卸载仍需实机验收，见[实现说明](docs/develop/implementation.md#m1-崩溃后的独立输入恢复)。

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

桌面“运行日志”页支持按引擎会话、运行 ID、级别和文字筛选，断线后仍可查看最近四个会话，并将保留日志导出到本机数据目录。缓存与导出范围见[运行日志说明](docs/develop/run-logs.md)。

桌面“运行记录”页按账号、应用、组标识、游戏日和结果筛选已保存历史，支持连续翻页和本页 JSON 导出，保留邮件 / 咖啡的分项结果及输入释放状态。导出前复验本页内容，文件保存到本机数据目录 `runs/exports/`。使用与验证范围见[运行记录说明](docs/develop/run-history.md)。

已转换的邮件、地图、咖啡及基础运行资源可由原生 CLI 打成独立版本包，通过显式 HTTPS 下载或选择本地 ZIP 安装。桌面“运行资源”页可刷新已安装版本，预检后退出并重启引擎以启用或回滚；同样保留离线 CLI 入口。用法见[运行资源包](docs/develop/runtime-resources.md)。联网版本发现与签名发布继续待实现。

已增加旧版闪光、OCR 和 YOLO 实际输出采样及纯 Rust 的 `recognition-compare` 命令，支持核对同一批固定图的概率、候选、文字、框与分数；OCR 同时比较过滤前结果。旧版采样环境单独放在忽略的 SDK 缓存中，仅用于迁移取证；新程序和正式回归不依赖 Python。命令见[闪光旧版输出说明](docs/develop/recognition-validation.md#旧版实际输出与自动比较)及 [OCR / YOLO 对照说明](docs/develop/recognition-validation.md#ocr-与-yolo-旧版对照)。比较结果与人工标注分别报告，不能把新旧共有的漏检当成人工预期通过。

六个 Rust crate 分别负责通用核心、协议、平台、Maa 适配、引擎与迁移工具；桌面位于 `apps/desktop/`。详见 [首轮实现说明](docs/develop/implementation.md)。

开发 CLI 使用工程 `data/`；桌面使用 Tauri 的用户应用数据目录。引擎记录任务终态与有容量限制的日志；截图按需返回 PNG，不自动写用户截图到日志。SDK 缓存位于 `sdk/maa/bin/`，通过固定清单校验所有运行 DLL，并限制加载目录。版本和压缩包 SHA-256 的来源是 [sdk.lock.json](sdk.lock.json)，原生文件清单见 [sdk-manifest.json](resources/sdk-manifest.json)。

## 迁移与验证

迷失之地已提供按组配置、旧挑战方案导入、藏品与日周目标决策、只读画面检查，以及入口、战略与周期增益、UP 识别、普通及矩阵选队、通用选择、武备与棱镜、硬币 / 血量商店、事件与对话、NPC 返回走位、抽奖和路径迭换内部子流程；候选脚本已支持预编译及场景 / 选队 / 战斗资源冻结，准备、导航、运动脱困、实际 HUD 队伍、遭遇战斗、层内循环与跨层交接已有内部执行器。中途 / 最终结算、单次挑战、实际重开 / 退出、悬赏领取及普通大世界返回已接通离线回路，日周、UP 和领取记录支持账号隔离、幂等提交及只重放记账的恢复，详见[迷失之地开发说明](docs/develop/lost-void.md)。独立 / 应用组试运行、统一工作线程、运行历史、桌面进度和输入恢复已接通，协议 v54。默认日常资源已升级为 schema 10，导航复用其中的固定模型，运行资源 ZIP 和便携构建纳入场景及配套能力检查；普通保持当前队伍已从出战页完整计数冻结实际人数并交给 HUD，少人出战确认已接通，入口图文提示及悬赏面板关闭已接通，并已生成匹配的本地便携包，正式可用标记及实机验收保留。

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

账号配置损坏无法读取，或文件丢失需要找回配置时，可在桌面总览的“离线配置恢复”查看、预览并确认恢复自动备份，完成后重连引擎；损坏原文独立保留。原生引擎的 `config-recovery` 命令继续可用，步骤及范围见[离线配置恢复](docs/develop/config-recovery.md)。

代码与参考资源遵循 GPL-3.0-or-later；MaaFramework / 官方 Rust 绑定为 LGPL-3.0。SDK 保留独立动态库，发行时保留其许可证和对应源码来源。依赖许可证与来源见 [第三方说明](docs/migration/third-party.md)。
