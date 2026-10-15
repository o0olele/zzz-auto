# M1 模型对照

本轮实现固定模型获取、Maa 资源转换和离线 CPU / DirectML 验证入口。模型版本以参考提交的默认配置为准，`models.lock.json` 固定官方发布包及使用文件的 SHA-256；缓存放在 `sdk/models`，转换资源放在 `resources/generated/recognition`。参考仓和来源模型不修改，新程序的构建与正式验证不使用 Python；一次性采集旧版证据使用下文的隔离环境。正式应用继续保持待迁移，模型能加载和旧版行为兼容分别验收。

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

每个原生会话缓存检测会话，和 Maa / 闪光共用识别互斥与 GPU 使用权。报告单列模型校验和会话创建、预处理、推理及后处理，并记录实际模型 / 标签哈希、输出形状和 DirectML 注册证据。测试仓覆盖已知张量、阈值相邻 float32、NMS 相等边界、类别过滤、越界与整数框、遮罩端点、真实 SDK 合成模型及固定图设备对照。取消约定见[原始推理的取消](#原始推理的取消)，真实正例、同机旧输出与性能仍单独验收。

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

阈值附近的已知概率、RGB / NCHW、CPU / DirectML、错误模型和设备失败已通过回归；10 月 15 日补入真实红光 / 黄光与消失帧，记录见[新增红黄光帧](#新增红黄光帧)。同机旧实现输出、真实阈值附近样本及完整性能基线仍待补齐。SDK 与旧版 ORT / OpenCV 的计算差异仍需真实语料对照，原始分数可读不等于已经验收全部旧行为。

## 原始推理的取消

`LegacyFlashRaw` 和 `LegacyYoloDetect` 共用每次调用独立的 `RawRecognitionControl`。调用方可以从其他线程请求取消并查看阶段；一个控制对象只用于一次调用，取消不可撤销。原有识别入口继续使用相同实现，无取消时返回格式保持不变。显式取消入口只接收这两种原始推理类型，Maa 原生识别与 logits 入口的取消另行验收。

图像读取、模型会话创建、预处理、原生推理和后处理分别检查取消，Rust 张量转换每 4096 像素、YOLO 数值校验每 4096 元素、候选处理每 128 项及每轮 NMS 再检查一次。在途 ORT 调用使用独立的 `RunOptions`，取消线程调用 `terminate()`；请求、登记和移除当前 RunOptions 使用同一把锁，避免请求刚好发生在推理开始前被遗漏。依据 [ONNX Runtime 取消接口](https://onnxruntime.ai/docs/api/c/struct_ort_api.html)，请求只设置停止标记，调用线程仍同步等待原生 `run_with_options` 返回。DirectML 已提交的计算、单个算子、模型加载与 SDK OpenCV 调用不承诺立即中断；不通过提前返回或超时释放会话与 GPU 使用权。

只要取消先于最终结果提交，结果均丢弃，返回可识别的 `RawRecognitionCancelled` 错误。错误记录请求阶段、调用总耗时、请求到收尾完成的耗时、是否投递原生停止、原生调用是否返回及原生错误；取消不能变成普通未命中。终态与取消请求通过同一把锁确定先后，完成之后的取消不改变结果。未请求取消的模型或推理失败保持原错误。每次新调用重新创建 RunOptions，停止标记不会污染缓存会话的后续调用。

CLI `recognize` 为这两种类型接入 Ctrl+C 与 `--cancel-after-ms`，计时从识别调用开始，包括图像读取和首次模型加载。取消时输出结构化错误，直到原生调用和当前数据已收尾才退出；其他类型显式指定取消参数时拒绝。该离线接口没有游戏输入，不能据此验收战斗、Pipeline 暂停或崩溃恢复。CPU / DirectML 在途取消、会话复用、阶段取消及停止耗时通过独立测试仓验收。

Maa 依据固定版本的[识别协议](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/docs/en_us/3.1-PipelineProtocol.md)、[分类源码](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaFramework/Vision/NeuralNetworkClassifier.cpp)、[检测源码](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaFramework/Vision/NeuralNetworkDetector.cpp)和[OCR 资源约定](https://github.com/MaaXYZ/MaaFramework/blob/v5.14.3/source/MaaFramework/Resource/OCRResMgr.h)。旧实现的具体版本与结果在独立测试仓案例和验收记录中维护。

运行入口见根 README。初始案例清单位于独立测试仓 `test/zzz_auto/fixtures/recognition/cases.json`，每例登记路径、SHA-256、参数、期望来源和设备比较容差，包含四个邮件 OCR 锚点及一个未命中分支、两张战斗分类画面、两种检测模型的能力调用。后续存档扩展与新增闪光帧见下文。空洞模型仍缺电视格子地图正例，真实闪光阈值附近样本仍待补。

## 使用已有存档截图

2026 年 10 月 15 日首次核对独立测试仓 `screens/`：有 142 个图像文件，部分 PNG / WebP 为同一画面的不同存储版本，不按 142 个独立场景计算。OCR 已有大量可用图片；迷失之地大世界的玛琳蓝色事件标记也已存在，并被此前 YOLO 对照引用。随后补入的红光 / 黄光 PNG 见[新增红黄光帧](#新增红黄光帧)。空洞电视格子地图正例、真实阈值附近样本与同机旧版实际输出仍待补齐。

新增 `test/zzz_auto/fixtures/recognition/screen-cases.json`，复用六个现有文件，人工查看像素后登记 12 个案例。OCR 包括城市导航的「录像店」「六分街」、日常手册的「今日最大活跃度」「400」、饮茶仙的「邻里心愿」「10000」、商店的「购买」，以及同一区域找不到「验证码」的分支。正例不设置 Maa expected 过滤，保留 ROI 内完整文字结果；负例单独验证指定文字不存在。ROI 按当前固定图片核对，不直接作为旧业务 screen_info 的迁移结论。

YOLO 包括迷失之地事件标记正例、商店中不存在该事件标记的负例，以及城市导航中不存在空洞格子目标的诊断负例；新增式舆防卫战画面的无闪光案例。蓝色事件标记按人工框 `[905,330,85,85]` / 12px 容差核对，框为 xywh；没有从本轮模型输出反写期望。全部案例的 `legacy_evidence` 为 `none`，人工预期不冒充旧版实测。

CPU 与 DirectML(0) 每案预热 5 次、正式采样 30 次，共 720 个正式样本。12 组设备输出均在登记容差内一致，各设备 11 / 12 项符合人工预期。空洞模型把城市导航图案误检为 `0027-安全区`，保留预期未命中和失败状态；需结合旧版对照及调用前的画面判断确定处理方式。没有因此改阈值或模型。报告位于测试仓 `observations-screens-20261015.json`，具体观察见[验收记录](../migration/acceptance.md#2026-年-10-月-15-日复用存档截图扩充识别案例)。

复跑命令如下。因为包含上述未通过案例，命令当前预期以退出码 1 结束，仍写出完整报告。OCR 的设备配置对照不能证明算子实际在 GPU 执行；识别兼容与整体性能继续等待旧版输出和其他阶段证据。

```powershell
cargo run -p zzz-engine --locked -- --resources resources/generated/recognition recognition-check --cases zzz-od-test/test/zzz_auto/fixtures/recognition/screen-cases.json --fixture-root zzz-od-test --output artifacts/screens-report.json --directml 0 --warmup 5 --samples 30
```

## 新增红黄光帧

用户在 `screens/战斗画面/` 补入 24 个 1920×1080 PNG，约 50.8 MiB。逐图查看并核对文件哈希：`红光A-出现前.png` 与 `红光A-出现1.png` 完全相同，画面中已有红光；保留两个来源文件，在案例清单的 `annotation.duplicate_files` 中登记，只用「出现1」作为独立案例，不把同一图片同时标为红光与无闪光。

新增 `test/zzz_auto/fixtures/recognition/flash-sequence-cases.json`，含 23 个独立帧：红光 A / B 各 5 个出现帧，黄光 A 的 5 帧和 B 的 4 帧，以及四组的消失帧。出现帧由用户文件说明和逐图视觉核对标为红 / 黄，消失帧标为无闪光，保留战斗特效和残影。源图、模型、阈值均保持原样；人工期望不根据模型结果修改，`legacy_evidence` 全为 `none`。图片序号没有时间戳，不用于闪光响应时序或延迟验收。

全部案例使用 `LegacyFlashRaw`、固定 `yolov8n-640-flash-20250921.onnx`、全帧 ROI 和原阈值 0.9；无闪光类别为 0，红 / 黄为 1 / 2，最高概率低于有效阈值时原结果为 -1。三个概率、候选、阈值后的类别及实际模型哈希写入报告。CPU / DirectML 正式结果及人工预期差异见[验收记录](../migration/acceptance.md#2026-年-10-月-15-日红黄光真实帧对照)。

```powershell
cargo run -p zzz-engine --locked -- --resources resources/generated/recognition recognition-check --cases zzz-od-test/test/zzz_auto/fixtures/recognition/flash-sequence-cases.json --fixture-root zzz-od-test --output artifacts/flash-sequence-report.json --directml 0 --warmup 5 --samples 30
```

## 旧版实际输出与自动比较

先完成闪光原始输出的同图对照。旧版采样工具放在独立测试仓，只读取固定参考提交和模型缓存，直接调用原 `Yolov8Classifier.run` 的预处理、推理及判定；记录原始三类概率时调用原后处理，不复制一份分类算法。Python 3.11.9 与参考 `uv.lock` 中的识别依赖安装在主仓忽略的 `sdk/legacy-baseline/`，与 Maa DLL 隔离进程；新程序、构建和日常 Rust 回归不依赖这个环境。

采样前校验参考提交、实际导入源码、依赖版本、模型、图片及案例清单的哈希。CPU 和 DirectML 分进程顺序运行；保留首次调用、预热、正式样本、稳定性与提供程序配置。旧版结果与人工期望分别保存，四个人工标注差异不能因为新旧一致而消失。

Rust `recognition-compare` 读取固定采样约定、旧输出和新输出，无需 Python 或加载 SDK。比较前校验案例清单、模型、源码和运行环境的登记信息，以及设备、完整案例集合、样本数量和结果稳定性；按案例容差比较全部概率、候选、命中与框。输入不匹配时拒绝比较，缺失或错误结果不能算相同。结论只覆盖本清单中的闪光输出，不自动提升为全部识别兼容或 M1 完成；性能和 OCR / YOLO 仍分别验收。

在工程根目录执行以下命令。准备工具只安装固定清单里的识别依赖，已有缓存时可加 `-Offline`，不安装到系统 Python 或参考仓。采样工具仅支持 `LegacyFlashRaw` 和 CPU / DirectML(0)，直接读取已由 `setup-models.ps1` 校验的原模型缓存。旧版概率通过继承原分类器的 `process_output` 旁路记录，再调用原后处理；未改旧版预处理、推理或类别判定。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File zzz-od-test/test/zzz_auto/legacy/setup.ps1
sdk/legacy-baseline/python/python.exe -B zzz-od-test/test/zzz_auto/legacy/export_flash.py --reference E:/Code/ZenlessZoneZero-OneDragon --fixture-root zzz-od-test --cases zzz-od-test/test/zzz_auto/fixtures/recognition/flash-sequence-cases.json --output artifacts/legacy-flash.json --directml
cargo run -p zzz-engine --locked -- --resources resources/generated/recognition recognition-check --cases zzz-od-test/test/zzz_auto/fixtures/recognition/flash-sequence-cases.json --fixture-root zzz-od-test --output artifacts/current-flash.json --directml 0
cargo run -p zzz-engine --locked -- recognition-compare --cases zzz-od-test/test/zzz_auto/fixtures/recognition/flash-sequence-cases.json --fixture-root zzz-od-test --runtime-lock zzz-od-test/test/zzz_auto/legacy/runtime.lock.json --legacy artifacts/legacy-flash.json --current artifacts/current-flash.json --output artifacts/flash-comparison.json
```

旧版与新版采样顺序运行，避免跨进程 GPU 重叠。新报告增加 `case_manifest_sha256`，绑定包括参数和容差在内的整份案例清单；旧报告缺少此字段时须重跑，不能补写哈希冒充原始采样。比较入口输出每设备每案例的两份观察、概率最大绝对差、人工预期检查及错误。命令退出码只表示新旧输出比较；两版都有的人工标注失败仍保留为 `all_human_expectations_passed=false`，不会被改写为通过。采样时间含旧版完整 `run`，新报告保留 debug 各阶段时间，本轮不进行性能验收。

## OCR 与 YOLO 旧版对照

下一轮复用存档 OCR 和两种 YOLO 案例，单独固定扩展采样环境与采样器，保留上一轮闪光报告和采样约定。OCR 调用原 `OnnxOcrMatcher.run_ocr` / `match_words`，模型用原 `ONNXPaddleOcr` 加载固定缓存；YOLO 调用原检测器，迷失之地继续调用原头像遮罩。只调整采样器的模型位置，不修改参考仓或重写原识别算法。

OCR 同时保存过滤前与过滤后结果，逐项比较文字、框和分数。原案例的「验证码」负例仅对固定字面文字使用原 `same_word=True`，不将 Maa 的正则 expected 宣称为旧 LCS 兼容。正式报告保留算法差异，不能放宽容差或修改人工期望来获得通过。YOLO 验证固定模型、标签文件、阈值、类别过滤、NMS 与遮罩后的最终输出；现有素材仍不替代空洞电视地图正例或同分 NMS 边界验收。

扩展案例为 `ocr-yolo-cases.json`：取 `screen-cases.json` 的 8 个 OCR / 3 个 YOLO 案例，加上 `yolo-cases.json` 的 4 个适配案例，原图片、参数、人工期望及 2px / 0.01 容差不变。15 案包含重复图片与重复 ROI，不能算作 15 张独立图片。采样器 `export_recognition.py` 和 `recognition-runtime.lock.json` 单独固定，不改闪光原采样器和原采样约定。扩展依赖增加参考 `uv.lock` 中的 Pillow 12.3.0、Shapely 2.1.2 和 pyclipper 1.4.0，采样前后核对 57 个参考源码文件，实际导入未登记源码时拒绝保存。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File zzz-od-test/test/zzz_auto/legacy/setup.ps1 -LockFile zzz-od-test/test/zzz_auto/legacy/recognition-runtime.lock.json
sdk/legacy-baseline/python/python.exe -B zzz-od-test/test/zzz_auto/legacy/export_recognition.py --reference E:/Code/ZenlessZoneZero-OneDragon --fixture-root zzz-od-test --cases zzz-od-test/test/zzz_auto/fixtures/recognition/ocr-yolo-cases.json --output artifacts/legacy-ocr-yolo.json --directml
cargo run -p zzz-engine --release --locked -- --resources resources/generated/recognition recognition-check --cases zzz-od-test/test/zzz_auto/fixtures/recognition/ocr-yolo-cases.json --fixture-root zzz-od-test --output artifacts/current-ocr-yolo-release.json --directml 0
cargo run -p zzz-engine --release --locked -- recognition-compare --cases zzz-od-test/test/zzz_auto/fixtures/recognition/ocr-yolo-cases.json --fixture-root zzz-od-test --runtime-lock zzz-od-test/test/zzz_auto/legacy/recognition-runtime.lock.json --legacy artifacts/legacy-ocr-yolo.json --current artifacts/current-ocr-yolo-release.json --output artifacts/comparison-ocr-yolo.json
```

本轮正式结果为 30 项同设备比较中 24 项一致：YOLO 两设备各 7 / 7 项通过，OCR 各 5 / 8 项通过。标题与同 ROI 负例的框宽差 3px，超过原 2px 容差；「400」在旧版检测阶段没有文字框，新版能正确识别。负例过滤后两边都为空，过滤前的标题框差异仍使比较失败。旧版与新版的城市导航空洞误检相同，人工预期继续未通过。`recognition-check` 与 `recognition-compare` 均以退出码 1 保存完整报告，不能因有失败而丢弃报告。

新报告增加 `build_profile`，本轮为 release。正式计时不含固定图片解码，但旧采样包含结果整理、裁 ROI 与 RGB 转换在计时外，新路径阶段定义不同；原始 YOLO 的 Rust CPU 会话固定单线程，旧版使用默认线程池，ORT 版本也不同。保留每例 30 个正式计时样本和 P50 / P95 作诊断，不能据此宣布达到 110% 性能目标。下一步先对齐测量范围并评估 CPU 线程策略，再补完整识别与动作链的性能；具体观察见[验收记录](../migration/acceptance.md#2026-年-10-月-15-日-ocr-yolo-旧版输出与-release-采样)。

## 原始推理 CPU 线程与统一计时

先用现有 7 个 YOLO 案例做控制变量实验，保持固定模型、图片、ROI、阈值及 NMS 不变。新旧计时统一从已解码的完整 1080p BGR 帧开始，包含颜色转换、遮罩、缩放、张量、推理、后处理与标准结果生成；固定图解码、模型加载、预热、稳定性比较、文件写出不进入正式样本。原生各阶段时间继续单列，新增加完整调用边界的样本，不能用旧报告的阶段 total 冒充统一计时。

线程选择属于原始 ORT 会话，不改变 Maa OCR 或原生 Pipeline 的线程配置。离线诊断允许显式选择 CPU intra-op 线程数，0 表示 ORT 默认，1 表示之前的单线程；会话创建后不能修改，报告记录请求值。保持顺序执行、inter-op 单线程和关闭自旋；DirectML 保留原线程设置及 GPU 独占规则。根据同机 release 输出与 P95 选择默认值，再复验闪光、取消、会话复用与输入清理。

旧版统一计时采样器与约定单独保存，不改此前采样器或存档。比较器仅在两份报告声明相同计时边界、新版为 release、样本完整稳定且输出一致时计算离线 P50 / P95 与 110% 目标；范围只限固定帧识别调用，不代替游戏截图、动作、IPC、停止或长期稳定性验收。

`recognition-check --raw-cpu-threads 0..64` 仅配置原始闪光 / YOLO 的 CPU 会话，0 交给 ORT 选择线程数，省略时采用默认策略。它不是实际工作线程数量的测量值；DirectML 的 intra / inter 仍各为 1。报告的 `adaptation.threading` 记录每会话设置，创建后不能修改。`boundary_measurements_us` 和 `boundary_timing` 记录统一边界的正式样本与分位数，原 `measurements` / `timing` 保留其阶段含义。

在没有其他本项目推理、构建或测试运行时，先编译，再顺序采样：

```powershell
cargo build -p zzz-engine --release --locked
sdk/legacy-baseline/python/python.exe -B zzz-od-test/test/zzz_auto/legacy/benchmark_yolo.py --reference E:/Code/ZenlessZoneZero-OneDragon --fixture-root zzz-od-test --cases zzz-od-test/test/zzz_auto/fixtures/recognition/yolo-performance-cases.json --output artifacts/yolo-legacy-aligned.json --samples 60
target/release/zzz-engine.exe --resources resources/generated/recognition recognition-check --cases zzz-od-test/test/zzz_auto/fixtures/recognition/yolo-performance-cases.json --fixture-root zzz-od-test --output artifacts/yolo-cpu-default.json --samples 60
target/release/zzz-engine.exe recognition-compare --cases zzz-od-test/test/zzz_auto/fixtures/recognition/yolo-performance-cases.json --fixture-root zzz-od-test --runtime-lock zzz-od-test/test/zzz_auto/legacy/yolo-benchmark.lock.json --legacy artifacts/yolo-legacy-aligned.json --current artifacts/yolo-cpu-default.json --output artifacts/comparison-yolo-cpu.json
```

控制变量时，为同一 release 引擎的识别命令分别传 `--raw-cpu-threads 1`、`0`、`2`、`4`、`8`，各输出不同文件；不要同时运行。旧版使用原 `Yolov8Detector` 及迷失之地遮罩，新增 BGR→RGB 转换在计时内，模型初始化、解码与稳定性比较在计时外。新采样器通过原冻结约定验证环境和源码，并用 `yolo-benchmark.lock.json` 固定新增工具与辅助文件。

比较器对统一计时要求双方至少预热 5 次、至少 30 个正式样本，并验证边界、release、样本数量和输出；不合条件时不将旧阶段计时当成可比性能。可比较时逐案给出 `performance`、P95 比例与 110% 结果，汇总为 `all_performance_targets_passed`。任何输出差异、错误、不稳定、缺失样本或超出目标均不能通过；旧版误检的人工预期失败仍单列。没有旧统一计时的历史报告继续只比较输出，性能状态为未提供。

2026 年 10 月 15 日在本机完成五组 30 次正式样本实验，ORT 默认与固定 8 线程的 7 案均达到离线 P95 目标；默认改为 0，避免将本机核心数量写死为所有机器的设置。用默认入口再做每边 60 次正式样本确认，7 / 7 项输出及离线 P95 目标均通过。对照、确认、闪光复验及 SHA-256 索引存放在测试仓 `fixtures/recognition/yolo-threading-20261015/`，具体数据见[验收记录](../migration/acceptance.md#2026-年-10-月-15-日原始-yolo-cpu-线程与统一计时)。`legacy_performance_available=true` 只说明该比较报告具备本范围的同机旧计时，不表示全部 M1 性能通过。
