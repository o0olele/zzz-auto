# M3 活跃度奖励识别准备

本批迁移快捷手册每日活跃度奖励的固定区域、完成标记和离线识别。`engagement-reward-bundle` 生成独立只读包，`engagement-reward-bundle-check` 不加载 SDK 检查资源，`engagement-reward-inspect` 使用离线截图控制器检查一张 1920×1080 图片。没有游戏输入、账号配置或运行记录写入；应用登记和应用组执行入口保持关闭，协议保持 v28。

## 依据与范围

参考旧 `EngagementRewardApp` 和 `compendium.yml`：大世界进入快捷手册日常页，点击“今日最大活跃度”，处理领取确认或奖励预览，再检查第四档完成标记并返回大世界。玩法为每日无消耗领奖；旧实现以第四档标记判断活跃度已满，不能把单个“确认”文字当作领奖成功。

画面依据为参考仓的[快捷手册日常说明](../../../ZenlessZoneZero-OneDragon/docs/game/screens/快捷手册-日常.md)及独立测试仓 `screens/快捷手册-日常/默认.webp`。现有真实截图只覆盖日常页和第四档完成标记，领取确认、奖励预览、未满及可领取画面没有留档。先转换“今日最大活跃度”“活跃度奖励-确认”“活跃度奖励-奖励预览”和“活跃度奖励-4”四个区域；前三项固定 OCR 精确正则，第四项转换原二值圆形掩码为 Maa 绿色掩码模板。

只读报告分别说明文字与标记是否可见。日常选中页签和日常标题共同提供画面依据；孤立确认文字和完成标记不能确定画面，更不证明本次领取。真实日常页上已有完成标记只表示截图中的可见状态，不写当日成功。互相矛盾的日常标题与奖励预览报告为歧义，保留每项原生识别详情供检查。

日常 TAB 的黄色筛选接入方式见下节。完整领取行为继续等待缺失画面，通用 OCR 的算法差异按[识别适配](recognition-validation.md#ocr-业务调用适配)保留。

## 日常页签选中态

只读包升级为 schema 2，增加日常页签 OCR 和禁用的颜色参数节点。页签取自参考仓 `compendium_errands.yml` 的 `TAB-日常`：旧区域 `[708,140,1636,220]` 转为 Maa ROI `[708,140,928,80]`，RGB 范围为 `[200,160,0]` 到 `[255,255,60]`，由 Maa 原生 `OCR.color_filter` 按 RGB 进行二值化。页签文字仍采用精确正则，不宣称与旧 LCS 包装器逐参数等价。

生成前同时校验日常画面的标识、区域、文字、颜色、识别标记与跳转目标，变化时拒绝写出。五项识别和一个无后继的禁用颜色节点仍只做诊断，包内文件数量不变；旧 schema 1 诊断包需重新生成，不影响现有执行资源包和用户当前版本选择。

报告增加 `daily_tab_selected`。只有黄色日常页签和日常标题同时出现、且没有矛盾的奖励预览标题，才报告 `daily_visible`；第四档标记也只在这个条件下提供日常页上的可见完成状态。孤立标题、孤立页签及拼接了其他选中页签的日常正文都不能提供该状态；确认文字仍不能单独定位画面。本项不表示已领取、未满或本次成功。

## 资源约束

生成前检查来源区域、固定模板和掩码、OCR 模型与字典，验证完全部来源后才写输出。来源与输出不能重叠，拒绝目录联接、符号链接及输出中的非受管文件。清单记录来源哈希、区域映射、已知限制和全部包内文件的大小及 SHA-256。

引擎仅接受固定五个识别节点、一个禁用颜色节点、五个受管文件及清单，不允许输入动作、有效后继、额外文件或替换模型、字典、模板；即使重算清单也不能扩大执行能力。该包是开发诊断资源，不加入当前执行资源 ZIP，也不修改用户的资源选择。

## 使用与后续

```powershell
cargo run -p od-tools --locked -- engagement-reward-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
cargo run -p zzz-engine --locked -- engagement-reward-bundle-check --bundle resources/generated/engagement-reward-preview
cargo run -p zzz-engine --locked -- engagement-reward-inspect --bundle resources/generated/engagement-reward-preview --image zzz-od-test/screens/快捷手册-日常/默认.webp
```

后续需补可领取、领取确认、奖励预览及未满真实素材，核对识别与返回路径后，再实现有检查点的领取流程、暂停继续、防重复及应用组每日记录和顺序交接。本批离线识别不作为这些行为或实机通过的证据。

## 验证

新增 12 项 Rust 回归，覆盖真实日常页的标题与完成模板、目标 / 作战 TAB、菜单、空白图、孤立完成标记、歧义判断、错误尺寸及损坏图片、来源区域和模板变化、模型校验、重复生成、输出保护、清单路径和版本，以及重算清单后的动作、后继、识别参数、模板和字典替换拒绝。应用组计划继续拦截活跃度奖励。连同每日计划 7 项和顺序调度 6 项，共 25 项通过。

真实 SDK 使用 CPU 和锁定 ppocrv5；日常标题命中，第四档模板命中，确认及奖励预览均未命中。CLI 检查在指定 SDK 目录不存在、隔离数据目录的资源选择文件损坏时仍能完成，不加载 SDK 或创建账号和记录；离线识别仅产生指定目录的 Maa 日志。CLI 输出留存在 `artifacts/engagement-cli-inspect.json`。

```powershell
cargo build --locked --target-dir target/engagement-validation -p zzz-engine -p od-tools
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/engagement-validation --locked --test engagement_reward --test daily_runs --test group_schedule -- --test-threads=1
cargo clippy --locked --target-dir target/engagement-validation -p od-core -p maa-adapter -p od-tools -p zzz-engine --all-targets -- -D warnings
```

相关构建、严格 Clippy 和格式 / 差异检查通过。首轮回归的固定报告目录写入受本机权限限制，测试改为输出诊断详情后重跑通过；真实留档识别本身已通过。前端、桌面命令、配置 schema 和资源包兼容范围未变，本轮没有重打包客户端或执行游戏领取。

### 日常选中态验证

2026 年 10 月 19 日新增 7 项回归，覆盖页签来源缺失、重复及规则变化时拒绝写出，重算哈希后的颜色 / 页签节点修改拒绝，真实日常正文与目标 / 作战页签拼接，孤立正文和页签，白色 / 蓝色标签，连续换图及奖励预览歧义。活跃度共 19 项通过，连同 OCR 12 项、每日计划 7 项、顺序调度 6 项共 44 项通过；相邻 OCR 的 CPU / DirectML 回归保留原算法差异检查，没有放宽容差。

原生工具已重新生成 schema 2 只读包，指定不存在的 SDK 和隔离数据目录时，纯资源检查通过且未创建该数据目录。原生 CLI 分别检查三张真实留档：日常页的黄色页签、标题和可见完成标记同时命中，目标与作战页不提供日常上下文。完整 CPU 报告存放于 `artifacts/engagement-selected-tab-{daily,target,combat}.json`，测试仓 `fixtures/recognition/engagement-selected-tab-20261019.json` 记录截图、报告和实现指纹及分案结果。新包未单独验证 DirectML，不将相邻 OCR 的设备证据当作本包验收。

引擎及工具构建、相关 crate 与活跃度测试严格 Clippy、格式和两仓差异检查通过。协议保持 v28，前端、执行资源包及账号配置未改动；没有执行真实游戏领取或新增实机通过结论，完整领取继续等待奖励画面。
