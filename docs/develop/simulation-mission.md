# 实战模拟室类型与关卡选择投递

本内部子流程接续[快捷手册传送](compendium-transport.md)，处理普通四类材料及自定义模板的类型重选、关卡定位、横向翻页和选择投递；二者已接入同一[传送与选关父流程](simulation-approach.md)。只有定位与投递回执已经开发完成；现有真实留档缺少可靠的关卡选中标记，因此结果固定为 `MissionEnd::AwaitingVerification`，回执的 `selection_verified` 始终为 `false`。父流程不能据此进入卡片、电量消费、选队或出战，也不能写计划次数和应用完成。

依据为参考仓 `CombatSimulation` 的类型选择、目录顺序翻页和文字框左上角向下 50 像素的点击规则，以及 `combat_simulation.yml`、`menu.yml`、战斗画面和副本目录。独立资源固定六份参考来源、原有返回模板及锁定 OCR；没有推测新的选中颜色、边框或关卡标题区域。

入口可以是模拟室类型页或挑战详情。即使传送已进入详情，也先按有模板证明的返回按钮回到类型页，重新选择冻结类型，避免采用游戏保留的旧类型。自定义模板走相同步骤；代理人方案培养因缺少头像目标识别而预检拒绝。`MissionRequest::from_plan` 校验计划所属分类及名称，选择器只负责类型和关卡，不处理等级。

关卡 OCR 只读取原有 `[128,724,1668,158]` 区域，使用完整名称匹配，区分“命破演练”和“击破演练”。文字框必须完整位于列表内，点击偏移也不能超出列表；动态位置只存在于当前帧。重复名称、重叠框、反向目录顺序及其他类型的已知关卡均拒绝选择。目标在可见目录范围前后时才向相应方向翻页，夹在已识别名称之间却未读到、没有已知名称或仅有残缺文字时继续等待，不推测目标位置。最多十次翻页，暂停不重置次数或活动预算。

每次输入先保存意图，再重新截图核对页面、冻结请求及当前目录顺序，拒绝超过 1500ms 的帧。明确未投递才重试同一步；原生错误保留不确定意图。选择已投递或可能已投递时，暂停继续仅重新读取详情，不重复选择；其他阶段继续会重新建立类型上下文。投递后需在新帧中再次读到目标名称及实际电量，才保存“投递后详情可读”的回执。它仍不能证明目标已选中。

`od_core::simulation_mission::MissionFlow` 保存请求、翻页次数、意图和未验证回执；`maa_adapter::simulation_mission` 复用原模拟室页面识别并提供固定区域和同帧输入；`zzz_engine::simulation_mission::MissionRun` 共用父控制器、暂停停止句柄及剩余预算。保存、识别、输入及释放均计时，终态保存期间的控制请求、超时或失败阻止返回正常回执。停止不能被继续覆盖，输入释放失败阻止后继；子流程不终结父控制句柄。

独立 schema 1 资源目录为 `resources/generated/simulation-mission`，清单为 `simulation_mission-manifest.json`。模型、模板、节点、来源及目录边界按固定清单检查，重新填写清单哈希不能替换节点。原模拟室卡片、快捷手册及恢复电量资源保持原义；协议仍为 v45，导入预览仍为 v18，没有增加桌面或 CLI 的真实输入入口。

```powershell
cargo run --locked -p od-tools -- simulation-mission-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
target/debug/zzz-engine.exe simulation-mission-bundle-check
target/debug/zzz-engine.exe simulation-mission-inspect --image zzz-od-test/screens/实战模拟室/战斗结果-获得奖励.webp
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test simulation_mission --test simulation_mission_execution -- --test-threads=1
```

校验在 SDK 和用户资源选择之前执行；只读识别固定 CPU，报告保持 `accepted=false`、`input_sent=false`、`selection_verified=false`。当前真实领奖留档只作为错误页面反例；类型、关卡、翻页和选择后态使用明确标注的组合图及隔离控制器，不能计为真实游戏或性能验收。后续需补关卡选中态素材与识别证明，再串联等级、卡片、电量恢复、选队、战斗及体力应用运行登记。
