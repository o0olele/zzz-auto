绝区零一条龙 · Rust 原型 0.1.2（Windows x64）

使用方法
1. 将整个压缩包解压到可写目录，保持全部文件和子目录完整。
2. 双击 zzz-desktop.exe 或 launch.cmd。无需安装 Python、Node.js 或 Rust。
3. 若电脑缺少 WebView2，运行 tools\MicrosoftEdgeWebview2Setup.exe；该微软安装器需要联网。安装完成后重新打开应用。
4. 打开绝区零，设置游戏客户区为 1920×1080，在控制台点击“扫描窗口”与“连接窗口”。
5. 在“原生诊断”获取截图、运行健康检查并测试停止。
6. 在“真实输入诊断”选择前台 / 后台键盘或 Xbox / DS4，填写按键与时间，勾选允许这次真实输入后启动。默认延时三秒、按住半秒；停止后等待输入释放终态。可观察游戏是否响应、是否还有残留动作。

当前范围
本包是 M0/M1 原型，可测试 SDK 加载、窗口连接、截图、通信、无输入健康检查和有时间上限的真实按键诊断。
30 个应用已登记，日常领奖、自动战斗、账号导入和插件尚未开放。本包未附带 OCR、闪光或 YOLO 模型。
桌面原型默认使用 FramePool、CPU 和前台控制器；尚未自动提升引擎权限。手柄测试另需 ViGEmBus 驱动，本包不安装驱动。
如果截图是黑色，在原生诊断选择 PrintWindow 后重新获取截图；也可尝试 DXGI / ScreenDC，但游戏必须完整可见，未最小化且未被其他窗口遮挡。选择方式后点击获取截图会自动重连控制器。
截图含疑似黑帧提示和亮度统计。加载过场可能是真实黑屏，请进入有正常画面的游戏场景再测试。截图方式偏好会保存，引擎日志记录实际使用方式及统计。
输入诊断仅支持数字按键，鼠标、摇杆与扳机未开放。前台方式可能临时接管输入；键盘用 VK（W=87），手柄用 XUSB 位掩码（方向上=1，A/Cross=4096）。结果计时反映 Maa 队列完成，不代表游戏端响应已验收。

数据与退出
portable.flag 表示便携模式，数据和运行日志保存在当前文件夹的 data\，不依赖源码目录或启动时的工作目录。
关闭主窗口会隐藏到托盘；点击托盘“停止并退出”结束引擎。
移动整个文件夹即可移动程序与数据。不要从压缩包内直接运行 exe。

排查问题
双击 check-environment.cmd 可检查 Maa 原生 SDK，结果保存为 diagnostic.log。
报告问题时可附上错误文字及 diagnostic.log；引擎日志位于 data\maa-log\。
release-manifest.json 记录运行文件的 SHA-256、SDK 和运行库版本。

运行环境
Windows 10/11 x64 和 WebView2。已附带 VC++ x64 运行 DLL。
WebView2 官方说明：https://learn.microsoft.com/microsoft-edge/webview2/concepts/distribution
VC++ 部署说明：https://learn.microsoft.com/cpp/windows/choosing-a-deployment-method
源项目与第三方许可证见 licenses\。
