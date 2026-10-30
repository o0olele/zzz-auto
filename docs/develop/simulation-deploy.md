# 模拟室出战与子战斗交接

内部流程接续同一父任务的选队准备，保存出战和低等级确认的意图，再等待实际战斗画面。按旧模拟室规则，确认攻击按钮后向前移动一秒，释放 W 并重新核对战斗画面，才交接冻结的自动战斗脚本。子战斗继续使用已有进入证明、连续两张新帧结算和输入释放规则；其结果可交给已有结算领奖执行器，不直接登记次数或应用成功。

依据为参考仓 `CombatSimulation`、`Deploy`、`common_deploy.yml`、模拟室和战斗画面资料。出战页要求固定“出战”与“预备编队”；驱动盘容量上限优先阻止输入。低等级只接受固定位置的完整“确定并出战”及已知 OCR 字形“确定並出战”。“队员数量少”的资料只提供通用“确认”按钮，缺少独立标题，不能据此区分其他弹窗，暂时拦截并记录为缺少画面依据。

输入前保存意图并重新截图，同帧核对弹窗与按钮，保持 1500ms 帧龄限制。已投递或不确定投递的出战、确认和前移不重放；暂停继续重新识别，不重置父预算。W 持有期间中断先释放，不补走剩余时长。保存失败、停止、超时及释放失败阻止交接；终态保存期间的控制请求继续有效。

出战会消费电量，必须先有冻结关卡的选中证明和同一计划的电量、选队结果。现有 `MissionReceipt.selection_verified=false` 不能通过预检；数量、电量及选队证明不能代替关卡证明。开发回放只在隔离控制器中构造明确标注的已验证前置，真实生产者仍待关卡选中态素材接入。不开放桌面或 CLI 带输入入口，完整体力应用继续关闭。

出战及前移使用前台控制器，子战斗采用冻结战斗输入配置。父任务负责同一运行槽内的控制器交接；调用子战斗前必须释放旧控制器并提供匹配输入方式的会话，不能另建并行控制入口。编译脚本、修订和资源清单在首个出战输入前冻结，继续不重新采用后续导入的脚本。

`DeployRequest::from_plan` 复验冻结类型、关卡、默认等级、实际卡片电量和编队参数，并要求关卡回执明确 `selection_verified=true`。准备和选队结果仍只提供各自的证明，不能将其中的标记改成关卡证明。`PreparedDeployment::load` 在输入前校验脚本修订、完整资源与脚本，`DeploymentRun::after_teams` 还要求同一冻结选队结果、正常返回和输入释放。

`DeploymentRun::execute_with_budget` 使用已有父控制句柄，完成后只有 `DeploymentEnd::Entered`、输入释放且无待处理请求时，才允许一次性 `into_combat` 转换。生成的 `CombatRun` 保留运行 ID、编译脚本、配置和父控制句柄。转换本身不发送输入；父任务按 `combat_mode()` 通过既有会话交接接口选择控制器，再调用 `execute_segment_with_budget`，并从同一剩余父预算扣除出战阶段的活动时间。战斗完成且释放输入后使用原 `SettlementRun::after_combat`，不将出战按钮消失或单张结算图当作战斗完成。

独立 schema 1 `simulation-deploy` 包固定六份来源、OCR 模型及全部只读节点；原模拟室、结算、日常和战斗包不修改定义。协议 v45、导入预览 v18 及既有输入恢复版本保持不变；界面输入的 W 已包含在共用混合恢复清单中。完整父任务接入前仍不开放体力应用。

2026 年 10 月 26 日继续接通[详情推进与选队交接](simulation-entry.md)：`EntryRun` 在首次下一步前接收已编译的战斗脚本，确认实际出战页和选队后，通过 `into_deployment` 一次性传入本执行器。关卡未验证的回执仍被拒绝，完整体力父任务和实际控制器切换继续待接入；当前协议沿用锄大地阶段的 v46、导入预览 v19。

```powershell
target/m4/debug/od-tools.exe simulation-deploy-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
target/m4/debug/zzz-engine.exe --root E:/Code/zzz-auto --sdk E:/Code/zzz-auto/artifacts/deploy-no-sdk --data E:/Code/zzz-auto/artifacts/deploy-unused-data simulation-deploy-bundle-check
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test simulation_deploy --test simulation_deploy_execution -- --test-threads=1
```

资源校验不加载 SDK 或创建用户配置，报告保留 `accepted=false`、`input_sent=false` 和 `selection_verified=false`；它只说明资源兼容，不能提供消费授权。组合图和已有战斗、领奖留档只用于开发回放，不作为真实出战、前移时长、控制器切换或实机验收。

2026 年 10 月 24 日新增核心及资源 8 项、真实 Maa SDK 隔离执行 10 项通过，包含完整战斗及领奖串联、首个输入前冻结脚本、三处提交后的暂停恢复、普通确认拒绝、容量上限、旧帧、资源和脚本修订变化、保存 / 释放失败、父预算及终态请求。首轮测试夹具误用了超出战斗配置范围的帧龄；完整链路的结算图随后也补齐了模拟室所需的“再来一次”，保留正式识别规则，修正夹具后回归通过。

相邻子战斗、关卡选择、卡片恢复串联、结算及选队核心回归共同通过，按唯一用例共 66 项；严格 Clippy 和配套原生构建通过。证据与待验收范围见[开发验证记录](../migration/acceptance.md#2026-年-10-月-24-日-m5-模拟室出战与子战斗交接)。
