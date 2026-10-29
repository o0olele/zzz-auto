# 代理人方案培养的快捷手册导航

参考 CompendiumChooseMissionType、快捷手册区域和 cv2_utils。内部 `AgentTransportRun` 接收原始代理人计划，处理实战模拟室、区域巡防、专业挑战室及恶名狩猎。模拟室沿用目录中的“代理人方案培养”关卡占位值，其余分类保留空关卡；不改原计划 ID、次数、编队或脚本。普通传送模块和原资源定义保持不变。

先核对训练页签、完整分类和蓝色选中态，再按参考顺序读取首个培养标记。标记中心向上 80 像素，配对下方最近的“前往”；对应的唯一完整副本名称、标记所属行及按钮必须一致。缺少目标行按钮时，不能借用下一行按钮。投递前保存具体目标和像素指纹，重新读取不符时撤销尚未发送的意图；前往已投递后不替换本次目标。像素指纹用于本次导航核对，不证明角色身份。

打开手册、训练、分类、前往和确认分别保存意图，只有投递后的实际画面才能推进。已有确认或副本页面不能作为起点；到达必须匹配分类，追猎详情还须匹配本次具体目标。模拟室只交接详情或类型列表。暂停继续保留已投递动作，完成后暂停会撤销旧到达回执并重新读图。保存失败不发送下一动作；停止释放 Alt 和鼠标。输入结果不明时不重复前往或确认。

找不到标记时，在参考目标区域中心左移 100 像素向上滚动 300；有标记但对应按钮缺失时，在前往区域中心向上滚动 200。恶名狩猎使用收窄后的 1254,552..1312,894 区域。原生拖动补齐终点后才记一次，取消的半次拖动先释放输入，继续时从新画面重读。十次完整滚动后仍须再次确认已知分类和列表，才能得到独立 `Unavailable` 终态；未知画面、错误到达和读取失败不能转成“未找到目标”。这个结果不代表培养完成。

独立 schema 1 包锁定十七条来源、固定 OCR、手册模板及只读 Pipeline。`AgentTransportReceipt` 仅在终态保存、输入释放及共享控制句柄无中断后交回，仍为 `selection_verified=false`。此回执自身不提供消费或次数许可；[代理人挑战](agent-challenge.md)已将区域、专业和追猎从实际到达直接接入各自的费用、准备、战斗和结算，再由父循环登记原计划并处理未找到、实际不足及达成。模拟室已接通[详情目标解析与投递](agent-mission.md)，生产选中证明、完整消费及正式执行入口仍待完成。

```powershell
cargo run --locked --target-dir target/m4 -p od-tools -- agent-transport-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
target/m4/debug/zzz-engine.exe agent-transport-bundle-check
target/m4/debug/zzz-engine.exe agent-transport-inspect --category 区域巡防 --image artifacts/agent-transport-book-1.png
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test agent_transport --test agent_transport_execution -- --test-threads=1
```

固定图检查不发送输入，accepted、input_sent、navigation_verified、selection_verified、completion_verified 均为 false。没有代理人方案真实留档，测试使用明确标注的合成标记和原画面锚点；全部真实语义、布局、操作和连续运行保留实机验收。按用户选择，本轮未启动或连接游戏，协议 v47、导入预览 v19 及正式可用标记保持不变。

新增 15 项，连同普通传送、标记及等级核心共 38 项定向检查通过，包含四类各五处暂停、错误到达、标记变化、缺少按钮、原始计划保持、半次滚动暂停及保存失败。连续读取回放暴露 SDK 终态先于 Tasker 空闲标记的时序差；共用识别适配在仍持有本会话识别锁时，有限等待本任务完成收尾后再放行下一次读取。严格 Clippy、原生构建、来源及只读命令检查通过，详见[验证记录](../migration/acceptance.md#2026-年-10-月-29-日-m5-代理人培养导航)，汇总 artifacts/agent-transport-verification.json 保持 goal_complete=false。
