# M3 已保存画面素材查看

在“画面素材”页补充已保存素材列表，帮助核对咖啡和活跃度奖励的留档。列表从固定数据目录 `diagnostics/screens/` 读取，重启、引擎断线或运行中仍可查看。当前采集预览与历史回看分别展示，查看历史不会替换待保存的图片。

支持按人工状态与备注筛选、每页 20 张、按目录中的采集秒和标识倒序翻页。新保存素材后清除筛选并刷新到第一页；查询变化时清除旧结果和旧图片，迟到响应不恢复旧选择。目录不存在时显示空列表，不创建目录。

列表只读取有界元信息并核对图片文件大小，展示不等于图片已通过完整校验。打开某项时重新核对元信息指纹、目录标识、PNG 大小、SHA-256 和实际 1080p 解码结果，变化、缺失或损坏时提示刷新。前端仅提交受限目录标识与元信息指纹，不能传入文件路径。

保留原有 schema 1 素材格式，不增加引擎协议或执行资源要求。单份元信息最多 16 KiB、图片最多 10 MiB，每次最多扫描 4096 个目录项，超过上限明确报错。拒绝目录联接和符号链接；损坏或不兼容项计数提示并保留原文，临时目录不进入列表。人工状态不代表识别正确、奖励已领取或实机验收通过。

## 验证

新增 8 项 Rust 回归，连同既有采集 13 项与日志 6 项共 27 项通过。覆盖缓存丢失后的只读读取、原生异步入口、空目录不创建文件、筛选和路径限制、新保存后的游标稳定性、不兼容与缺失项、元信息变化、同大小图片替换、重算哈希后的错误 PNG / 尺寸、扫描上限、目录联接和原文件保留。真实日常留档转 PNG 后重新回看，像素与原图一致。

新增 7 项界面回归，连同既有采集、外观和原生诊断共 35 项通过，覆盖断线与运行中查看、待保存预览独立、筛选及分页、列表与图片迟到响应、切页失效和重新读取、文件变化与解码错误后的重试、新保存清除筛选、损坏计数和备注限制。明暗主题及 960 像素宽布局已检查。

```powershell
$env:ZZZ_AUTO_TEST_ENGINE = (Resolve-Path target/engagement-validation/debug/zzz-engine.exe).Path
cargo test --manifest-path zzz-od-test/test/zzz_auto/rust/Cargo.toml --target-dir target/engagement-validation --locked --test screen_samples --test run_logs -- --test-threads=1
npm --prefix zzz-od-test/test/zzz_auto/frontend test -- screen-samples.spec.ts appearance.spec.ts desktop.spec.ts --reporter=list
npm --prefix apps/desktop run build
cargo build --locked --target-dir target/engagement-validation -p zzz-desktop -p zzz-engine --features custom-protocol
```

前端生产、配套引擎及原生 debug 桌面构建、主 workspace 和新增测试严格 Clippy、格式与差异检查通过；保留已有前端块大小提示。使用隔离数据和已有截图，没有读取真实用户素材、操作游戏或重打包便携 ZIP。真实 WebView2 与用户数据目录继续待实机验收。
