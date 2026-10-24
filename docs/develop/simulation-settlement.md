# 实战模拟室结算与领奖收尾

本内部子流程接续同一父任务的 `CombatSegmentEnd::Result(Completed)`，依次核对普通结算的“完成 + 再来一次”、点击完成、识别模拟室专有的“获得 + 前往 + 确认”弹窗、确认奖励，以及重新读到带实际电量的副本详情。撤退、倒带、退出、普通确认及恢复弹窗均不能提供完成证明。单独打开一张结算或奖励图片只能得到只读观察，不能证明本次战斗成功。

依据为参考仓的实战模拟室和战斗画面文档、两份 `screen_info`、`CombatSimulation` 以及 `ChooseNextOrFinishAfterBattle`。既有真实奖励留档核对弹窗；缺少真实结算与返回详情的分支使用明确组合帧验证。旧操作在点击完成后即结束，没有处理领奖弹窗；新版按已有画面资料继续确认并核对返回，不把按钮投递当作领取成功。

`SettlementFlow` 分别保存完成与领奖确认的意图、投递回执、实际奖励观察及返回电量。已投递或可能投递的输入不重放，只有适配器明确未投递才撤销意图。暂停继续撤销返回证明，重新读实际后态；保存、截图、输入及释放均消耗父流程剩余活动预算，暂停等待不计时。父子共用控制器和控制句柄，保存失败、停止、超时或输入释放失败均阻止正常交回。

独立 schema 1 资源 `simulation-settlement` 锁定来源、OCR 与全部节点，不改变已有模拟室卡片或关卡包。提供生成、校验和只读识别命令；协议 v45、导入预览 v18 保持原义。本子流程不点击“再来一次”、不消费电量、不写计划次数或应用完成。完整父流程仍须关联冻结关卡、真实战斗进入与连续新帧结束证明、结算收尾和输入释放后，才能登记一次完成。

等级切换仍缺少可核对的菜单区域和交互规则：旧模拟室代码只识别“挑战等级”标题，没有设置等级。不能从这个标题推断当前等级或构造切换坐标；非默认等级后续接入时须补充画面依据。关卡选中证明、卡片与恢复电量串联、选队、完整副本执行及全部实机验收继续保留。

`od_core::simulation_settlement::SettlementFlow` 维护结算检查点，`maa_adapter::simulation_settlement` 负责同帧读取及输入，`zzz_engine::simulation_settlement::SettlementRun::after_combat` 检查战斗返回值和释放状态。收尾完成返回 `SettlementEnd::Collected`，回执保留实际电量；父流程必须检查返回值及输入释放，不能只检查快照的完成字段。暂停继续撤销返回电量证明，保存终态期间收到暂停、停止或耗尽预算同样阻止正常交回。

```powershell
cargo run --locked -p od-tools -- simulation-settlement-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
target/debug/zzz-engine.exe simulation-settlement-bundle-check
target/debug/zzz-engine.exe simulation-settlement-inspect --image zzz-od-test/screens/实战模拟室/战斗结果-获得奖励.webp
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test simulation_settlement --test simulation_settlement_execution -- --test-threads=1
```

资源校验在 SDK 和用户配置加载前执行，重新填写哈希不能替换节点、来源或模型。只读报告保持 `accepted=false`、`input_sent=false`、`completion_verified=false`；单张图片识别为 `reward` 也不提供本次领取或战斗完成证明。
