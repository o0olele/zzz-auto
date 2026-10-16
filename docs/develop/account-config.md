# M3 账号与应用组基础

2026 年 10 月 16 日开始 M3。首批交付账号与应用组的独立数据、旧 YAML 导入预览、修订冲突检查及桌面编辑入口。咖啡等新游戏流程、通用设置表单、按账号运行记录及资源下载继续在 M3 后续实现。

## 数据与导入约定

引擎使用宿主指定的数据目录，账号及应用组保存到 `config/accounts.yml`。每次修改携带读取时的修订号，在文件锁内重新读盘并校验；冲突返回最新配置。保存前备份旧文件，临时文件同步落盘后完整替换。损坏或未知版本不能回退成默认值。运行中、暂停中、停止中及原生请求未结束时拒绝修改，账号切换不会登录游戏。引擎与离线导入工具还共用数据目录占用锁，导入前须退出使用该目录的引擎；同一目录只允许一个引擎持有。

提权切换保留“先启动候选进程、取消授权仍保留旧引擎”的行为。候选进程完成身份配对后等待首个请求；桌面确认旧引擎退出后，才发出请求并初始化候选引擎、接管数据目录。初始化失败返回结构化错误，不绕过目录占用锁。

导入入口为 `od-tools import-config`，显式指定旧程序目录和新数据目录。默认只读预览，列出账号、应用组、归档文件及待迁移项，不输出 YAML 值或账号密码。应用导入必须提供预览指纹；源文件变化后需重新预览。新数据目录不能与旧目录重叠。

读取 `config/one_dragon.yml` 的账号列表，以及 `config/NN/<group>/_group.yml` 的有序应用列表；默认组不存在时兼容 `one_dragon_app.yml` 的 `app_order` / `app_run_list`。保留未注册应用的顺序和启用值，新登记的默认应用补在末尾并禁用。实例编号、名称、当前实例及参与一条龙的标记保持原含义。

源目录内非 sample 的 YAML 按原始字节归档到新目录的 `imports/<指纹>/config/`，包括未知字段、各应用设置、账号凭据和旧运行记录。这些内容只在本机保存，不通过 IPC、日志或桌面快照返回；归档不是新引擎已经支持这些字段的证明。当前只转换账号与应用组，其他文件在预览中明确列为“仅归档，待迁移”。旧运行记录不转成新版成功记录。

首批导入只允许空白配置；同一份导入重复应用不覆盖后续编辑，其他导入拒绝覆盖已有配置。归档完整写入后才启用账号配置，失败可重新执行；修改前的配置备份位于 `config/backups/`。符号链接、目录联接、非 UTF-8 / 损坏 YAML、重复编号与重复应用等明确报错，不跳过后声称完整导入。

## 使用

桌面新增“账号与应用组”页面，支持新增 / 编辑账号、设为当前账号、参与一条龙标记，以及应用组内启停和上下移动。界面显示配置修订；冲突后读取最新内容并提示重新编辑，不自动覆盖。浏览器预览没有本地配置，只有连接桌面引擎后才开放编辑。

源码环境可先在 PowerShell 预览，保存返回的 `preview_id`，退出使用目标数据目录的桌面 / 引擎后应用：

```powershell
cargo run --locked -p od-tools -- import-config --source E:\Code\ZenlessZoneZero-OneDragon --data E:\Code\zzz-auto\data
cargo run --locked -p od-tools -- import-config --source E:\Code\ZenlessZoneZero-OneDragon --data E:\Code\zzz-auto\data --apply --preview-id '<上次预览的64位指纹>'
```

将示例中的数据目录替换为实际目标目录。便携版使用程序目录下的 `data`；安装版和桌面开发模式使用 Tauri 的用户数据目录，当前 Windows 默认位置为 `%APPDATA%\io.onedragon.zzz-auto`。源码 CLI 的默认数据目录与桌面开发模式不同，不能因为在工程下运行工具就假定写入了桌面目录。`od-tools` 是源码开发工具，本轮未新增便携导入程序或重打包测试 ZIP。

首次导入前不要保存默认账号：保存会建立正式配置，首批导入将拒绝覆盖它。已归档的原始 YAML 不自动参与游戏运行，游戏登录、通用配置字段转换及日常调度属于后续工作。

## 验证

独立测试仓覆盖只读预览、原文件不变、原始字节归档、旧格式与未知应用、默认禁用、重复导入、源变更、目录重叠、损坏文件、并发修订与备份、运行中修改拒绝及桌面冲突提示。所有验证使用临时目录和模拟引擎，不操作游戏。

```powershell
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test account_config --test mail_pipeline --test regression --test elevation -- --test-threads=1
npm --prefix zzz-od-test/test/zzz_auto/frontend test
```
