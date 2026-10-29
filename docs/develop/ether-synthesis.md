# 单次以太电池合成

`SynthesisChallengeRun` 已在同一父任务内串联快捷手册资源栏、电池详情、合成入口和单次兑换。`SynthesisEntryRun` 只负责导航，`SynthesisRun` 负责合成页到领取返回；两者共用运行 ID、控制句柄、调用方持有的原生会话和父任务总活动预算，不创建并行控制器。运行模式须与实际会话的游戏窗口属性一致，回放不能冒充游戏目标。当前要求起点为无遮挡的快捷手册；大世界到快捷手册、完整工作线程的窗口 / 账号冻结及桌面运行入口继续开发。

来源固定为参考仓 `ExchangeEtherBattery`、三份 `item_processing*.yml` 和[道具处理画面](../../../ZenlessZoneZero-OneDragon/docs/game/screens/道具处理.md)。入口增加快捷手册区域、原电池模板的原图 / 二值掩码和说明，掩码沿用现有绿色屏蔽转换。本地 `resources/ether-synthesis/source-lock.json` 与 `resources/ether-synthesis-entry/source-lock.json` 校验来源，入口构建还核对内嵌模板与参考原图一致。两个独立 schema 1 包只包含只读识别节点和锁定 OCR 模型，不包含可直接消费资源的 Pipeline 动作。正式可用标记仍为 `accepted=false`。

导航先在已识别的快捷手册资源栏定位电池模板。详情用完整“以太电池”及“[获取]合成”文字定位，不猜测按钮坐标；候选区域只保留于当次调用，既不序列化，也不直接用旧帧点击。点击前重新截图，在局部区域复核名称、入口和已知覆盖层，再采用新帧按钮位置。快捷手册和到达合成页采用各自的有限区域读取，避免全屏 OCR 使输入前的新帧失效。选择合成入口后的等待上限为 20 秒活动时间，暂停不清零，意图不确定时不重新点击。

同一帧必须识别道具处理标题、以太电池名称和拥有量、合成数量 ×1、当前电量 / 60 以及合成按钮。素材不足或电量低于 60 返回 `Unavailable`。数量不同、读数缺失或无法确认身份的弹窗不能发送输入。点击合成前再次读取新帧，要求资源与保存意图时一致；若已确认未投递且电量自然增加，重新读取并保存基线。每次动作都独立保存意图，发布失败不投递，原生错误保留不确定意图，暂停后不重复消费。

消费确认要求完整的“合成 1 个以太电池”确认句、取消 / 确认按钮，以及素材区的电量 60、储值电卡 1、丁尼 100。这里按真实留档建立检查，参考画面文档中“100 储值电卡”的描述与截图不符，不采用该错误数值。领取要求获得标题、以太电池名称、数量 1 及确认按钮。最后返回合成页，电池必须恰好增加 1；电量须处于“原电量 −60”至该值与 240 的较大者之间，允许期间自然恢复，不假设恢复速率。原电量在自然上限以上时不能凭空增加。额外电量消耗、超出自然恢复范围或电池数量异常仍拒绝；缺少确认 / 获得 / 领取任一环节也不能采用。原有严格差值日志继续通过同一重放校验。

`SynthesisRun` 只从内部保存并释放输入的正常终态提供结果，私有执行 ID 在暂停继续间保持不变。父任务在首个导航输入前预检全部下游资源并冻结清单；交接保存失败不进入合成，父任务终态保存失败、暂停或停止均不开放 `verified_exchange()`。`ChargeRegistrar::record_synthesis_challenge_persisted` 由此取子流程，再复验同一控制句柄、运行 ID 和冻结计划，沿用日志先于配置的提交。保存失败保留旧结果、禁止重新执行，可在暂停或停止期间通过 `retry_pending_persisted` 补交。素材不足不加次数，本轮跳过该计划；所有候选均因素材不足而不可用时返回 `UnavailableMaterials`，不能视为完整成功。旧模拟室日志与去重规则保持兼容。

开发命令：

```powershell
cargo run --locked --target-dir target/m4 -p od-tools -- ether-synthesis-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
cargo run --locked --target-dir target/m4 -p od-tools -- compendium-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
cargo run --locked --target-dir target/m4 -p od-tools -- ether-synthesis-entry-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
target/m4/debug/zzz-engine.exe ether-synthesis-entry-bundle-check
target/m4/debug/zzz-engine.exe ether-synthesis-entry-inspect --image zzz-od-test/screens/快捷手册/目标TAB.webp
target/m4/debug/zzz-engine.exe ether-synthesis-bundle-check
target/m4/debug/zzz-engine.exe ether-synthesis-inspect --image zzz-od-test/screens/道具处理/合成确认.webp
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test ether_synthesis -- --test-threads=1
cargo test --locked --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation-restore-charge --test ether_synthesis_entry -- --test-threads=1
```

只读检查不连接游戏，不登记本次兑换成功；输出保持 `input_sent=false`、`completion_verified=false`，入口检查另有 `navigation_verified=false`。回归采用真实快捷手册、四张真实合成留档与明确标注的资源详情 / 返回组合画面；资源详情缺少真实留档，不能把模拟画面当作实机证据。真实导航、消费、持续多次合成、实际暂停停止与异常恢复，以及正式资源发行仍待验收。用户 2026 年 10 月 28 日明确选择暂不启动游戏，继续开发并保留这些验收项。
