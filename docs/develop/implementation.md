# 首轮实现

路线图以 [docs/ROADMAP.md](../ROADMAP.md) 为唯一来源。参考源码固定到 `b15662f7ce61bcfdeaad6b401695e5dcb0af8f42`，不修改参考项目。

M0 清单与现有基线已按路线图关闭，当前推进 M1 的可运行工程。原型提供真实 Maa SDK 加载、窗口发现、1080p 截图、原生健康检查 Pipeline、受控输入探针及离线识别入口。全部游戏应用在通过各自验收前显示“待迁移”，不返回虚构的成功结果。M1 的 CPU / DirectML、游戏前后台输入、手柄、模型适配和暂停兼容结论依靠固定语料与实机记录，不能由编译成功替代。

## 实现边界

- `od-core` 定义应用登记、运行槽、结果和坐标规则，不依赖 Tauri 或 Maa。
- `od-protocol` 定义限长 JSON 帧、版本、请求 ID、结构化错误和 TypeScript 类型。
- `maa-adapter` 在专用工作线程中持有 SDK 和所有原生句柄。单一路径加载固定 SDK，明确完成与投递的区别。首轮不宣称支持 Pipeline 原位暂停。
- `od-platform` 提供窗口、进程身份、命名管道身份校验和游戏控制权互斥。
- `zzz-engine` 提供 CLI 与桌面命名管道入口。HTTP、MCP 和复杂业务按后续阶段验收，不提前开放任意 Pipeline 或文件执行。
- `od-tools` 用 Rust 扫描迁移输入、转换旧区域和模板、校验 SDK 与生成协议类型。
- Tauri 进程监督随附引擎，React / Fluent UI 显示真实快照、迁移登记、截图与诊断。浏览器预览只提供静态界面说明，不模拟引擎成功。

桌面通过随机命名管道和一次性配对密钥连接引擎；新托管入口由桌面先建立服务端，引擎回连，双方确认进程、用户及会话后通过管道配对，密钥不放在命令行或临时文件。旧 `serve` 的 stdin 配对入口保留供独立验证。管道拒绝远程连接、使用当前用户 ACL。数据目录与资源目录由宿主指定，不依赖引擎当前工作目录。连接丢失时停止工作并退出，独立 CLI 不受桌面生命周期限制。

运行请求先占用唯一运行槽，再进入工作线程。停止返回“正在停止”；工作线程等待 Maa Tasker、控制器队列与输入释放完成后才保存终态。原生阻塞不能假装超时后已释放；错误保留诊断。首轮截图和无输入健康检查可以在无游戏情况下验证。游戏输入探针由桌面确认后启动，或使用 CLI 显式指定 `--allow-input`。

所有测试放在独立 `zzz-od-test/test/zzz_auto/`，通过路径依赖引用本项目。主仓不添加内联测试或测试目录。版本、清单和已验证 / 未验证项目记录到验收文档。

## M1 引擎单独提权

桌面默认普通权限启动引擎。用户点击“提升引擎权限”后，通过 Windows `ShellExecuteExW` 的 `runas` 启动随附的同一个引擎，WebView 不重启或提权。启动在后台线程执行，等待 Windows 授权不阻塞界面；用户取消授权时返回明确错误并保留旧引擎。启动参数仅含固定引擎路径、资源 / 数据目录、随机管道名和宿主 PID，不支持前端指定程序或命令。

新托管入口 `serve-client` 回连由桌面创建的私有管道，避免高完整性引擎创建的管道阻止普通桌面写入。双方验证 PID、同一用户和交互会话，再交换一次性配对信息；不同管理员账号的凭据提升会被拒绝。引擎完成身份与配对检查前不加载 SDK 或创建业务数据。连接和配对均有时限；成功配对后保留进程句柄至确认退出，配对失败的候选不能执行任务并自行超时退出。客户端保留 Tokio 默认的 `SECURITY_IDENTIFICATION`，不允许服务端模拟提升后的客户端。

权限切换只在无运行任务时开放，后端再次核对任务状态。候选引擎先配对，旧引擎收到正常退出命令、释放输入并退出后才切换；不自动重连游戏或重跑任务。切换失败关闭候选管道，候选引擎自行收尾。桌面断线或请求超时关闭管道以触发引擎清理，不强制终止；旧进程尚未退出时保留其句柄并拒绝重连 / 提权，避免误报停止或同时控制游戏。托盘退出同样等待引擎退出，无法确认时显示错误并保留桌面。

等待 UAC 授权期间每十秒向旧引擎发送状态请求，避免其 45 秒空闲检测断开当前连接。候选启动失败或授权取消时不自动断开旧引擎。协议升为 v6，前端状态单独报告实际引擎 / 桌面完整性级别，不把请求提权视为已成功提升。

状态通道显示引擎实际完整性级别；自动回归覆盖普通子进程、授权取消结果、错误 PID / 配对参数、配对超时、退出与断线，以及超过 45 秒的授权等待。实际 UAC 授权、跨权限通信、不同用户 / 会话拒绝和管理员游戏输入仍需实机验收；原生崩溃后的恢复由下节的独立路径处理。

启动依据为微软 [ShellExecuteExW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shellexecuteexw) 和 [SHELLEXECUTEINFOW](https://learn.microsoft.com/en-us/windows/win32/api/shellapi/ns-shellapi-shellexecuteinfow)；使用进程句柄确认退出，直接传递经过 Windows 参数规则转义的 Unicode 参数，不经过命令解释器。管道客户端的身份模拟限制见 [Tokio ClientOptions](https://docs.rs/tokio/1.53.2/tokio/net/windows/named_pipe/struct.ClientOptions.html#method.security_qos_flags)。独立测试仓的 `elevation.rs` 引用同一份桌面会话实现，普通子进程路径使用真实引擎，授权取消使用注入的失败结果；不会在自动测试中触发 UAC。

引擎权限路径的独立回归可运行 `cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test elevation -- --test-threads=1`；其中包含 46 秒的授权等待模拟。界面回归覆盖浏览器禁用、运行中禁止提权、授权取消和切换后重新连接游戏。原始日志保存在主仓 `artifacts/validation-engine-elevation-20261015.log` 与 `validation-elevation-final-20261015.log`，摘要及输入索引保存在测试仓 `fixtures/elevation/`。

## M1 崩溃后的独立输入恢复

桌面托管的单次输入探针在发送 IPC 前，由 Rust 宿主把保守恢复计划同步写盘。记录包含原引擎和游戏的 PID / 创建时间、窗口、输入方式、单次动作及原引擎完整性级别；不依赖展示回调、引擎内存或崩溃进程的析构。收到运行 ID 后原子补全记录，只有同一运行的显式 `released` 终态或明确的请求拒绝才能清除。应答前崩溃仍保留初始计划。首次写入不能覆盖另一桌面实例的记录；读盘失败、损坏或版本不支持时阻止运行，不能当作空记录。

断线后先等待旧进程退出；进程身份同时检查创建时间，防止 PID 被复用。恢复不会强制终止仍活着的引擎。点击“恢复遗留输入”后，宿主启动随附引擎的内部 `recover-input` 入口，沿用私有管道、同用户 / 会话 / 双方 PID 校验及一次性密钥。该入口不初始化 Maa、模型、运行槽或游戏任务，只接受受限恢复计划。恢复需要原引擎的权限，普通桌面恢复管理员引擎时单独触发 Windows 授权；取消、超时及失败保留记录，仍禁止重连、提权切换及新动作。正常托盘退出也先停止并读取释放证明，不能以 Shutdown Ack 代替证明。

前台 / Legacy 键鼠只通过 Win32 发送记录中的 KeyUp 或鼠标抬起，鼠标不移动，不释放任意其他键。后台模式校验原游戏进程和窗口，取得游戏控制权互斥，覆盖同进程仍存在的顶层窗口，包含 SDK 可能选中的弹窗；鼠标客户区坐标转换到各窗口。SendMessage 采用 1.5 秒超时并检查结果。PostMessage 先检查目标响应，再沿原队列追加释放，避免同步 KeyUp 越过尚未处理的 KeyDown；成功只证明入队，界面提示尚未确认游戏消费。游戏进程已退出时不向复用的后台窗口发送消息。恢复最多覆盖 16 个窗口，整体响应限 30 秒，部分失败也保留完整计划以便重试。

Xbox / DS4 不创建新手柄冒充旧设备复位。确认拥有进程已退出后，根据 [ViGEmClient 的设备移除契约](https://github.com/nefarius/ViGEmClient/blob/master/include/ViGEm/Client.h)由驱动移除该进程拥有的虚拟设备，界面要求人工确认游戏状态。该契约与键鼠 API 投递成功都不能代替游戏消费、真实驱动卸载和高权限路径的实机验证。恢复失败时提示停止使用、按下并松开对应键鼠、关闭游戏并检查虚拟设备，必要时重启 Windows；损坏记录须在确认旧引擎已退出、游戏已关闭且输入已人工恢复后，备份并移走数据目录中的记录再重启桌面，禁止在输入状态不明时直接删记录。

后台路由依据固定 SDK 的 [MessageInput](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaWin32ControlUnit/Input/MessageInput.cpp)；Win32 投递限制见 [SendInput](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput) 和 [SendMessageTimeoutW](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendmessagetimeoutw)。恢复结果要求用户先检查游戏，再手动重连并重新勾选输入确认；不会恢复之前的任务。当前独立恢复覆盖桌面托管输入探针，CLI 不受桌面监督，游戏带输入 Pipeline 尚未开放，后续必须先登记全部可能输入再接入恢复计划。

独立测试仓的 `input_recovery.rs` / `input_window.rs` 使用隐藏测试窗口、真正的进程退出和独立恢复可执行进程，覆盖发送前落盘、应答丢失、明确释放证明、存活进程拒绝、PID 复用、窗口挂起、控制权冲突、全部鼠标按键、扩展键、弹窗及 PostMessage 队列顺序。模拟按下消息仅发往测试自身窗口，不向游戏或系统输入流按下任何键；前台包检查只构造事件而不调用 SendInput。另有桌面界面回归覆盖取消恢复、失败保持拦截、手动重连及确认重置。

```powershell
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test input_recovery -- --test-threads=1
```

## M1 输入诊断

0.1.3 排查用户反馈的键盘未响应：按键在桌面中按名称选择，保留 VK 协议；前台 / 后台保持原方法，并增加 Maa 的 LegacyEvent / PostMessage 对照方式。窗口快照记录游戏与引擎的 Windows 完整性级别，键盘投递前重新检查权限和窗口身份；权限较低时报告失败。前台探针在启动延时结束后要求游戏处于前台，避免把键盘发送给其他窗口。手柄不执行此键盘检查；Xbox 已有用户基础实测反馈，DS4 仍待验证。SDK 仍可能报告接口完成但游戏未消费事件，兼容结论须由实机反馈确定。

后续业务按前台键盘和后台 Xbox 数字按键划分输入边界。用户已反馈前台键盘响应、持有期间停止与无残留动作，以及实际后台 Xbox 数字按键正常；SendMessage / PostMessage 后台键盘均未生效，保留供诊断，不作为游戏应用的可用后台键盘能力。0.1.4 鼠标按键及 Xbox / DS4 摇杆与扳机已有基础实机通过反馈；具体前后台组合、DS4 数字按键、完整 Pipeline 与异常收尾继续验收，具体证据只维护在[验收记录](../migration/acceptance.md)。

桌面与 CLI 共用有时间上限的按键探针，支持前台 / 后台键盘及 Xbox / DS4 数字按键。桌面启动前需要勾选真实输入确认；引擎校验确认、模式、按键与时间，先占用运行槽，再在 Maa 工作线程切换控制器。切换前销毁旧控制器并释放输入与控制权；切换失败清除连接状态。截图、健康检查、其他探针与重新连接在运行期间拒绝执行。

探针支持启动延时和中途停止。取消发生在延时阶段时不再投递按下；已经投递的按下必须等原生队列完成，再按顺序释放，不能把停止请求当作取消了控制器队列。持有阶段每 5ms 检查取消，不等待整个持有时间。按下前登记按键，按下失败、停止、正常结束及退出均尝试释放；释放和 inactive 都得到完成确认才报告成功。原生阻塞仍等待，不能用软件超时假装输入已经释放。

输入结果作为运行记录的结构化字段保存，区分投递耗时、按下完成耗时、实际持有时间、释放耗时和检测到取消后的收尾时间。探针计时不包含控制器重建；收尾计时包含未完成的按下，不包含请求跨进程传输和取消检查间隔，不能直接作为游戏端响应或完整停止 P95。检查之间最多休眠 5ms，Windows 调度仍可能延长实际间隔。独立测试使用真实 Maa 队列和可记录、可失败的自定义控制器；这提供顺序和清理证据，实际前后台输入、驱动、权限、摇杆、鼠标与崩溃恢复仍需分别验收。

## M1 鼠标与手柄模拟量诊断

0.1.4 扩展同一个输入探针，协议升为 v5。输入配置使用显式 `action`，区分数字按键、鼠标按键、摇杆和扳机，避免把坐标或模拟量当作 VK。旧运行记录的 `key` 配置仍能反序列化，重复指定 `key` 与 `action` 则拒绝。桌面按输入方式限制可选动作，每次修改参数、切换动作或窗口后重新确认；CLI 保留旧 `--key` 入口，通过 `--action mouse|stick|trigger` 显式选择新动作。

鼠标经 Maa `touch_down/up` 在客户区坐标持有左、右、中或两个侧键，投递前通过同一队列获取截图、确认尺寸及坐标缓存，再检查窗口身份、1920×1080 尺寸、权限与所选前台条件。截图失败或尺寸异常时不投递鼠标输入，截图等待期间取消也等待该队列操作结束后收尾。坐标只允许 X 0..1919、Y 0..1079。手柄使用同一接口的专用通道：左右摇杆为 0 / 1，X / Y 为 -32768..32767；左右扳机为 2 / 3，力度为 1..255。手柄释放通过 `touch_up` 回中或归零。接口依据固定 [Maa 5.14.3 手柄映射](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/docs/en_us/2.4-ControlMethods.md#analog-input-touch_downtouch_movetouch_up)及[鼠标映射源码](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaWin32ControlUnit/Input/InputUtils.h)。

适配器分别记录未释放的按键和通道。投递前登记，原生输入失败后也尝试释放；取消时等待已投递操作完成，再释放并等待 inactive。释放失败时保留状态，清理通过之前拒绝新动作、控制器切换与健康检查 Pipeline；正常退出会重试清理。持有时长、取消轮询和计时边界沿用数字按键探针，`key_down_ms` 继续表示各类动作的按下完成时间；鼠标截图准备计入 `elapsed_ms`，不计入按下时间。鼠标拖动、滚轮和连续摇杆变化留待后续实现，当前工具用于单次持有与释放验收；桌面托管的崩溃恢复见上节。

## M1 模型对照入口

新增固定模型获取、资源转换及 `recognition-check` CLI，具体设计见[模型对照说明](recognition-validation.md)。Maa 适配器提供可复用的固定图缓存、识别计时及 GPU 会话独占规则；CPU 会话销毁后才创建 DirectML 会话。该规则限制本进程，离线 CLI 与其他引擎 / 旧版程序的同机 GPU 测量须分开运行。该入口不开放桌面任意文件调用，不启动游戏任务，也不将模型能加载记为业务迁移通过。

## M1 带输入 Pipeline 停止验证

健康检查与离线 Pipeline 探针共用停止、等待及输入收尾实现。离线探针只接受固定图或自定义控制器，不能用于已连接的游戏控制器，也不新增桌面或 IPC 命令。测试仓提供原生 Pipeline 和可记录输入、延迟完成、注入失败的控制器，验证真实 Maa 队列；输入清单必须列出该 Pipeline 全部分支可能使用的按键和鼠标 / 手柄通道。

投递前把输入清单记入待释放状态，即使按下回调报告失败，也不能假定没有产生输入。停止或超时后请求 `post_stop`，等待原任务、停止任务和 Tasker 收尾，再在同一控制器队列显式释放清单中的输入，最后等待 `inactive`。控制器的长按 / 阻塞回调可能晚于 Tasker 结束；固定 SDK 在仍停止时会返回无效投递 ID，适配层继续等待清理请求被队列接受并执行，不以 Tasker 终态代替控制器完成。正常完成和失败也执行相同清理。原生 `auto_up` 作为补充，不能代替可检查结果的释放；重复释放在队列结束后执行，可能包括尚未走到的分支所声明的输入。清理失败保留待释放状态，下次工作须先重试，失败时禁止启动新任务。

探针分别报告成功、失败、主动停止和超时；SDK 接口错误及输入清理失败返回错误，不能写成成功或已安全停止。停止耗时从工作线程发现取消开始，包含原生停止调用、阻塞回调和显式输入释放，不包含 IPC 和轮询发现取消前的时间。阻塞回调结束前持续等待，不以 250ms 目标作为强制交还控制权的超时。

固定 SDK 的 `pre_delay`、`post_delay` 和单次 `repeat_delay` 使用阻塞休眠，不能用长等待实现及时停止。健康检查把原两秒 `post_delay` 改为 81 次无输入动作、每次间隔 25ms，等待总量仍约两秒，循环之间由 Maa 检查停止。测试仓保留三秒长等待和 25ms 短间隔的对照；长按动作及无法取消的回调仍需等待，不能把这类结果算入正常动作的 250ms 目标通过项。后续业务不得用重复执行领取 / 提交的方式拆等待，应使用无输入等待或重新识别画面的步骤。

M2 的安全暂停采用“停止投递、等待在途动作、释放输入、保存业务步骤”，继续前重新识别并从可恢复步骤执行；不将 `post_stop` 或从入口重跑当作原位暂停。不可重复的领取 / 提交先查询是否已生效。此轮只验证停止所需的原生队列与输入清理，不宣称业务步骤保存、恢复或游戏实机验收已完成。

行为依据为固定 SDK 的 [Tasker 停止实现](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaFramework/Tasker/Tasker.cpp#L150)与 [Controller 的停止、投递检查和自动释放](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaFramework/Controller/ControllerAgent.cpp)。`auto_up` 的自动释放路径不检查底层释放的返回值；控制器仍停止时拒绝新投递。Rust 适配层据此保留自己的清理状态，并由真实 SDK 回归验证，不能用可丢弃的展示回调判断是否已释放。

长等待的限制见同版本 [TaskBase::sleep](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaFramework/Task/TaskBase.cpp#L199)；短间隔方案只限定每次不可中断的休眠长度，不能证明截图、识别、输入和 IPC 的完整停止链均满足目标。

单独复跑：

```powershell
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test pipeline_stop -- --test-threads=1
```

添加 `--nocapture` 可输出 `PIPELINE_OBSERVATION` JSON，记录取消、超时、阻塞回调和等待方式对照的诊断耗时。它们使用 debug 构建，没有正式预热或足够的同类性能样本，不能作为完整游戏链路 P95 验收。首轮原始观察和输入索引保存在测试仓 `fixtures/pipeline/observations-20261015.json` 与 `manifest-20261015.json`，结论见[验收记录](../migration/acceptance.md#2026-年-10-月-15-日带输入-pipeline-停止与等待方案)。
