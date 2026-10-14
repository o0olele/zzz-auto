# M1 模型对照

本轮实现固定模型获取、Maa 资源转换和离线 CPU / DirectML 验证入口。模型版本以参考提交的默认配置为准，`models.lock.json` 固定官方发布包及使用文件的 SHA-256；缓存放在 `sdk/models`，转换资源放在 `resources/generated/recognition`。参考仓和来源模型不修改，不使用 Python 构建或验证。正式应用继续保持待迁移，模型能加载和旧版行为兼容分别验收。

OCR 采用旧 PP-OCRv5 检测、识别与原字典，字典原样复制为 Maa 的 `keys.txt`；旧方向分类默认关闭，可视化字体不进入引擎资源。闪光与两种检测模型保留标签索引，不重新训练或替换模型。资源转换先检查全部文件，再复制并校验输出，生成资源附带差异清单。

验证只通过固定图自定义控制器调用 Maa 独立识别，不产生游戏输入。按同一份有哈希的语料，先完成 CPU，再销毁 CPU 会话并创建 DirectML 会话；同一进程中的 DirectML 会话共用独占使用规则。每个案例只准备一次固定图，解码 / 编码 / Maa 图像缓存时间记录为 `fixture_prepare_us`，不混入正式推理样本。首次识别调用与预热、正式样本分开计时，记录 SDK 投递、完成等待及详情读取的时间，并计算 P50 / P95；这些仍不包括游戏截图链路。未命中保留为真实识别结果，模型缺失、校验失败和详情缺失属于错误。

CPU / DirectML 对照忽略原生请求 ID 和结果顺序，逐项比较文字、类别、框及分数，并使用按案例登记的容差。旧版文档快照与同机实际旧输出分别标记；没有旧输出或旧版测量时不能宣称兼容、性能目标通过。DirectML API 返回成功也不能证明整个图都在 GPU 执行；报告保留设备请求与可取得的原生日志证据，未取得证据时不标记为已验证的 DirectML 性能。

固定 Maa 5.14.3 与旧实现已发现的差异：

- 闪光：旧版保持比例、以 `INTER_LINEAR` 缩放并在右下补 114，直接读取模型输出并使用 0.9 阈值；Maa 分类使用 `INTER_AREA` 拉伸 ROI 并再次 softmax，原生分数不能代替旧置信度。需补充适配或自定义识别后再验收闪避行为。
- YOLO：Maa 居中补边，旧版在右下补边；尺寸取整、插值、候选过滤、框裁剪和 NMS 还需对照。迷失之地旧实现另遮盖头像区域，本轮全帧调用用于识别能力诊断。
- OCR：Maa 使用 FastDeploy PPOCR 路径，旧版最长边限制、识别高度、框扩张和 LCS 语义仍需逐项对照。字典相同不代表所有预处理相同。

## YOLO 适配方案

已新增离线入口 `LegacyYoloDetect`，固定空洞事件与迷失之地模型，复用 SDK 的 OpenCV 和 ONNX Runtime，不增加原生 DLL、不改模型。Maa 原生检测入口保留用于对照。固定 SDK 的检测路径使用居中补边、INTER_AREA、多类别候选、框裁剪及自身 NMS，无法仅靠 Pipeline 参数恢复旧版行为，因此补充原始张量后处理。

直接检查固定 ONNX 图并经真实 SDK 验证：空洞模型输入为 `[1,3,416,736]`，输出为 `[1,87,6279]`，83 个类别；迷失之地输入为 `[1,3,736,736]`，输出为 `[1,20,11109]`，16 个类别。不能从模型名称里的 736 推断两者均为正方形。完整帧在两种模型中均先缩放到 736×414，再分别在底部补 2 行或 322 行。

适配只接收完整 1920×1080 BGR 帧：保持比例，按 Python 偶数舍入缩放，INTER_LINEAR 插值，左上对齐并在右下补 114；RGB / NCHW 归一化复用原始闪光路径。迷失之地缩放前将 `(104,40)..(844,110)` 涂黑，包含右下端点，与参考 `cv2.rectangle` 一致。标签 CSV 随模型资源生成并校验哈希，不接受调换标签索引；当前固定 CSV 无 category 列，非空 category 过滤不会凭空匹配类别。

输出要求 float32 `[1,4+C,N]`，类别过滤在 argmax 前执行，置信度严格大于 float32 阈值。坐标先按各轴实际缩放尺寸恢复为 float32，再转 xyxy；在浮点框上按类别执行 NMS，IoU 相等也抑制，最后向零取整并保留越界坐标。相同类别、相同分数且互相抑制的候选返回明确的排序歧义错误，不能把尚未核对的 NumPy 排序当作兼容通过。错误张量、非有限数值、无效框、取整后空框或超过 512 个结果返回诊断错误，不当作未命中。

每个原生会话缓存检测会话，和 Maa / 闪光共用识别互斥与 GPU 使用权。报告单列模型校验和会话创建、预处理、推理及后处理，并记录实际模型 / 标签哈希、输出形状和 DirectML 注册证据。测试仓覆盖已知张量、阈值相邻 float32、NMS 相等边界、类别过滤、越界与整数框、遮罩端点、真实 SDK 合成模型及固定图设备对照。真实正例、同机旧输出、性能、在途取消仍单独验收。

浮点运算顺序依据参考源码与 [NumPy 2.4 类型提升规则](https://numpy.org/doc/2.4/reference/arrays.promotion.html)：float32 数组与 Python float 阈值比较时保留 float32；与 int64 尺寸数组相乘按提升后的精度计算，再写回 float32。参考 NMS 的 `np.argsort(scores)[::-1]` 未指定 stable，依据 [argsort 文档](https://numpy.org/doc/2.4/reference/generated/numpy.argsort.html)不能假定同分稳定排序；需要同环境旧输出之后才能解除当前重叠同分拒绝。

## 闪光分类适配方案

本轮先补闪光分类，不扩展 YOLO 或游戏战斗流程。固定模型的末尾为 `Softmax(axis=1)`，输出为 `[1,3]` 概率；Maa 会再做一次 softmax，但公开 JSON 只包含最终分数，未公开内部的 `raw`。因此构建工具生成独立模型，把末尾 Softmax 改为 Identity，保留权重、输入、输出与其余节点。Maa 对 logits 做一次 softmax 后得到原模型概率。源模型与原生诊断入口继续保留，生成物的大小、哈希和来源登记在固定模型清单；不符合约定结构的模型拒绝转换。

Rust 适配入口 `LegacyFlashClassify` 接收原图 ROI，在 SDK 自带的 `opencv_world4_maa.dll` 中调用 `cvResize(INTER_LINEAR)`，按旧版的 round 规则保持比例，左上对齐并在右下补 114，生成 640×640 BGR 图后交给 Maa。Maa 仍负责 RGB / NCHW / 归一化及 CPU / DirectML 推理；不新增 ORT 或 OpenCV DLL。原图为 1920×1080，默认 ROI 为全帧。参数只接受固定衍生模型、原标签顺序、ROI 与默认 0.9 的阈值；不把原生 expected 过滤或分类分数当作旧判定。

适配结果的命中表示最大概率达到阈值，低于阈值对应旧版 `class_idx=-1`；命中的框映射回原 ROI。类别 0 表示模型识别为无闪光，类别 1 / 2 表示红光 / 黄光。报告保留阈值、候选类别与分数；即使 CPU 与 DirectML 都未命中，也比较候选类别、框和分数，不能因为结果列表都为空就忽略差异。图像解码单列，正式样本包括缩放、补边、Maa 调用与业务判定，不能与上一轮仅推理完成时间混为一项。

固定 SDK 的公开 JSON 分数保留六位小数，真实推理已复现：原概率约为 0.8999998，详情返回 0.9。分数与阈值之差不超过 0.000001 时，本入口报明确的精度错误，不能当成已命中、普通未命中或旧版阈值相等通过。此范围包括保守的 float32 计算余量。未来必须取得原始浮点输出或可靠的原生判定，才能验收此边界；不通过降低阈值或估算原始分数绕过。SDK 与旧版 OpenCV 的像素取整，以及归一化的浮点差异也仍需同机对照。

0.000001 是当前诊断的拒绝范围，不代表已经证明模型在两套推理路径下的整体浮点误差上限。

验证包括结构错误和生成物篡改拒绝、固定色序 / 补边 / 插值案例、已知 logits 的真实 Maa 推理、0.9 阈值两侧及舍入边界拒绝，以及固定战斗截图的 CPU / DirectML 对照。算法结构验证不代替同机旧实现输出：真实红光、黄光和阈值附近标注样本、旧版输出与性能基线未补齐之前，整体兼容性和 M1 继续待验收。

## 原始浮点输出方案

已增加 `LegacyFlashRaw`，通过 Rust `ort` / `ort-sys` 2.0.0-rc.10 动态绑定调用已经固定、校验并加载的 `onnxruntime_maa.dll`，读取原始分类模型的 float32 概率。DLL 实际版本为 1.29.0，绑定请求 API 22；先检查版本及 API 指针，再初始化，并确认 Rust 获得的 API 与 SDK 完全相同。SDK 文件哈希保持不变，绑定关闭原生库下载 / 复制，加载路径不取自环境变量或系统 PATH。此补充只解决 Maa 未公开原始张量和 JSON 六位小数的问题，不新增原生 DLL，不修改来源模型。原来的 Maa 原生分类、logits 适配与精度错误保留，便于同图对照。

原始输出使用同一份 SDK OpenCV 缩放与补边数据，再按旧版 RGB / NCHW / 除以 255 后转 float32 的顺序构造张量。CPU / DirectML 会话在所属 Maa 会话内缓存，使用相同设备；DirectML 注册失败必须报错，不能静默回退。原生推理结束并销毁会话后才释放统一 GPU 使用权，不与该会话的 Maa 任务并行执行。结果先校验输入输出名称、形状、类型与概率范围，再按最大值和阈值判断，直接序列化原始浮点数，不经过 Maa 分数 JSON。

参考仓锁定 NumPy 2.4.6，旧代码将 float32 模型分数与 Python float 阈值比较。按 [NumPy 2 标量类型提升规则](https://numpy.org/doc/2.2/numpy_2_0_migration_guide.html#changes-to-numpy-data-type-promotion)，本入口先把阈值转成 float32，再执行包含相等边界的比较；默认有效阈值为 0.8999999761581421。并列最大值取第一个索引，低于阈值保留候选与全部概率并返回 `class_index=-1`。这与先把分数提升到 float64 后直接比较 0.9 的行为不同，已用前后相邻 float32 值独立验证。

模型按校验过的内存内容创建会话，报告记录实际加载 SHA-256；正式固定图检查会核对它与模型锁文件一致。CPU / DirectML 对照比较三类原始概率、候选和命中结果。每个会话内增加识别调用互斥，Maa / logits / 原始推理不能并行使用同一会话；GPU 会话销毁时先销毁补充会话和 Maa 资源，再释放原有 GPU 使用权。

首次模型校验 / 会话创建由 `session_setup_us` 单列，正式采样区分预处理、同步推理和结果读取；补充路径没有 Maa 队列，报告的 `execution_path` 为 `sdk_onnxruntime_direct`，`enqueue` 为 0。DirectML 配置证据分为 Maa 日志与原始 SDK 注册成功，不能解释为全部算子都在 GPU 执行。注册时关闭内存模式和并行执行，遵循 [ONNX Runtime DirectML 约定](https://onnxruntime.ai/docs/execution-providers/DirectML-ExecutionProvider.html)。

阈值附近的已知概率、RGB / NCHW、CPU / DirectML、错误模型和设备失败已通过本轮回归；真实游戏正例、同机旧实现输出及完整性能基线仍待补齐。补充路径目前同步等待推理完成，没有对在途推理接入停止 / 取消；不把此诊断入口当成已具备自动战斗停止能力。SDK 与旧版 ORT / OpenCV 的计算差异仍需真实语料对照，原始分数可读不等于已经验收全部旧行为。

Maa 依据固定版本的[识别协议](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/docs/en_us/3.1-PipelineProtocol.md)、[分类源码](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaFramework/Vision/NeuralNetworkClassifier.cpp)、[检测源码](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaFramework/Vision/NeuralNetworkDetector.cpp)和[OCR 资源约定](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaFramework/Resource/OCRResMgr.h)。旧实现的具体版本与结果在独立测试仓案例和验收记录中维护。

运行入口见根 README。案例清单位于独立测试仓 `test/zzz_auto/fixtures/recognition/cases.json`，每例登记路径、SHA-256、参数、期望来源和设备比较容差。本轮包含四个邮件 OCR 锚点及一个未命中分支、两张战斗分类画面、两种检测模型的能力调用。空洞模型当前使用非地图画面，仅证明调用和结果格式；缺少正例，不能验收空洞检测。闪光尚缺红光 / 黄光及阈值附近的已标注样本。
