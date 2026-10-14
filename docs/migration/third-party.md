# 来源与许可证

- 参考源码与样例：[ZenlessZoneZero-OneDragon](https://github.com/OneDragon-Anything/ZenlessZoneZero-OneDragon)，本轮固定提交见迁移清单，GPL-3.0；主仓 `LICENSE` 保留原许可证。转换模板和配置沿用原来源，不把它们变成独立无来源素材。
- 原生 SDK：[MaaFramework v5.14.3](https://github.com/MaaXYZ/MaaFramework/releases/tag/v5.14.3)，LGPL-3.0，下载校验值固定于 `sdk.lock.json`。发布时动态分发 SDK，保留其 `LICENSE.md` 与源代码地址。
- 官方绑定：[maa-framework-rs](https://github.com/MaaXYZ/maa-framework-rs)，固定 `maa-framework 1.25.2` 和 `maa-framework-sys 5.14.3`，LGPL-3.0。
- 原始张量补充绑定：[ort](https://github.com/pykeio/ort/tree/v2.0.0-rc.10)，固定 `ort` / `ort-sys` 2.0.0-rc.10，MIT / Apache-2.0。禁用原生库下载和复制，动态使用固定 Maa SDK 的 `onnxruntime_maa.dll`，不另行分发其他 ORT DLL。
- 桌面：[Tauri](https://github.com/tauri-apps/tauri)，MIT / Apache-2.0；[React](https://github.com/facebook/react)、[Fluent UI](https://github.com/microsoft/fluentui) 与 [Vite](https://github.com/vitejs/vite)，MIT。
- Maa SDK 包含 OpenCV、ONNX Runtime、DirectML 等原生依赖。本轮记录实际 DLL SHA-256；M7 仍须核对每项原生依赖的许可、可再分发范围与发布清单。该步骤未由编译成功代替。

Rust 与前端精确依赖分别保存在 `Cargo.lock` 和 `apps/desktop/package-lock.json`。SDK 中的 Node 包装器、CLI、示例插件和 AgentBinary 不列入本项目运行包；只打包固定运行 DLL。

M1 离线模型缓存来自参考项目配置使用的 [OneDragon-Env PP-OCRv5 发布](https://github.com/OneDragon-Anything/OneDragon-Env/releases/tag/ppocrv5)和 [OneDragon-YOLO 模型发布](https://github.com/OneDragon-Anything/OneDragon-YOLO/releases/tag/zzz_model)。发布包和使用文件的 SHA-256 在 `models.lock.json` 固定，来源不变，字典和标签保留。缓存与转换结果不作为源码提交，也未加入当前便携包；正式模型分发的许可与来源清单继续按 M7 核对。
