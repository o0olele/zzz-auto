# M3 损坏配置的离线恢复

账号配置损坏导致引擎无法读取，或文件丢失需要找回配置时，可以通过随引擎提供的原生命令 `config-recovery` 从本数据目录的自动备份恢复。它在资源选择、Maa SDK 和引擎初始化前执行，不需要连接游戏、加载模型或启动桌面。正常配置继续使用[桌面配置备份恢复](config-backups.md)。

## 使用

先退出使用该数据目录的桌面和引擎。命令必须显式指定已存在的绝对数据目录；不会在拼错的目录中创建新数据。开发模式和安装版默认使用 `%APPDATA%\io.onedragon.zzz-auto`，便携版使用程序目录下的 `data`，源码 CLI 的 `data` 与桌面开发目录不同。

在工程目录执行以下命令，将示例数据目录替换为要恢复的实际目录。第一步只构建引擎，后续命令不要求 Rust 环境；正式包的引擎名为 `zzz-auto-engine.exe`。

```powershell
cargo build --locked -p zzz-engine
$recoveryData = Join-Path $env:APPDATA 'io.onedragon.zzz-auto'
.\target\debug\zzz-engine.exe --data $recoveryData config-recovery list
.\target\debug\zzz-engine.exe --data $recoveryData config-recovery preview --id 'accounts-r4-<备份UUID>.yml'
.\target\debug\zzz-engine.exe --data $recoveryData config-recovery restore --id 'accounts-r4-<备份UUID>.yml' --preview-id '<上一步的64位preview_id>' --confirm
```

`list` 返回当前文件状态、字节数、SHA-256，以及按修订排序的最多 50 份备份和省略 / 不可用数量。当前状态分为 `missing`、`damaged`、`healthy`、`unsupported`；列表不解析备份原文，具体备份仍须预览校验。可以显式预览已知但在列表中被省略的旧备份。

`preview` 严格校验所选备份的格式、修订、账号和设置，显示将恢复的账号、当前账号、应用组、通用设置、保留编号、新修订和指纹。当前配置内容只返回状态、长度和哈希，不返回损坏 YAML 或其中的凭据。内容与导入来源以所选备份为准，损坏文件无法完整核对；执行前须核对恢复账号与实际游戏登录的对应关系。

`restore` 必须提供指纹及 `--confirm`。正常可读配置和可辨认的不支持 schema 被拒绝，使用正常入口或配套版本处理。目录仍被引擎、导入工具或其他恢复进程占用时也拒绝操作；关闭窗口隐藏到托盘不等于退出。无需移动或删除锁文件。

## 保存与失败处理

离线工具整个生命周期持有现有数据目录所有权锁，预览和恢复另取配置写锁。指纹绑定规范化数据目录、当前原始文件状态、所选备份、扫描的备份内容及最终恢复计划；原文、备份目录、可恢复编号或目标修订变化后须重新预览。列表及预览只读取业务数据，可能创建占用和配置锁文件。

恢复先写临时文件并同步落盘，再将损坏文件逐字节保留到 `config/recovery/accounts-damaged-<UUID>.yml`，同时在 `config/backups/` 保存一份带新修订的有效恢复结果，最后完整替换 `config/accounts.yml`。缺失文件没有损坏原文归档。返回 `preserved_file` 和 `recovered_backup`，便于查验。损坏原文不混入正常备份列表，也不输出到命令行。

文件写入或替换失败时，当前文件保持原值，临时文件清理；已完整保存的原文归档及有效候选备份保留。失败可能增加备份，因此应重新预览后再提交，不自动重试。恢复后重启桌面，账号与设置经既有接口重新读取，快捷键由桌面重新注册；恢复不会登录游戏或启动自动化。

修订按现存备份最高修订加二；若损坏文件仍能辨认出更高修订，则至少为该修订加一。加二为正常保存时比当前配置落后一版的备份预留空间。修订溢出拒绝恢复；无法从完全损坏且没有对应备份的内容推断原修订，预览明确展示拟用值。

恢复收集所选及其他可校验备份、损坏文件仍可辨认的账号编号与保留编号，以及 `runs/accounts/NNNN` 的目录编号，写入保留集合。新账号不能复用这些编号，历史和每日完成状态继续归属原账号。历史目录只检查编号与目录类型，不读取或改写运行记录；无法从已删除或完全不可读的数据推断未知编号。其他损坏备份计数并跳过，所选备份损坏则直接失败。

所有业务文件限定在当前数据目录，禁止符号链接、目录联接和任意备份路径。单文件上限 4 MiB，备份目录与账号历史各最多扫描 10000 项，备份读取总量上限 64 MiB；越界、无效历史目录布局或保存失败均明确报错。导入原文、历史记录、每日成功状态、模型缓存及运行资源选择保持原值。没有有效备份时不会重置为默认配置或尝试猜测修补。

## 验证与范围

新增 13 项 Rust 回归通过，覆盖有界列表、原文隔离、损坏 / 缺失恢复、修订及编号保留、文件及备份变化、占用锁、写入失败、目录联接、大小限制、未知版本、坏备份及溢出。包含真实命令行进程的显式目录 / 确认检查、跨进程占用，以及损坏配置恢复后启动引擎并经现有配置协议读取的回归。加上既有备份、账号、应用组管理、每日计划与运行记录，共 59 项通过。

```powershell
cargo build --locked -p zzz-engine
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test config_recovery --test config_backups --test account_config --test group_management --test daily_runs --test group_runs -- --test-threads=1
```

主 workspace 和新增测试严格 Clippy、Rust 格式与差异检查通过。本轮没有更改桌面表单、IPC 或资源格式，协议保持 v20，未重打包便携 ZIP。测试仅使用生成的配置和隔离临时目录，没有修改真实用户配置或操作游戏。真实用户目录、正式发行包环境和恢复后的游戏参数生效仍待实机验收；桌面启动失败时的一键恢复入口尚未实现。
