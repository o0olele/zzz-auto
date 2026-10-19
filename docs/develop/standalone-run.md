# M3 独立应用运行

“独立应用运行”提供当前账号的单项执行入口。选择应用组作为参数来源，再选择组内的邮件、咖啡、活跃度奖励、丽都城募或刮刮卡；可以运行保存为禁用的应用。配置和启用项保持原样，执行采用同一资源预检与每日证明规则。后三项的参数和完成条件见[首批日常](basic-daily-runs.md)，真实领取后态仍待验收。

独立运行每次执行所选应用，保留并展示已有每日成功证明。成功且记录保存、输入释放后形成新的账号每日证明；失败或停止不撤销此前成功。独立运行不触发一条龙结束后动作，按账号保存的本机结果提醒继续生效。

协议 v31 增加 `preview_standalone`、`start_standalone_trial`，参数包含账号、来源组及一个应用 ID；启动另需配置修订、游戏日和账号 / 输入确认。引擎重新读取保存配置、检查组归属并计算计划，复用原有资源预检、运行槽、持久检查点和执行器。`RunContext.standalone` 缺省为 false，旧记录仍可读取，新记录冻结独立运行标记。每日策略在本次计划中为每次运行，长期设置不变。

页面切换后从快照恢复同一任务，暂停 / 继续 / 停止使用同一运行 ID。修改账号、来源组、应用或连接状态会撤销预览与确认，迟到响应失效；未保存结果及未释放输入继续拦截新任务。当前协议 v33，桌面在输入投递前登记 v7 输入恢复计划，旧 v1..v6 记录保留原义。结束后动作管理器独立拒绝带独立运行标记的任务，避免误登记。下节保留 v31 首次交付时的回归范围，当前集成见首批日常说明。

## 验证

2026 年 10 月 19 日新增 6 项核心、2 项真实 Maa SDK 模拟控制器、1 项桌面恢复及 1 项提醒回归。覆盖单项及组归属、禁用应用、每日已有成功仍执行、旧记录兼容、账号 / 修订 / 游戏日 / 输入确认、结果冻结、重复运行后失败保留此前成功、配置原文字节不变、v6 恢复登记及同 ID 暂停继续。结束后动作管理器拒绝误登记的独立任务。

连同既有账号运行、每日计划、顺序调度、邮件、历史、结束后动作、提醒与资源兼容共 110 项 Rust 回归通过。资源兼容逐一校验 schema 1 的协议 15..31 和 schema 2 的协议 16..31，拒绝未知协议和 schema 2 的协议 15。新增 6 项界面用例，连同运行页、账号、历史和提醒共 86 项通过；覆盖切页控制、来源或应用变化后确认撤销、往返变化后的迟到响应、错误来源 / 列表 / 结束后动作、保存失败、输入未释放、断线及浏览器预览。明暗主题和 960 像素宽截图已检查，位于主仓 `artifacts/m3-standalone-{light,dark}.png`。

前端生产、配套引擎、包含静态前端的原生 debug 桌面构建，以及 workspace 与相关测试的严格 Clippy、主仓格式检查通过。随附引擎与 `target/rerun-validation/debug/zzz-engine.exe` 的 SHA-256 相同。前端保留已有大于 500KB 的单块提示。

本轮使用隔离配置、模拟控制器和浏览器，没有操作真实游戏或修改真实用户配置。真实 WebView2、游戏邮件和咖啡完整领取仍待实机验收；咖啡与活跃度的素材缺口继续保留。此前记录的独立测试仓全量编译接口失配仍单独待修，本批不据此声称全量测试通过。

```powershell
$env:ZZZ_AUTO_TEST_ENGINE = (Resolve-Path target/rerun-validation/debug/zzz-engine.exe).Path
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/standalone-recovery --locked --test standalone_run --test group_runs --test group_selection --test daily_runs --test group_schedule --test mail_pipeline --test run_history --test after_done --test run_notifications -- --test-threads=1
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/standalone-recovery --locked --test input_recovery standalone_arms -- --test-threads=1
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/standalone-recovery --locked --test runtime_resources shortcut_protocol_preserves_existing_resource_protocols_and_rejects_unknown_versions -- --test-threads=1
npm --prefix zzz-od-test/test/zzz_auto/frontend test -- one-dragon-run.spec.ts accounts.spec.ts run-history.spec.ts run-notifications.spec.ts --reporter=line
npm --prefix apps/desktop run build
cargo build --locked --target-dir target/rerun-validation -p zzz-engine -p zzz-desktop --features custom-protocol
```
