# 模拟室卡片与电量恢复串联

本内部流程串联已有卡片准备和恢复电量执行器。先完成卡片方案，在新的详情帧读取实际当前及所需电量；电量充足时返回准备结果，关闭恢复时返回明确不足结果，否则保存“下一步”的意图并复验同一需求及当前电量，等待实际恢复主弹窗后交接恢复执行器。恢复或取消后必须重新看到带实际电量的副本详情。

依据为参考仓的实战模拟室、恢复电量画面文档、两份 `screen_info`、`CombatSimulation` 和 `RestoreCharge`；恢复来源、最小兑换量与严格持有量规则沿用[共用子流程](restore-charge.md)。只点击“下一步”打开不足弹窗，不点击出战或再来一次。若投递后意外进入出战页或其他页面，拒绝正常交回；弹窗没有出现时不重复点击。

卡片和恢复检查点、有序交接阶段、下一步意图、冻结缺口及实际结果共同保存。父子使用同一控制器、暂停停止句柄和剩余活动预算；暂停不重置预算或已提交消费。未投递回执才允许重试下一步，暂停继续重新检查最终详情，保存失败、预算耗尽和输入释放失败阻止后继。全部子流程资源在首个输入前校验。

结果只证明本次数量和电量，保留 `selection_verified=false`。完整父任务须在调用本流程之前确认同一冻结关卡已选中；现有类型与关卡投递回执无法提供这一前置条件，因此本轮不开放体力应用运行、不写计划次数或每日记录。非默认等级与代理人方案培养仍预检拒绝。

复用原 schema 1 模拟室及恢复电量资源的固定只读节点，不修改节点、来源或模型定义，不新增外部输入入口；协议 v45、导入预览 v18 保持原义。实际恢复弹窗采用已有真实留档，卡片、详情和交接使用明确组合帧，开发回放不计为实机验收。

`od_core::simulation_preparation::PreparationFlow` 保存冻结的卡片数量、恢复策略、实际缺口及两个子流程检查点；`zzz_engine::simulation_preparation::PreparationRun` 复用原 `CardsRun` 和 `RestoreRun`。`PreparationRequest::from_plan` 校验模拟室目录及默认等级，`PreparationResult` 分开返回 `Ready`、`Disabled` 和 `Insufficient`，始终保留 `selection_verified=false`。已完成检查点不能直接构造新任务采用旧电量证明。

下一步输入前重新截图，只接受与本次冻结缺口完全一致的当前 / 所需电量，并保留原 1500ms 帧龄上限。明确未投递时撤销意图并重新读电量；投递或不确定投递后保留意图，暂停不重开弹窗。子流程回调只读取调用方传入的同一张图，要求原挑战等级、下一步及独立可读电量；恢复弹窗、普通确认、领奖和编队页均不提供返回证明。

```powershell
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test simulation_preparation --test simulation_preparation_execution -- --test-threads=1
```

资源仍通过 `simulation-bundle`、`restore-charge-bundle` 生成和既有只读命令校验。没有新增桌面或 CLI 的真实输入入口；调用者必须检查 `PreparationEnd::Completed` 及输入释放，再采用结果，不能只检查内部快照的完成字段。

2026 年 10 月 24 日，新增核心 8 项和真实 Maa SDK 隔离执行 12 项通过，连同卡片准备 23、恢复电量 22 及体力配置 / 估算 16 项，共 81 项 Rust 回归通过。覆盖卡片与恢复的完整交接、储蓄 / 电池 / 库存不足、卡片编辑和消费后暂停、下一步不重放、同帧返回、资源预检、保存失败、旧帧、释放失败、错误返回，以及父子终态保存期间的暂停 / 停止 / 失败 / 超时。构建与原生证据范围见[开发验证记录](../migration/acceptance.md#2026-年-10-月-24-日-m5-模拟室卡片与电量恢复串联)；开发回放不作为实机验收。
