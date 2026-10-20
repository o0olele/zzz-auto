# M4 Rust 战斗开发

M4 使用 Rust 执行条件脚本，Maa 唯一控制器提供截图、识别和动作。日常与战斗共享现有运行槽、事件、历史及暂停 / 停止入口。真实游戏、延迟指标、长时间稳定性和声音阈值对照均保留为验收，不以自动回归替代。

## 脚本与时钟

保留 YAML 的场景、触发器、优先级、时间窗、值范围、嵌套 handler、继承打断条件、状态及动作模板和旧 `data` 参数。表达式中的 `&` / `|` 与参考解析器一样从左到右结合，括号和 `!` 优先；空表达式为真。加载时展开全部模板，拒绝循环、未知指令和字段，并把文件与字段路径包含在错误中。禁止加载失败后运行部分脚本。

加载支持 YAML 锚点、`<<` 合并键和模板说明字段。`state_template` 与外层 `states` / `interrupt_states` 同时存在时保留外层条件，避免旧加载器丢失条件；不允许同时再写 `operations` 或 `sub_handlers`。旧样例中设置 / 清除状态误带的方括号和中文引号在载入时去掉，写入表达式实际使用的状态名。此两项修正与旧程序的静默忽略行为有区别，合并和未合并脚本均应列入正式回放验收。未知业务状态与角色名称在启动前拒绝，`自定义-` 状态允许扩展。

调度与识别使用单调时间。状态记录保留观测时间，过期或前一代识别结果不得更新状态。暂停先取消执行、等待控制器并释放输入，继续时清空识别缓存和待执行动作，重新识别；不重放暂停前动作。脚本中的未来状态时间与负数窗口继续支持。

## 调度与输入

一次只有一条动作序列。普通场景只在空闲时轮询，触发场景按间隔与优先级抢占；同一场景不抢占自身。父子 `interrupt_states` 合并。所有动作经单一执行器投递；抢占、暂停和停止先释放上一序列持有输入，失败则结束任务并保留恢复记录。按下、点按、松开、重复、前后等待、角色切换、状态修改与转向共用该规则。

无时长的按下可跨 handler 保持，显式松开、角色切换、抢占、画面失效、暂停和停止负责释放。逻辑按键共享物理按键时按持有关系合并，方向摇杆叠加，扳机取最大值；转向结束只移除转向持有。定时松键优先于可能阻塞的下一次截图识别。周期锁定与转向等待主序列空闲后进入同一队列，避免另一线程抢用输入。动作事件在对应原生按下 / 松开确认后写入，实际时序与旧版点按完成时间的差异仍需验收。

M5 增加供应用流程调用的 `CombatRun::new_segment / execute_segment`：共用外层控制句柄，先确认进入战斗，再用连续两张新帧确认普通完成或撤退。子战斗释放输入后交回结果，由外层处理结算按钮、检查点和整个应用的暂停 / 终态；不能直接将子战斗进度当作整个应用的成功。原有独立自动战斗仍持续运行到中断或超时。资源预加载、交接边界及离线验证见[情报板共用基础](remaining-game-apps.md#情报板开发中的交接规则)。

## 识别与声音

识别层复用固定 Maa 及已有闪光原始推理，统一提供角色、按钮、连携、快速支援和目标状态。识别节流在引擎内完成，候选角色失配时下一帧全量扫描。CPU 与 DirectML 继续服从适配层的串行推理和 GPU 租约，声音线程只提交状态。

Maa 模板匹配负责角色头像、普通攻击按钮及模板型角色状态；Rust 保留旧 RGB / HSV、灰度长度、掩码、连通块与轮廓组合规则。锁定目标需要嵌套轮廓的中心邻近关系，连携条需要外轮廓周长，Maa 单个识别类型未覆盖这些组合，使用 `imageproc` 补充。当前资源包括 60 名角色、527 个模板、9 个战斗入口（含开发用闪避助手）、7 个旧闪避入口，共 1656 个内容文件。只需要普通攻击按钮或闪光的脚本会跳过角色全量扫描；不需要闪光的脚本不创建闪光推理。角色、快速支援和连携候选有独立节流，候选缓存不能充当新画面的识别证明。

声音路径由 Windows 输出回录产生 PCM，转单声道与 32 kHz 后，采用四阶 1 kHz Butterworth 高通、双向滤波、无去均值标准化及 FFT 相关。配置和报告明确保留算法与阈值，靠近阈值的真实音频仍须对照旧版。设备错误应显示并使当前声音会话停止，不自动改选其他设备继续输入。

静音包显式填零；断续、积压与超过 150 ms 的停顿清空旧音频并重置检测器。回录缓冲有 0.5 秒上限。相关性按模板长度归一，严格大于阈值才触发，触发后清窗并去重。非 32 kHz WAV 使用固定窗化 sinc 重采样，回录的格式转换由 WASAPI 共享模式完成；两者与旧 librosa / 设备链路的实际差异没有用纯信号测试代替。声音默认关闭，默认阈值 0.1 仅作为旧配置起点。手柄转向使用右摇杆与显式速度参数，键鼠使用 Maa 相对位移，实际灵敏度与方向须分别标定。

## 桌面、配置与恢复

侧栏“自动战斗”读取当前账号的 `combat` 设置，支持闪避 / 完整战斗、脚本、合并文件优先级、自动终结技状态、截图间隔、CPU / DirectML、输入方式、声音和转向设置。保存后核对当前账号并确认输入，才可开始。主线程界面不参与动作定时；全局 F9 暂停 / 继续、F10 停止仍进入同一个引擎入口。运行状态与停止按钮置于页面顶部。

配置保存在数据目录 `config/accounts.yml`，账号复制、备份恢复和旧配置导入保留这些字段。旧 `battle_assistant.yml` 与 `game.yml` 中的脚本名、背景手柄、点按时长及可转换键位会迁入；无法转换的自定义键位保留原文件归档并返回警告。战斗脚本内容需在战斗页单独预览、确认导入。导入验证全部模板及合并 / 未合并入口，将原字节保存到 `combat-scripts/snapshots/<内容身份>/`，通过原子替换 `active.json` 选择；来源变化或选择变化会使旧预览失效，同名入口采用导入快照。账号备份只恢复设置，不回滚脚本快照。

启动冻结账号、设置与脚本修订，运行记录保留该修订。战斗与日常共用运行槽和控制器交接。桌面在启动前保存 v8 输入恢复计划，覆盖本次所有自定义映射；未收到同 ID、相符账号上下文与输入释放证明时保留恢复登记。释放失败阻止新任务，历史写入失败支持重试。继续使用同一运行 ID，清空旧调度代际、识别和动作队列，重新采集战斗画面；不回放暂停前输入。

## 构建与离线入口

开发构建读取参考仓和固定模型缓存，发行运行不需要 Python 或参考仓。`prepare-engine.mjs` 自动准备战斗包；Tauri、便携包和 schema 4 完整运行资源包均包含战斗资源，复制前后执行独立校验，保留 `accepted: false`。资源兼容规则见[运行资源包](runtime-resources.md)。

```powershell
cargo run -p od-tools --locked -- combat-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
cargo run -p od-tools --locked -- types
cargo run -p zzz-engine --locked -- combat-bundle-check
cargo run -p zzz-engine --locked -- combat-inspect --image zzz-od-test/screens/战斗画面/默认.webp --dodge-only
cargo run -p zzz-engine --locked -- combat-replay --scripts resources/generated/combat/scripts --script 闪避助手 --events artifacts/m4-replay-events.json
```

`combat-bundle-check` 不加载 SDK；`combat-inspect` 只对固定图片识别；`combat-replay` 只运行确定性调度，不采集音频或发送输入。回放文件是 JSON 帧数组，每帧包含 `at`、`updates`，可选 `complete`、`pause`、`resume`；状态更新包含 `name`、`at` 和可选 `value` / `add` / `time_add` / `clear`。命令输出明确保留未验收标记。真实运行入口为 `combat --hwnd <窗口> --instance-id <账号> --allow-input`，使用保存的设置，Ctrl+C 请求统一停止收尾。

## 开发验证

2026 年 10 月 20 日，独立测试仓 `zzz-od-test/test/zzz_auto/` 的 33 项 M4 Rust 检查通过：条件脚本 10、声音算法 6、映射 / 资源 / 图像规则 6、战斗运行与导入 / 资源安装 10、桌面恢复登记 1。使用真实 Maa SDK 和隔离 CustomController 覆盖了持有输入时暂停、同 ID 恢复后的重新识别、过期画面不出招、无期限按下跨 handler 保持、释放失败阻止新任务；没有操作游戏或启动真实音频回录。另通过相邻账号、备份、应用组及历史回归，5 项战斗界面回归、前端生产构建、桌面 Rust 检查与 workspace 严格 Clippy。

```powershell
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/m4 --offline --test conditional --test audio_dodge --test combat_input --test combat_runtime
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/m4 --offline --test input_recovery combat_journal
npm --prefix zzz-od-test/test/zzz_auto/frontend test -- combat.spec.ts --reporter=line
npm --prefix apps/desktop run build
cargo clippy --workspace --target-dir target/m4 --offline -- -D warnings
```

既有构建目录部分文件在当前受限账户下不可写，本轮使用单独的 `target/m4`；命名管道历史测试通过 `ZZZ_AUTO_TEST_ENGINE` 指向该目录中的当前引擎，避免调用旧协议二进制。实际状态 / 动作对照回放、声音阈值边界、输入延迟和抢占指标、GPU 跨路径观测、手柄 / 转向及两小时稳定性仍为待验收。应用清单的正式可用标记继续关闭。

本轮离线输出保存在 `artifacts/m4-rust-tests.log`、`m4-bundle-check.json`、`m4-replay.json` 与 `m4-inspect.json`。资源检查通过；五帧调度示例在暂停中不启动动作，继续后使用新代际；固定战斗图识别命中普通攻击按钮，冷启动画面年龄为 260 ms，超过默认 250 ms 限制时实际执行器会丢弃该帧。固定图识别和调度回放均报告 `input_sent: false`，不据此推断实机延迟达标。
