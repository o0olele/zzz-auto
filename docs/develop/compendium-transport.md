# 快捷手册到实战模拟室的传送

M5 体力执行新增内部传送子流程：大世界按住 Alt 显示按钮，重新截图后打开快捷手册，切换训练，点击实战模拟室分类，在同一帧中定位冻结类型及其“前往”按钮，确认传送后重新读取模拟室入口。父流程取得 `TransportEnd::Arrived` 后，仍须选择关卡、核对等级、准备卡片、选队并处理电量和战斗。到达入口不增加计划次数，也不写应用成功记录。

依据为参考仓的快捷手册、训练、大世界及模拟室 screen_info、`TransportByCompendium`、三个手册选择操作，以及快捷手册和实战模拟室画面文档。独立来源清单同时锁定副本目录与模拟室识别依据。训练选中态沿用原生 RGB 黄色过滤，完整匹配“训练”；所有页签的普通文字不能证明选中。

本轮范围是传送入口。普通四类材料按固定类型名称寻找，旧 OCR 的“代理人晉升”仅兼容完整名称。自定义模板沿用旧版先传送基础材料的路线，同时保留原请求的“自定义模板”，交给[类型与关卡选择投递](simulation-mission.md)；后者重新选择类型，投递回执仍待选中态验证。代理人方案培养依赖尚未迁移的头像目标选择，进入流程前明确拒绝；不能静默采用基础材料或其他目标。

“前往”按钮必须与目标标题出自同一帧，位于标题下方、下一条标题上方且相距小于 180 像素。重复目标、同一条目内多个按钮、目标按钮不见或只出现相邻条目的按钮均不能点击。目标不在当前列表时，沿旧区域向上拖动列表，最多 20 次；没有已知模拟室类型的未知列表不滚动。实际输入前重新截图，再做页签、目标和按钮配对；动态坐标不保存在检查点中，超过 1500ms 的帧拒绝输入。

只有本次已保存的“前往”意图，才能采用完整“是否立即传送？”和固定区域的“确认”。普通确认或恢复、获得弹窗优先拦截背景。直接在既有确认弹窗或模拟室详情上启动不取得本次传送证明。传送确认投递之后只等待新目的地，不重复点击确认；目的地必须是模拟室类型列表或带实际电量的挑战详情，出战、编辑及结算画面不能当作到达。

`od_core::compendium::TransportFlow` 保存冻结类型、滚动次数、动作意图和实际目的地版式；`maa_adapter::compendium` 提供只读识别、资源校验与同帧输入；`zzz_engine::compendium::TransportRun` 复用父控制器、暂停停止句柄及剩余活动预算。每次输入前保存意图，明确未投递才重试；原生错误保留不确定意图。暂停后重读画面，搜索阶段重新点击分类，已投递“前往”或确认不重放。终态保存期间收到暂停也撤销旧到达证明，继续后重新读取实际电量。

保存、截图、识别、输入和释放均计入预算；保存失败、停止或未能确认输入释放均阻止父流程继续。到达不会终结共享控制句柄。本子流程采用键鼠控制器；后续完整体力入口负责游戏控制权、键鼠与战斗控制器交接，以及桌面输入恢复登记。当前没有增加公开游戏输入入口或修改运行槽。

独立 schema 1 资源包含固定节点、三个已转换的大世界模板与锁定 OCR，保留逐文件大小、哈希、来源和目录边界检查。重新填写清单哈希不能替换节点、模板或来源。现有日常、模拟室卡片、恢复电量及随便观资源定义保持原义，协议仍为 v45，导入预览仍为 v18。

```powershell
cargo run --locked -p od-tools -- compendium-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
target/debug/zzz-engine.exe compendium-bundle-check --bundle resources/generated/compendium
target/debug/zzz-engine.exe compendium-inspect --bundle resources/generated/compendium --image zzz-od-test/test/one_dragon/base/screen/screen_loader/test_get_match_screen_name/compendium_train.webp --mission-type 基础材料
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test compendium --test compendium_execution -- --test-threads=1
```

校验命令在加载 SDK 及解析用户资源选择前执行；截图检查固定 CPU，只读报告保持 `accepted=false`、`input_sent=false`。真实留档覆盖大世界、训练页和其他页签，模拟室目标条目、传送确认和详情使用明确组合帧。真实传送、确认版式、后台及权限场景和异常恢复继续待验收。[关卡定位与选择投递](simulation-mission.md)已接续开发，回执仍待验证；后续补齐关卡选中态证明，再串联卡片、电量恢复、选队和战斗结算，开放完整体力应用。
