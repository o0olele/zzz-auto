# M3 一条龙结束后动作

在当前桌面会话启动的应用组正常完成后，可选择无动作、关闭游戏、关机、休眠或睡眠。选择保存到账号配置的全局设置，运行启动时冻结；独立诊断、失败、停止、超时和已经完成的历史任务不触发。CLI 和独立引擎只保存设置，不自行执行桌面或系统动作。

## 收尾与取消

桌面宿主只为本会话成功接受的应用组登记一次动作，使用会话和运行 ID 区分。整组正常完成、分项全部成功或按日跳过、至少实际完成一项、结果保存且输入确认释放后，进入 60 秒倒计时。所有实际动作均提供相同的取消机会；托盘隐藏时显示主窗口，任意页面都可查看和取消，托盘也提供“取消结束后动作”。取消只撤销本次动作，保存的选择供下次运行使用。

倒计时由 Rust 单调时钟维护，WebView 不执行动作计时。断线、会话或任务变化、结果保存失败、资源工作冲突、桌面退出及监测间隔异常会撤销登记；重新连接和重启不恢复或补发。倒计时及执行中拒绝开始新输入任务、修改配置和切换资源。到期前再次检查同一任务和输入恢复登记，领取和输入均不重放。

系统动作仅在倒计时结束后提交，倒计时期间不创建 Windows 关机计划。关机使用系统目录中的 shutdown.exe /s /t 0，不传 /f；Windows 的正数 /t 会隐式强制关闭应用，因此取消等待保留在宿主内。睡眠和休眠使用 SetSuspendState，并按官方要求临时启用关机权限。关闭游戏只向本次运行绑定的窗口发送关闭请求，核对 PID、进程创建时间及窗口归属，不按进程名称批量结束；拒绝访问或未退出时报错，不强制结束。

依据：[Windows shutdown](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/shutdown/)、[SetSuspendState](https://learn.microsoft.com/en-us/windows/win32/api/powrprof/nf-powrprof-setsuspendstate)。

## 配置与验证

旧配置缺少字段时默认无动作。旧版 after_done 只转换已知中文枚举，未知值保留原文并提示采用无动作；导入预览、备份和离线恢复展示实际设置。运行计划展示冻结前的保存值，启动仍需本次账号与前台输入确认。

自动验证使用模拟时钟、隔离配置、命名管道及注入的动作执行器，禁止在测试中真正关机、休眠、睡眠或关闭游戏。真实 Windows 电源动作、管理员游戏关闭及 WebView2 倒计时另行验收。

当前协议为 v26，导入预览为 v6。原有运行资源 schema 1 / 协议 15..25 和 schema 2 / 协议 16..25 继续兼容。设置缺失时只读补无动作，保存时写入 `general.after_done`；更改普通截图或邮件设置会保留它。旧客户端不认识新字段，桌面及引擎须配套更新。

新增 14 项 Rust 回归覆盖配置默认值及严格校验、修订、复制、备份和离线恢复、旧设置转换及原文归档、真实命名管道、完整成功及输入证明、暂停及停止、失败和超时、旧请求 ID、会话及窗口切换、监测中断、取消、单次提交与执行器失败。连同相邻账号配置、每日计划、应用组、备份恢复、顺序调度及旧资源兼容共 73 项 Rust 回归通过。新增 6 项界面回归，连同运行页、账号、设置、备份、离线恢复及桌面共 81 项通过，验证保存及冲突、通用设置保留、明确启动确认、跨页取消及新任务拦截、原生状态推进及旧取消响应。明暗主题、960 像素宽页面和倒计时截图已检查。

生产前端、配套引擎及包含静态前端的原生 debug 桌面构建通过，主 workspace 与新增测试严格 Clippy、格式和差异检查通过。资源兼容专项修正仍固定为 v25 的检查条件后通过，覆盖基础包 v15..26、咖啡包 v16..26，并继续拒绝未知协议和咖啡包 v15。保留已有前端单块大于 500KB 的提示；本轮未重打包便携 ZIP。

```powershell
node scripts/prepare-engine.mjs
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test after_done --test account_config --test daily_runs --test group_runs --test config_backups --test config_recovery --test group_schedule -- --test-threads=1
npm --prefix zzz-od-test/test/zzz_auto/frontend test -- one-dragon-run.spec.ts accounts.spec.ts config-backups.spec.ts offline-config-recovery.spec.ts desktop.spec.ts --reporter=line
npm --prefix apps/desktop run build
cargo build --locked -p zzz-desktop --features custom-protocol
```

## 试用

先在当前账号准备只启用邮件、采用“每次运行”的应用组，不需要消耗咖啡次数。进入“一条龙运行”，选择“关闭游戏”并保存，重新预览核对结束后动作和启动确认，再开始邮件。

邮件成功后应显示 60 秒倒计时；先测试在其他页面或托盘取消，确认游戏保持打开且保存值仍是关闭游戏。需要验证关闭行为时再次明确启动，等倒计时到期后核对游戏退出及结果。管理员游戏可能拒绝普通桌面的关闭消息，宿主会展示失败并保留游戏，不自动提权或强制结束。

关机、休眠和睡眠会实际改变 Windows 电源状态，自动验证没有执行这些动作。本批未操作用户游戏、账号目录或电源状态，M3 仍保持进行中。
