# M3 一条龙运行页

独立的“一条龙运行”页集中展示当前账号、保存的应用组顺序、运行计划和当前任务。账号与应用组页继续负责配置编辑；总览提供连接游戏及进入运行页的入口。

## 运行约定

页面只读取当前账号的配置，按保存顺序选择一个应用组。切换账号和编辑启用项仍在“账号与应用组”完成。选择应用组不保存配置，也不开始游戏输入；启动继续使用引擎的完整预检、每日成功判断、修订复验和本次账号及输入确认。当前支持邮件与咖啡，其他启用项由整组预检拦截。

进入页面时，优先展示正在执行的应用组；执行、暂停和停止收尾期间固定组选择。当前任务读取引擎快照中的账号、组名、运行 ID 和分项进度，独立于配置读取。读取配置失败时仍可暂停、继续、停止已有应用组或重试保存记录。其他诊断任务占用运行槽时，提示进入原生诊断，禁止另行开始应用组。

重新读取配置会撤销原计划与输入确认，并保留仍存在的所选组；切页后重新选择，正在执行的组优先。断线、窗口及截图方式变化同样需要重新预览。配置和计划的迟到响应不能覆盖新的页面状态，即使保存状态已变回请求时的值，旧响应也会作废。保存错误保留已经显示的计划供核对，但撤销确认并拦截启动和继续。重进页面从引擎快照读取同一任务，不自动开始或继续运行。独立运行页初版协议为 v25；接入结束后动作后为 v26，资源内容不变。

## 使用

运行页已接通[结束后动作](after-done.md)的全局保存和计划确认。正常完成后由宿主倒计时，任意页面及托盘都可取消；本页选择改变后须先保存或恢复原值，再预览并启动。

1. 在“账号与应用组”选择与实际游戏登录对应的当前账号，保存启用项、顺序及咖啡参数。
2. 在总览连接游戏，从总览按钮或侧栏进入“一条龙运行”。
3. 选择组；如只运行部分应用，开启“仅本次选择应用”并勾选。点击“预览运行计划”，核对游戏日、每日跳过、实际参数和阻止原因。
4. 如需重试本游戏日已完成的应用，开启“仅本次重新运行已完成应用”后重新预览；核对本次重跑及结束后动作，勾选账号和前台输入确认后启动，在启动延时内切回游戏。
5. “当前任务”展示运行时账号、组、分项及输入状态，可暂停、继续或停止；“查看完整运行记录”进入历史筛选与导出。

## 仅本次选择应用

运行页提供可关闭的“仅本次选择应用”。默认沿用保存的启用项；开启后从这些启用项初始化，可临时勾选当前组内的其他应用、全选或恢复保存的启用项。选项只保存在当前页面，刷新配置、换组、断线或离开页面后恢复默认，不写账号配置，不改变组的保存顺序。没有选择时禁止预览和启动。

协议 v29 在原预览及启动命令中增加可省略的 `app_ids`。省略或 null 沿用已保存启用项；显式列表必须非空、至多 256 项、不重复且全部属于所选组。列表是本次启用项的完整替代，允许临时选择已保存为禁用的应用；实际顺序始终由组决定，不能通过请求重新排序。未迁移或未知应用被选中时照常拦截，只在本次未选择它们时排除。

预览和启动由同一引擎函数计算；启动再次读取修订、游戏日和本次列表，沿用账号与输入确认、运行互斥及完整资源预检。计划的 `enabled` 表示本次是否选择，运行上下文的 `app_ids` 只冻结本次列表；每日成功、历史、提醒及结束后动作均依据本次实际执行结果。全部已完成时不启动，也不触发结束后动作；有执行项时仍需在计划中确认已保存的结束后动作。

勾选变化立即撤销旧计划和输入确认，迟到响应即使对应相同的最终选项也不恢复。执行期间选择冻结；切页返回从快照显示同一任务及实际应用，不能把页面默认选项当作正在运行的内容。账号页原有整组试运行继续使用省略列表的命令，旧运行资源包保持兼容。

## 仅本次重新运行已完成应用

运行页增加“仅本次重新运行已完成应用”。默认关闭，沿用应用组保存的运行策略；开启后，本次选择中的所有可运行应用都会执行，包括本游戏日已有成功记录的应用。可与“仅本次选择应用”一起使用，仍按保存顺序执行。刷新配置、切组、离开页面或断线会撤销临时选择，运行期间不能修改；任务自身的标记从引擎快照读取，切页返回仍可查看。

预览保留长期 `run_policy` 和每项已有的成功运行 ID，单独提示“本游戏日已完成，本次重跑”，输入确认也明确包含已完成项。此操作不重置每日记录、不增加游戏内领取次数。取消开关后重新预览，恢复原有每日跳过；未知或未迁移应用、空列表、非当前账号、资源缺失、未释放输入和未保存结果继续拦截。结束后动作沿用已保存的设置，重跑正常成功后仍会按确认内容倒计时。

保存为“每次运行”的组开启本次重跑时同样读取每日记录，显示此前成功证明；关闭临时重跑后仍沿用原有每次运行行为。

协议升级为 v30，原预览和启动命令增加可省略的布尔值 `rerun_completed`。省略或 false 沿用原行为，null、数字及文字不作为布尔值接受。预览和启动使用同一个 `GroupRunOptions` 计划计算入口，启动重新读取修订、账号、游戏日、所选应用和每日证明。`RunContext.rerun_completed` 冻结到当前任务和持久历史中，缺少该字段的旧记录按 false 读取；false 不写入 JSON，已有配置文件不变。桌面核对返回标记，开关往返变化后旧预览与输入确认失效。账号页整组试运行保持默认行为，旧执行资源包继续兼容。

重跑会新增独立运行记录，保留此前成功记录。只有本次实际成功且完整保存、输入释放的分项才能成为新每日证明；失败或停止的重跑不撤销此前成功。原有暂停、继续、停止、运行槽、恢复登记、提醒和结束后动作共用原实现。

## 验证

新增 10 项独立测试仓界面回归，覆盖当前账号和组顺序、只读导航、预检与输入确认、每日全部完成及未知项、刷新和切组使计划失效、同 ID 控制与切页返回、保存失败、配置读取失败及等待时的任务控制、其他任务占用、未释放输入、断线及迟到响应、旧修订和浏览器预览。连同既有账号与设置、外观、历史共 63 项回归通过；修正刷新重置所选组及保存失败展示兼容问题后，针对性复跑 11 项通过。明暗主题、960 像素宽布局及运行中分项截图已检查。

```powershell
npm --prefix zzz-od-test/test/zzz_auto/frontend test -- one-dragon-run.spec.ts accounts.spec.ts run-history.spec.ts appearance.spec.ts --reporter=line
npm --prefix apps/desktop run build
cargo build --locked -p zzz-desktop --features custom-protocol
```

生产前端和包含静态前端的原生 debug 桌面构建通过，保留已有大于 500KB 单块提示。本批只使用模拟引擎与隔离浏览器数据，没有操作真实游戏或修改真实账号配置。真实 WebView2 和游戏运行入口待实机验收；咖啡完整领取、实际游戏交接和原 M1 / M2 边界继续保留，M3 仍进行中。

### 本次选择验证

2026 年 10 月 19 日新增 7 项核心及 2 项真实 SDK 模拟控制器回归，覆盖非空 / 重复 / 越界列表、组归属、保存顺序、临时启用、未知和未迁移项、当前账号、确认、修订、游戏日、实际单邮件执行、配置原文不变及只有所选应用的每日证明。按日策略下再次预览为全部完成，重复启动拒绝且无额外输入。宿主恢复回归同时验证带本次列表的命令仍在投递前登记 v6 全部输入，并保留暂停 / 继续收尾规则。

连同账号运行 8 项、每日计划 7 项、顺序调度 6 项、邮件原有 27 项、输入恢复 1 项及资源兼容 1 项，共 59 项 Rust 回归通过；资源兼容逐一检查此前协议的两类包及当前协议 29。初次邮件打包检查读取旧 v28 引擎而失败，测试入口补充已有约定的 `ZZZ_AUTO_TEST_ENGINE`，显式指定本轮配套引擎后通过；没有放宽协议断言。测试中两处既有 Clippy 提示一并修正或对 Windows 只读标记恢复作局部说明。

新增 7 项界面用例，连同运行页原 16 项、账号 37 项、历史 7 项及提醒 8 项，共 75 项通过。覆盖仅本次勾选及不写配置、按保存顺序提交、当前任务真实列表、空选择、全选与恢复、撤销确认、选项往返迟到响应、刷新 / 换组 / 离页 / 断线清空、每日跳过、未知应用及错误响应；明暗主题和 960 像素宽布局已检查，截图在 `artifacts/m3-group-selection-{light,dark-small}.png`。

前端生产、配套引擎及包含静态前端的原生 debug 桌面构建、主 workspace 和相关测试的严格 Clippy 通过。本机默认构建缓存写入受限，沿用独立 `target/engagement-validation` 并核对更新后的随附引擎哈希；没有使用旧二进制宣称当前协议通过。真实用户配置、WebView2 及游戏操作仍待实机验收。

```powershell
$env:ZZZ_AUTO_TEST_ENGINE = (Resolve-Path target/engagement-validation/debug/zzz-engine.exe).Path
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/engagement-validation --locked --test group_selection --test group_runs --test group_schedule --test daily_runs --test mail_pipeline -- --test-threads=1
npm --prefix zzz-od-test/test/zzz_auto/frontend test -- one-dragon-run.spec.ts accounts.spec.ts run-history.spec.ts run-notifications.spec.ts --reporter=list
```

### 本次重跑验证

2026 年 10 月 19 日新增 3 项核心及 2 项真实 SDK 模拟控制器回归。覆盖默认关闭与旧记录读取、严格布尔值、临时应用选择、原顺序与每日策略、已有成功证明、未知应用及账号 / 修订 / 游戏日 / 输入确认拦截。真实 SDK 邮件流程先正常完成，再以新运行 ID 重跑；再次失败后仍读取前一次成功证明，恢复默认运行时继续按日跳过。重跑前后配置原文字节相同；保存为“每次运行”的组开启重跑也展示已有成功证明。

连同账号运行、每日计划、顺序调度、邮件、历史、结束后动作、结果提醒、宿主输入恢复及资源兼容，共 101 项 Rust 回归通过。资源兼容逐一复验 schema 1 的协议 15..30 和 schema 2 的协议 16..30，拒绝未知协议及 schema 2 的协议 15。宿主回归确认带重跑标记的请求在投递前仍登记 v6 全部恢复输入。初次管道回归误用旧版本引擎，明确指定本轮 `ZZZ_AUTO_TEST_ENGINE` 后通过；结果提醒入口同样补齐该测试变量支持，没有放宽版本校验。

新增 5 项界面用例，连同运行页原有 23 项、账号 37 项、历史 7 项及提醒 8 项，共 80 项通过。覆盖已完成项重跑、本次选应用组合、配置不变、确认文案、运行中冻结、同一任务切页回看、开关往返迟到响应、刷新 / 换组 / 离页 / 断线重置、返回计划标记不符、未知应用及未保存结束后动作拦截。明暗主题及 960 像素宽布局截图已检查，保存在 `artifacts/m3-rerun-completed-{light,dark}.png`。

生产前端、配套引擎及包含静态前端的原生 debug 桌面构建、主 workspace 与本次相关测试的严格 Clippy、主仓格式检查通过。前端保留已有大于 500KB 单块提示。引擎使用 `target/engagement-validation`，桌面使用 `target/rerun-validation`；随附引擎与新构建引擎 SHA-256 一致。本轮使用临时目录、模拟控制器及隔离浏览器，没有操作真实游戏或修改真实账号配置；WebView2 和游戏重跑仍待实机验收，咖啡与活跃度领取素材缺口继续保留。

额外执行独立测试仓的全量 `cargo test --no-run` 未通过：既有 `desktop_runtime_resources.rs` 仍使用旧版 `ResourceManager::new` / `install` 和缺少 `archive` 的安装请求，尚未适配此前的宿主运行时与安装接口调整。这些文件及接口不属于本次变更；全量编译不计为通过，本次相关测试单独编译及运行通过。

```powershell
$env:ZZZ_AUTO_TEST_ENGINE = (Resolve-Path target/engagement-validation/debug/zzz-engine.exe).Path
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/engagement-validation --locked --test group_selection --test group_runs --test group_schedule --test daily_runs --test mail_pipeline --test run_history --test after_done --test run_notifications -- --test-threads=1
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/engagement-validation --locked --test input_recovery account_group_arms_union_journal_before_send_and_allows_readonly_queries -- --test-threads=1
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/engagement-validation --locked --test runtime_resources shortcut_protocol_preserves_existing_resource_protocols_and_rejects_unknown_versions -- --test-threads=1
npm --prefix zzz-od-test/test/zzz_auto/frontend test -- one-dragon-run.spec.ts accounts.spec.ts run-history.spec.ts run-notifications.spec.ts --reporter=list
npm --prefix apps/desktop run build
cargo build --locked --target-dir target/rerun-validation -p zzz-desktop --features custom-protocol
```
