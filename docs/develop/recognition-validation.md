# M1 模型对照

本轮实现固定模型获取、Maa 资源转换和离线 CPU / DirectML 验证入口。模型版本以参考提交的默认配置为准，`models.lock.json` 固定官方发布包及使用文件的 SHA-256；缓存放在 `sdk/models`，转换资源放在 `resources/generated/recognition`。参考仓和来源模型不修改，不使用 Python 构建或验证。正式应用继续保持待迁移，模型能加载和旧版行为兼容分别验收。

OCR 采用旧 PP-OCRv5 检测、识别与原字典，字典原样复制为 Maa 的 `keys.txt`；旧方向分类默认关闭，可视化字体不进入引擎资源。闪光与两种检测模型保留标签索引，不重新训练或替换模型。资源转换先检查全部文件，再复制并校验输出，生成资源附带差异清单。

验证只通过固定图自定义控制器调用 Maa 独立识别，不产生游戏输入。按同一份有哈希的语料，先完成 CPU，再销毁 CPU 会话并创建 DirectML 会话；同一进程中的 DirectML 会话共用独占使用规则。每个案例只准备一次固定图，解码 / 编码 / Maa 图像缓存时间记录为 `fixture_prepare_us`，不混入正式推理样本。首次识别调用与预热、正式样本分开计时，记录 SDK 投递、完成等待及详情读取的时间，并计算 P50 / P95；这些仍不包括游戏截图链路。未命中保留为真实识别结果，模型缺失、校验失败和详情缺失属于错误。

CPU / DirectML 对照忽略原生请求 ID 和结果顺序，逐项比较文字、类别、框及分数，并使用按案例登记的容差。旧版文档快照与同机实际旧输出分别标记；没有旧输出或旧版测量时不能宣称兼容、性能目标通过。DirectML API 返回成功也不能证明整个图都在 GPU 执行；报告保留设备请求与可取得的原生日志证据，未取得证据时不标记为已验证的 DirectML 性能。

固定 Maa 5.14.3 与旧实现已发现的差异：

- 闪光：旧版保持比例、以 `INTER_LINEAR` 缩放并在右下补 114，直接读取模型输出并使用 0.9 阈值；Maa 分类使用 `INTER_AREA` 拉伸 ROI 并再次 softmax，原生分数不能代替旧置信度。需补充适配或自定义识别后再验收闪避行为。
- YOLO：Maa 居中补边，旧版在右下补边；尺寸取整、插值、候选过滤、框裁剪和 NMS 还需对照。迷失之地旧实现另遮盖头像区域，本轮全帧调用用于识别能力诊断。
- OCR：Maa 使用 FastDeploy PPOCR 路径，旧版最长边限制、识别高度、框扩张和 LCS 语义仍需逐项对照。字典相同不代表所有预处理相同。

Maa 依据固定版本的[识别协议](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/docs/en_us/3.1-PipelineProtocol.md)、[分类源码](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaFramework/Vision/NeuralNetworkClassifier.cpp)、[检测源码](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaFramework/Vision/NeuralNetworkDetector.cpp)和[OCR 资源约定](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaFramework/Resource/OCRResMgr.h)。旧实现的具体版本与结果在独立测试仓案例和验收记录中维护。

运行入口见根 README。案例清单位于独立测试仓 `test/zzz_auto/fixtures/recognition/cases.json`，每例登记路径、SHA-256、参数、期望来源和设备比较容差。本轮包含四个邮件 OCR 锚点及一个未命中分支、两张战斗分类画面、两种检测模型的能力调用。空洞模型当前使用非地图画面，仅证明调用和结果格式；缺少正例，不能验收空洞检测。闪光尚缺红光 / 黄光及阈值附近的已标注样本。
