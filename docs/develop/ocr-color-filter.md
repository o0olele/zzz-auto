# OCR 颜色范围适配

本批继续补路线图中的颜色规则迁移。`OcrText` 接受旧版 RGB 颜色上下界，复用锁定 Maa 5.14.3 的原生 `OCR.color_filter`，再沿用现有文字匹配、置信度和全图坐标处理。这里只读取图片，不增加游戏输入或领奖入口。

旧 `OcrService._apply_color_filter` 对 RGB 原图执行包含上下界的 `inRange`，随后将单通道掩码转换成三通道黑白图。Maa 的原生实现可完成同样的二值化；其截图为 BGR，因此引用 `method=4`（BGR 转 RGB）的 `ColorMatch` 节点。RGB 阈值保持原顺序，命中像素送入 OCR 时变为白色，其余为黑色，黑色范围也按此方式处理。

来源为固定 [Maa OCR 协议](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/docs/zh_cn/3.1-任务流水线协议.md#ocr)及 [OCRer 源码](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaFramework/Vision/OCRer.cpp)，旧版处理位于参考仓 `src/one_dragon/base/matcher/ocr/ocr_service.py`。这只对齐颜色预处理含义，不证明新旧 OCR 模型、文字框或原始分数相同。

## 调用和约束

```json
{"model":"ppocrv5","roi":[708,140,928,80],"words":["日常"],"match_mode":"exact","color_range":[[200,160,0],[255,255,60]]}
```

通过现有 `recognize --kind OcrText --params <JSON文件>` 或 `recognition-check` 调用。上下界各为三个 0..255 整数，逐通道下界不得大于上界。省略或传 null 保留原图路径；不支持颜色空间、节点名或任意 Pipeline 注入，未知参数继续拒绝。ROI、单行识别、Unicode 文字规则、阈值精度限制仍按[OCR 业务调用适配](recognition-validation.md#ocr-业务调用适配)执行。

绑定资源时创建一个固定内部颜色节点，若资源已有同名节点则拒绝，防止覆盖资源内容。节点禁用、无后继且无输入动作，仅供 OCR 读取颜色参数。识别在现有互斥内更新该节点并等待原生调用完成，节点数量固定；无颜色过滤的后续调用不引用它。仅修改会话内存，不改写磁盘资源、模型、执行包或用户配置。

## 验证

新增 6 项 Rust 回归，包含颜色参数的形状、整数范围与逐通道上下界、节点名注入拒绝、无输入内部节点、实际 SDK 的 RGB 包含端点、范围外像素及通道顺序、原生过滤与参考黑白 PNG 的逐结果一致性、透明通道为 255 的 PNG、黑色范围、连续切换过滤条件和内部节点冲突。后者同时确认原资源文件未被修改。

复用三张真实快捷手册留档形成 9 案：目标 / 日常 / 作战选中态、未选中标签、随后不带颜色过滤的调用、RGB 上下界互换以及全颜色范围变成纯白的负例。独立清单 `ocr-color-cases.json` 固定图片哈希，保留原始 OCR 对照与已有 3 处差异。黑白参考图片由 Rust 按旧版包含上下界的 RGB 规则生成，用于验证预处理含义；没有新增 Python 执行依赖，也没有将该对照描述成旧 OCR 输出一致。

连同既有 OCR 6 项、活跃度只读识别 12 项和邮件流程 27 项，共 51 项回归通过。首轮新增 SDK 调用先固定了进程日志目录，后续报告读取另一临时目录导致 DirectML 配置证据缺失；测试改为共用同一日志目录后，两组设备回归通过。邮件有一项旧测试仍假设咖啡无执行器，改为仍未迁移的活跃度奖励后单独复跑通过，继续检查整组拒绝与没有输入，不改变产品的执行器判定。

每案预热 5 次、采样 30 次，CPU 和 Maa DirectML 配置各 270 个正式样本，9 案均稳定并符合预期，9 组设备输出对照在原容差内一致。完整 debug 报告保存在主工程 `artifacts/ocr-color-sampling.json`；测试仓的 `fixtures/recognition/ocr-color-index-20261019.json` 保存分案输出、原图与清单指纹、实现文件哈希及完整报告哈希。首次写入测试仓的新目录失败后改存工程 artifacts，重新采样并以退出码 0 完成。报告使用 debug 构建，不作为性能验收；DirectML 原生日志只确认 NN 会话配置，不能证明 OCR 每个算子的实际执行设备。

```powershell
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/engagement-validation --locked --test ocr_text --test engagement_reward -- --test-threads=1
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/engagement-validation --locked --test mail_pipeline -- --test-threads=1
target/engagement-validation/debug/zzz-engine.exe --resources resources/generated/recognition --data artifacts/ocr-color-sampling-data recognition-check --cases zzz-od-test/test/zzz_auto/fixtures/recognition/ocr-color-cases.json --fixture-root zzz-od-test --output artifacts/ocr-color-repeat.json --directml 0 --warmup 5 --samples 30
```

引擎与原生 debug 桌面构建、相关 crate 和新增测试的严格 Clippy、格式与差异检查通过。桌面界面和业务协议未变，继续为 v28；当前诊断通过既有 CLI 使用，不修改咖啡或活跃度执行入口。训练和战术、其他颜色调用及游戏实际导航仍待对应素材或实机验收，活跃度完整领取继续保留原待办。
