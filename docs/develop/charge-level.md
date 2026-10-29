# 体力副本指定等级

参考 notorious_hunt.py 的“选择难度”、notorious_hunt.yml 的难度入口 / 列表，以及 charge_plan 的固定等级目录：指定等级当前落在恶名狩猎的深度追猎。其他分类的旧操作没有执行等级切换，继续保持默认等级约束，不能把已保存等级当作实际选中。

深度追猎在核对同次目标、周常耗尽与深度模式之后、恢复或下一步之前选择指定等级。默认等级保持现状；指定等级须读取入口文字，已是目标时采用该帧明确读数，否则分别保存打开列表和选择的意图。只在已登记列表内匹配唯一完整等级，不用重复点击最高等级作兜底。选择后必须重新读到目标等级及实际 60 电量，恢复、暂停继续和消费前仍复核；错误、缺失、重复或未确认的等级不允许下一步。

等级识别使用独立资源包、固定来源和只读节点，按需要在首个导航输入前预检并冻结。原深度追猎资源不改定义，默认等级不增加该依赖。指定等级缺少真实留档，将使用标注用途的合成画面与现有副本回放验证；真实入口文案、菜单布局、切换和消费全部保留实机验收。用户选择暂不启动游戏，本轮遵守，正式体力入口及完整目标保持待办。

独立 schema 1 包锁定五条来源、四个数据文件及固定 OCR，不接受重新计算哈希后的区域、模型、来源或额外文件变更。只读命令不连接游戏，不提供本次选中或成功证明：

```powershell
cargo run --locked --target-dir target/m4 -p od-tools -- charge-level-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
target/m4/debug/zzz-engine.exe charge-level-bundle-check
target/m4/debug/zzz-engine.exe charge-level-inspect --image artifacts/charge-level-selected-fixture.png
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test charge_level -- --test-threads=1
```

等级核心、真实 SDK 回放及代理人标记定向验证共 17 项通过，包含指定等级后接默认等级各计一次、菜单意图不重复，以及默认等级清除可选识别；严格 Clippy、原生构建、来源和只读命令检查通过。完整范围见[验证记录](../migration/acceptance.md#2026-年-10-月-29-日-m5-指定等级与代理人标记)，汇总 artifacts/charge-level-verification.json 保持 goal_complete=false。
