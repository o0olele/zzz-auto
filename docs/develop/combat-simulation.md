# 实战模拟室识别与卡片准备

体力执行在电量恢复之后，先补齐实战模拟室的固定识别与卡片准备。[快捷手册传送](compendium-transport.md)及[类型与关卡选择投递](simulation-mission.md)已提供内部接口；关卡投递尚不提供选中证明，父流程继续负责确认目标与冻结计划。本子流程只在已经进入副本详情后调整卡片数量，并交回实际读取的当前、所需电量。它不点击下一步、出战或再来一次，不消耗电量，也不增加计划次数。

依据为参考仓 `docs/game/screens/实战模拟室.md`、`assets/game_data/screen_info/combat_simulation.yml`、`battle.yml` 和 `src/zzz_od/operation/compendium/combat_simulation.py`。旧版每次点击第一张已选卡片五次以清空，再点击第一张候选卡片指定次数，最后保存方案。数量范围 1..5，每张需要 20 电量；默认数量保持游戏当前方案，必须读取实际所需电量，不能用进本前的 20 电量估算代替。

识别同时保留类型列表、副本详情、卡片编辑、出战、普通结算及实战模拟室领奖弹窗。弹窗先于背景；单独“确认”、普通奖励或未知数字不提供可输入画面。卡片编辑仅以固定“保存方案”区域识别，数量操作使用已有三个固定卡片区域，未推测新的颜色或选中边框。详情要求挑战等级、下一步及独立可读的电量；保存后的所需电量必须等于本次卡片数量乘 20 才交回。该证明只表示数量已准备，不证明副本目标、选队或战斗成功。

类型列表要求至少三个不同的固定目录类型，重复文字不能凑数。固定 OCR 在组合帧上将“代理人晋升”读为“代理人晉升”，仅对这个完整类型名称兼容繁体字；其余文字仍完整匹配，电量仅接受十进制整数，不能把“1.00”改写为 100。

每次动作先保存检查点，再由适配器重新截图核对同一页面，拒绝超过 1500ms 的帧。只在确认未发送输入时沿原步骤重试；暂停后放弃未完成的计数，从可见编辑页重新清空并选择，或核对已保存的详情页。保存之后数量不符则停止。父子流程共用控制器、暂停停止及剩余活动预算，保存失败、停止、预算耗尽或输入释放失败均阻止后继。

完成电量读取但尚未交回父流程时收到暂停，继续后也撤销旧证明，重新核对页面及实际电量；不能把暂停前结果直接交给选队或恢复流程。[选队准备](simulation-teams.md)已提供从出战页开始的内部接口，不负责详情页到出战页的推进或消耗电量。[出战与子战斗交接](simulation-deploy.md)要求另有可靠的关卡选中证明，不能采用当前未验证回执。

新增独立 schema 1 资源及只读命令，固定参考来源、OCR 模型和全部节点；既有日常、随便观和恢复电量资源保持原义，协议仍为 v45。真实留档目前只有战斗结果获得弹窗；详情、编辑、类型列表及出战的开发验证使用明确标注的组合帧，不计为实机验收。快捷手册传送内部入口已接通，目标副本确认、选队、电量恢复串联、战斗结算与体力应用运行登记继续接入。

`od-core::simulation_cards::CardsFlow` 保存冻结数量、清空 / 添加计数、投递意图及实际电量；`maa_adapter::simulation` 负责只读识别、固定资源校验和同帧输入复验；`zzz_engine::simulation_cards::CardsRun` 继承父预算与控制句柄，完成后仅返回 `CardsEnd::Prepared`。父流程须再核对目标副本和编队，按实际电量决定是否调用恢复子流程，不能把卡片准备当作副本已完成。

```powershell
cargo run --locked -p od-tools -- simulation-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
target/debug/zzz-engine.exe simulation-bundle-check --bundle resources/generated/simulation
target/debug/zzz-engine.exe simulation-inspect --bundle resources/generated/simulation --image zzz-od-test/screens/实战模拟室/战斗结果-获得奖励.webp
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test simulation_cards --test simulation_cards_execution -- --test-threads=1
```

`simulation-bundle-check` 在 SDK 加载及运行资源选择前执行；`simulation-inspect` 固定采用 CPU，只读命令保持 `accepted=false`、`input_sent=false`。普通战斗结算只报告版式，不能代替战斗进入及连续新帧结算证明。开发验证的 GDI 数字使用适合原区域的字号，不从小数或残缺数字推测电量；不放宽真实识别的数字解析或置信度。

[卡片与电量恢复串联](simulation-preparation.md)已提供共用父控制器、冻结缺口和返回详情复验的内部接口。[结算与领奖收尾](simulation-settlement.md)须接续同一父任务的已确认战斗结束结果和输入释放。这些接口不写计划次数；关卡选中证明、选队及战斗的完整串联仍待接入。等级切换缺少可核对画面，不能从“挑战等级”标题推断当前等级。

2026 年 10 月 23 日新增用例、构建结果及相邻制造回归的三项未通过情况见[开发验证记录](../migration/acceptance.md#2026-年-10-月-23-日-m5-实战模拟室识别与卡片准备)。这些结果不作为真实游戏或性能验收。
