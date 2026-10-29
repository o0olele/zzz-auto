# 其他体力副本的快捷手册传送

本轮补齐区域巡防、专业挑战室和恶名狩猎的内部传送子流程。它使用完整的冻结计划，保留旧区域名称原文，只在执行时解析兼容名；恶名狩猎名称采用冻结目录中的明确别名。代理人方案培养需要头像与培养目标识别，继续独立开发，不能通过普通副本路线绕过。

深度追猎父任务允许传送后落在其他已知恶名详情。到达回执保存实际标题，仍为 `selection_verified=false`；它只说明本次进入恶名入口。消费父任务必须复核冻结目标，必要时保存返回列表意图并完成唯一名称选择，详见[深度追猎](deep-hunt.md)。其他分类、未知标题和已有弹窗仍不能作为到达或消费证明。

传送顺序为大世界打开手册、选择训练、确认所需分类、找到完整副本名称及同一条目的唯一前往按钮、确认传送、重新读取入口。训练黄色选中态与分类蓝色选中态分别识别。名称只做空白和装饰标点归一化，不做相似度猜测；未知、重名、重复前往、其他分类或未选中页签均不授权前往。滚动最多 20 次，每次仍须确认当前选中的分类及已知条目。

真实专业挑战训练页核对发现，直接用蓝色过滤再 OCR 会丢失白字。分类现采用原色完整文字定位，并在同帧文字框扩边区域内用 Maa ColorMatch 确认至少五分之一像素为登记的蓝色范围；候选框限制在分类列表内，不采用邻近头像或其他行的颜色。普通文字、蓝底及所选分类的完整条目同时满足后，才允许前往。资源详情、恢复电量及已知确认 / 奖励覆盖层先于手册背景判断。

所有输入先保存意图，实际动作再用 1500ms 内新帧复核。同一运行控制句柄和活动预算跨暂停保留，前往与确认的不确定投递不重放。开始时已有确认弹窗或副本入口不能被当作本次传送的证据；需要当前已保存的前往与确认路径。终态保存失败、暂停、停止、超时或未释放输入时不提供到达回执。

区域巡防和专业挑战入口分别读取其来源中登记的电量区域，不能套用模拟室卡片坐标。恶名狩猎分别识别列表中的奖励次数锚点，或带街区按钮及完整副本标题的详情页。到达回执仍是 `selection_verified=false`，不能作为消费或成功计数的证明；等级、专业挑战燃竭开关、深度追猎模式、鸣徽、准备、战斗与结算仍须各自接通。体力父流程的整组预检继续阻止缺少消费执行器的计划，不会在传送之后才发现无法执行。

新资源使用独立 schema 1 和固定模型、模板、节点、来源校验。旧模拟室传送包和序列化结构保持原义。验证采用现有真实专业挑战训练页及明确标注的分类、确认、到达组合图，不启动游戏，不把组合回放计为实机验收。

```powershell
cargo run --locked --target-dir target/m4 -p od-tools -- charge-transport-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
target/m4/debug/zzz-engine.exe charge-transport-bundle-check
target/m4/debug/zzz-engine.exe charge-transport-inspect --image zzz-od-test/test/one_dragon/base/screen/screen_loader/test_get_match_screen_name/compendium_train.webp --category 专业挑战室 --mission-type 恶名·装甲哈提
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test charge_transport --test compendium --test charge_catalog -- --test-threads=1
```

新增 13 项及相邻 14 项共 27 项 Rust 回归通过，涵盖三条路线、别名、分类与前往配对、暂停后不重复前往 / 确认、新帧、目标与覆盖层变化、保存失败、停止、滚动上限、入口类别及资源重哈希篡改拒绝。最终已知电池详情覆盖层另外复验同一用例通过，不重复计数。严格 Clippy、引擎 / 工具 / 原生桌面构建和只读 CLI 通过，详见[验证记录](../migration/acceptance.md#2026-年-10-月-29-日-m5-三类体力副本传送)。
