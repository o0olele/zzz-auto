# 深度追猎单次挑战与体力父循环

体力计划中的恶名狩猎使用深度追猎，固定预估 60 电量。参考 notorious_hunt.py、notorious_hunt_move.py、对应 screen_info 和 charge_plan.md：先由同次快捷手册传送到列表或目标详情，列表按目录全名及明确别名选择；详情重新核对标题。若传送进入其他已知副本详情，须保存自己的返回列表意图、实际回到列表并重新选择完整目标；不采用外部切换后直接出现的目标详情。仍有周常奖励次数时，当前计划必须跳过，即使未启用电量不足跳过，也不能改打周常或消耗体力。

进入消费前要求周常已用尽的明确证据、深度追猎 ON 及实际需要 60 电量。深度信息图标按参考原图与掩码转换并校验；有奖励次数标签时须读取实际次数，缺失或冲突均不能当作零。无报酬模式与开启确认分别保存意图，确认须具备开启深度追猎语义；未知模式、费用或奖励状态返回待验证。恢复后重新核对同一目标及全部消费前置。

出战共用选队和低等级确认，深度追猎随后使用已有距离、交互、鸣徽和搜索移动流程，不能复用模拟室固定前移作为完成证明。移动与战斗按冻结目标、唯一会话槽位及剩余预算交接，界面和移动采用前台 CPU，战斗按冻结配置切换。结果仅由正常战斗、自己的完成及奖励意图、实际返回同一详情组成；成功、实际不足和周常阻挡保留独立类型及执行 UUID。周常阻挡不增加次数，不作为整组成功。

目前缺少恶名狩猎列表、详情、深度模式和开启确认的真实留档；将以明确标注的合成图和已有战斗、恢复、移动素材进行离线验证。真实文案、布局、模式切换、实际鸣徽及连续运行全部保留实机验收。用户选择暂不启动游戏，本轮遵守；正式体力入口、代理人方案及完整目标继续待办。

内部入口为 `DeepHuntRun` 与 `ChargeWorker::new_with_combat`。所有启用区域、专业和追猎脚本在首次导航输入前编译并冻结；追猎额外校验和加载固定距离模型。共用出战类型默认仍要求固定前移，只有已核实的追猎类型交给独立移动阶段。移动活动预算累计 120 秒，暂停保留交互、鸣徽意图及已用预算；移动和普通战斗分别重新核对画面及输入方式。

`HuntSucceeded`、`HuntInsufficientCharge` 和 `HuntWeeklyRewardsAvailable` 采用原日志先于配置的提交路径。周常阻挡保留明确的 1..3 次实际读数，按本次运行跳过，不增加成功次数；剩余计划仍按原顺序选择，全部处理后保留阻挡终态。正常结果保存失败后只补交原执行 UUID，停止状态下不再操作游戏。返回导航要求已识别列表或完整详情和实际返回模板。

资源为独立 schema 1，六个受管数据文件和二十条参考来源，包含 OCR、只读 Pipeline 与从原图和二值掩码确定性转换的深度信息模板。构建及诊断命令：

```powershell
cargo run --locked --target-dir target/m4 -p od-tools -- deep-hunt-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
target/m4/debug/zzz-engine.exe deep-hunt-bundle-check
target/m4/debug/zzz-engine.exe deep-hunt-inspect --mission-type 初生死路屠夫 --image artifacts/deep-hunt-on-fixture.png
target/m4/debug/zzz-engine.exe charge-navigation-inspect --deep-hunt-bundle resources/generated/deep-hunt --image artifacts/deep-hunt-on-fixture.png
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test deep_hunt -- --test-threads=1
```

固定图只读检查保持 `accepted=false`、`input_sent=false`、`completion_verified=false`，导航另保持 `navigation_verified=false`。合成画面中的明确模式、耗尽证据与 60 电量只证明该固定图的读取结果，真实玩法和操作继续保留验收。协议 v47、导入预览 v19 和正式可用标记保持不变。

[指定等级](charge-level.md)使用独立 schema 1 资源，只在计划要求时预检并冻结。打开列表与选择等级分别保存意图，菜单必须仍属于同次目标、周常耗尽及深度 ON；选择后重新读取目标等级和实际费用，最高等级不作盲目重复点击。暂停和恢复后重新读取，默认等级保持原资源路径。

奖励次数沿用参考的单行识别，固定 1516,1018..1538,1043 区域及原置信阈值，避免把 22×25 的数字拆成多个检测片段；没有可靠读数仍为未知，不能填零。ON 与无报酬标签冲突时禁止消费。周常阻挡终态同样完成返回大世界，保留独立结果，不记整组成功。

2026 年 10 月 29 日，追猎专项按唯一用例 19 项，连同次数调度、持久化及相邻定向共 84 项通过；严格 Clippy、原生构建、来源核对和只读 CLI 通过。初始失败、修复及分段复验范围详见[记录](../migration/acceptance.md#2026-年-10-月-29-日-m5-深度追猎单次挑战与父循环)，汇总 artifacts/deep-hunt-verification.json 保持 `goal_complete=false`。
