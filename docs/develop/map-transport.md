# M3 地图传送首条路线

本批实现咖啡前置的“澄辉坪 → 汀曼咖啡”，起点为已经打开的普通地图，终点为传送后识别到大世界。流程使用固定 Maa Pipeline，Rust 保存选择及确认检查点。当前仅开放离线截图检查和测试仓的受控执行，不接入真实游戏、应用组或每日成功记录。

## 证据与范围

先阅读参考项目的传送机制、地图画面、`map.yml`、`MapTransport` 和 `CoffeeApp`。三个真实留档分别为测试仓 `screens/地图/区域选择.webp`、`选传送点.webp` 和 `传送确认弹窗.webp`。大世界沿用邮件已验证的“功能导览 + 预备编队”两个模板。普通地图入口是 N，3D 地图入口是 M；本批不处理打开地图或返回起点。

确认弹窗包含“是否传送至该地点？”和“确认”，两者同时命中才允许确认。提问文字区域依据上述弹窗留档设为 1080p 的 `[700, 480, 1220, 570]`，按钮沿用 `map.yml`；仅有通用“确认”不构成传送依据。区域与传送点严格限制在各自区域内，固定文字精确匹配，不采用旧版的宽松 LCS。首版只处理可见目标，不迁移横向拖动和地图目录；缺少或未解锁目标时超时退出。

## 顺序与恢复

先选择澄辉坪，再确认汀曼咖啡可见，然后选择传送点、检查传送弹窗、确认、等待大世界。选择区域和传送点时必须同时识别清晰的地图返回按钮与目标文字；弹窗留档中的模糊背景不能通过这些检查。任务开始时遇到已有弹窗直接拒绝，不能认领来源未知的目的地。

固定 SDK 的组合识别引用仅取子节点的识别算法和参数，不执行子节点的 `inverse`。因此不能把反转 OCR 节点嵌在 `And` 中充当“没有弹窗”的条件；本实现使用上述正向锚点及起点检查。

投递选择传送点前保留检查点，暂停后仅重新检查弹窗；投递确认前保留确认检查点，暂停后仅等待落地。对是否执行存在不确定性时不重复传送。每次 Pipeline 收尾等待 Maa 队列并释放鼠标；暂停时间不计入有效超时，停止不能被继续覆盖。取消和超时仍须确认释放，释放失败不能报告成功或暂停。

离线的多帧测试使用真实 Maa Tasker 与会对点击换帧的自定义控制器，验证从地图到大世界、暂停恢复、停止、超时、无关确认、未解锁和输入失败。模拟落地只验证流程，不证明真实目的地或实机成功。

## 开发验证入口

准备锁定的 Maa SDK 和 `sdk/models/ppocrv5` 后执行：

```powershell
cargo run -p od-tools --locked -- map-bundle --reference E:/Code/ZenlessZoneZero-OneDragon
cargo run -p zzz-engine --locked -- map-bundle-check --bundle resources/generated/map-preview
cargo run -p zzz-engine --locked -- map-inspect --bundle resources/generated/map-preview --image zzz-od-test/screens/地图/传送确认弹窗.webp
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/validation --locked --test map_transport --test coffee --test group_schedule -- --test-threads=1
```

`map-bundle-check` 只校验文件，不加载 SDK 或创建账号数据；`map-inspect` 使用无输入的固定截图控制器，只返回 `area_visible`、`point_visible`、`transport_dialog`、`world_visible` 或 `unknown` 及原生详情。这些观察值均不构成应用完成。真实留档识别报告写入测试仓 `artifacts/m3-map-observations.json`。

生成器核对旧区域、模板与二值掩码，复制锁定 OCR 和字典，拒绝来源与输出重叠及非受管文件。加载侧再次验证固定路线、完整文件清单、模型哈希、目录联接及 Pipeline 全部节点；重新填写清单哈希不能修改确认条件或模型。资源只用于开发预览，未加入正式发行清单。桌面协议继续为 v13。

咖啡点单后态素材和真实往返仍缺失。此批完成后继续补咖啡点单后的跳过、电量、额外咖啡与已达上限分支，再接入真实执行及多应用交接。
