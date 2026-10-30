# 专业挑战室单次挑战与父循环

普通专业挑战按本次完整快捷手册路径确认目标，独立读取剩余电量 1475,1000..1528,1055 与需要电量 1535,1000..1580,1055。常规单次需要 40 电量，不能复用区域巡防 60 电量或伪造模拟室选关证明。代理人方案和指定等级继续单独处理。

参考 expert_challenge.py 与体力玩法说明，专业挑战在下一步前关闭燃竭模式，复用恶名狩猎的 ON 区域 1658,672..1754,722 和确认按钮。新实现保存关闭及确认的独立意图，点击前在新帧复核。ON 时允许读取较大的实际费用用于观察；只有明确 OFF 且实际需要 40 电量时才允许恢复或下一步。不能用 ON 未命中推断 OFF；未知状态保留为待验证。确认须有本次关闭意图及明确关闭燃竭的语义，普通确认或开启语义不能授权。

缺口恢复、选队、出战与前移、战斗、结算和实际返回沿用现有有界流程，各自保留专业类型。单场完成采用自己的结果和执行 UUID；正常保存并释放输入后才允许计数，失败、撤退和未验证模式不计成功。结算后重新读专业详情的 40 电量和明确 OFF 状态。父循环先编译脚本、预检资源，再以唯一原生槽位交接；保存结果后返回大世界重读下一轮。

当前没有专业详情、OFF 和燃竭确认的真实留档；严格识别与执行分支使用明确标注的合成画面验证，实际文案、布局与操作保留验收，缺少明确状态时不得继续消费。遵守用户暂不启动游戏的选择，正式入口与完整目标仍待完成。

入口、模式切换、恢复和结算采用自己的资源包及 `Expert.` 只读节点，ON 文字须通过参考 RGB 过滤，OFF 须在同帧明确出现。关闭确认同时要求本次关闭意图、燃竭模式与关闭语义及固定确认按钮；分开的标题与正文可以组合识别，但开启或普通确认不能授权。连续三次已识别详情仍为未知模式或错误常规费用时返回待验证；恢复后、暂停继续和输入前重新取帧。燃竭打开时的较高费用只用于观察与关闭，不能作为 40 电量准备成功。

`ChargeWorker::new_with_combat` 预编译全部启用区域及专业计划的脚本，冻结相应资源并复验战斗设置；原 `new` 缺少战斗上下文时拒绝这两类计划。专业与区域可在同次父任务中按顺序运行，分别保存 40 与 60 电量的独立结果；公共单次专业接口和内部父循环均使用私有执行证明，不能用审计快照或固定图 inspect 伪造成功。页面返回仅允许非消费导航，仍须独立详情及实际返回模板。

开发与验证命令：

```powershell
cargo run --locked --target-dir target/m4 -p od-tools -- expert-challenge-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
target/m4/debug/zzz-engine.exe expert-challenge-bundle-check
target/m4/debug/zzz-engine.exe expert-challenge-inspect --image artifacts/expert-challenge-off-fixture.png
target/m4/debug/zzz-engine.exe charge-navigation-inspect --expert-challenge-bundle resources/generated/expert-challenge --image artifacts/expert-challenge-off-fixture.png
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test expert_challenge -- --test-threads=1
```

构建包为 schema 1，包含四个受管数据文件及十三条参考来源。只读命令保持 `accepted=false`、`input_sent=false`、`completion_verified=false`，导航检查另保持 `navigation_verified=false`。固定合成图读到 ON / 240 或 OFF / 40，只证明该固定图被识别；不等于真实专业详情布局、模式切换或完整副本已验收。协议 v47、导入预览 v19 与正式可用标记不变。

2026 年 10 月 29 日，专业专项 16 项、次数调度 19 项、持久化 21 项及相邻定向 5 项共 61 项唯一回归通过；严格 Clippy、原生构建、来源核对与只读 CLI 通过。首轮测试断言修复及最终复验范围详见[验收记录](../migration/acceptance.md#2026-年-10-月-29-日-m5-专业挑战室单次挑战与父循环)，汇总 artifacts/expert-challenge-verification.json 保持 `goal_complete=false`。
