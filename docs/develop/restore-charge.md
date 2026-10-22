# 副本内电量恢复

体力计划配置之后，先实现可被各副本复用的电量恢复子流程。父流程须先确认本次副本需求及当前电量，并打开实际恢复弹窗；子流程不点击“下一步”或“再来一次”，不启动战斗，也不修改计划次数和每日记录。

玩法及画面依据为参考仓的 `docs/game/gameplay/合成电池.md`、`docs/game/screens/恢复电量.md`、`assets/game_data/screen_info/restore_charge.yml` 和 `src/zzz_od/operation/restore_charge.py`。储蓄等额兑换，以太电池每个恢复 60 电量。仅使用冻结配置允许的来源，双来源优先储蓄，不合并两个不足来源。保留旧版兑换数量必须严格小于持有量的规则；恰好用尽时保留不足结果。菲林不进入来源枚举。

主弹窗读取两种库存和选中边框；快捷使用重新读取实际来源、持有量及默认数量，只允许本次缺口需要的最小数量。来源改变、数字不可读和数量不一致都不能提交。来源选择、打开快捷使用与实际消耗分开；提交意图保存成功后才能输入。原生层在输入前重新截取并核对同一画面，拒绝过期帧。

提交后只等待同数量“电量”奖励及返回后态，不重复消费；奖励确认另存意图。战后重试可能不出现奖励弹窗，因此实际成功始终要求父流程在同一新帧正向识别其自身无遮挡的返回画面，并提供当前及所需电量。需求必须与冻结值一致，当前电量须达到原值加本次恢复量；弹窗消失、未知画面或单独“确认”均不提供成功证明。返回画面的识别回调由各副本实现，本轮不猜测缺少留档的副本入口。

子流程复用父控制句柄、原生会话及剩余活动预算。暂停和停止先清空输入；暂停期间不计时，继续保留消费及奖励确认意图。保存失败、父预算耗尽和释放失败阻止后继。独立 schema 1 资源只包含恢复所需 OCR 节点及固定模型，冻结参考源码和区域来源，不改变现有日常、随便观资源及 v45 协议。

`od-core::restore_charge` 保存业务检查点，`maa_adapter::restore_charge` 负责同帧识别与输入复验，`zzz_engine::restore_charge::RestoreRun` 负责父预算、持久化和收尾。父返回回调仅获得只读会话及本次截图，不应重新截图或从背景数字推断返回成功；不识别或不属于当前副本时返回 `None`。只有 `RestoreOutcome::Restored` 表示恢复成功，子流程安全返回但来源关闭 / 库存不足分别保留 `Disabled` / `Insufficient`，不能据 `RestoreEnd::Completed` 增加刷本次数。

```powershell
cargo run --locked -p od-tools -- restore-charge-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
target/debug/zzz-engine.exe restore-charge-bundle-check --bundle resources/generated/restore-charge
target/debug/zzz-engine.exe restore-charge-inspect --bundle resources/generated/restore-charge --image zzz-od-test/screens/恢复电量/快捷使用.webp
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test restore_charge --test restore_charge_execution -- --test-threads=1
```

`restore-charge-bundle-check` 在 SDK 加载及用户资源选择之前执行；检查命令保持 `accepted=false`、`input_sent=false`。开发验证使用三张未经修改的恢复原图，以及明确组合的以太来源、库存、奖励数字和父返回画面；这些组合帧不能计为真实游戏通过。真实恢复、战后无奖励路径及各副本执行验收继续保留。

2026 年 10 月 22 日，新增核心 11 项、真实 Maa SDK 隔离执行 11 项，共 22 项通过；相邻体力计划 16 项、应用内战斗 14 项、随便观配置与编排 10 项、共用选队 9 项全部通过，合计 71 项 Rust 回归。覆盖储蓄优先、电池回退与取整、恰好用尽拒绝、错误来源 / 数量 / 奖励 / 需求、消费及奖励确认后的暂停、不出现奖励弹窗、保存失败、父预算、旧帧、释放失败、停止优先、来源变化和重新填写清单哈希后的节点篡改。原始恢复三帧均使用固定 CPU OCR，回放不连接游戏。

Workspace 与新增测试目标严格 Clippy、主仓格式、新测试格式及两仓空白检查通过。引擎、工具和原生桌面构建成功，桌面随附引擎与 `target/debug/zzz-engine.exe` SHA-256 均为 `6408E04306F29E225974021DDB9993AF4CE719011FA7EDA1DD7A1E531385C0E0`。原生命令生成独立资源后，使用不存在的 SDK 和全新数据路径执行包校验成功且未创建数据目录；只读截图命令正确识别原始快捷使用中的储蓄电量 2400 和兑换数量 17。调试构建耗时不作为性能验收，未执行真实游戏恢复或修改用户配置。
