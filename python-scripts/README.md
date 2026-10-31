# Python 开发脚本

开发阶段把需要频繁调整的应用逻辑放在此目录。Python 3.11 及以上即可运行，当前脚本只使用标准库，不需要 pip 安装依赖。修改 `.py` 或配置后，重新运行测试或重新启动一次空洞探索即加载新版本，不触发 Cargo 或桌面打包。

- `apps/`：应用决策。目前的 `map_exploration.py` 包含绿点识别、分支歧义判断、导航选择、距离进度和到点决策。
- `configs/`：应用参数。绿点颜色、点间距和停滞阈值从 JSON 读取。
- `run.py`：截图检查、回放和真实游戏测试入口。
- `tests/`：脚本回归。
- `development.json`：开发版默认启用的应用和 Python 解释器。机器专用设置复制为 `development.local.json`，该文件不提交；其中 `python` 可以填写解释器绝对路径。设置 `enabled: false` 恢复原生策略。发布构建不加载开发脚本。

地图缩放、拖图扫描、目标详情核对、追踪确认、实际输入、战斗和解锁结果验证由已有 Rust 执行器完成。脚本不直接调用系统键鼠接口，也不自己创建 Maa 控制器。截图、模型和连续 W 保留在同一原生会话中。后续可继续把应用流程逐步迁入 `apps/`。

## 首次准备

在工程根目录执行一次（修改原生接口后才需要再次编译）：

```powershell
cargo build --locked -p zzz-engine
```

## 直接测试 Python

```powershell
python -B -m unittest discover -s python-scripts/tests -v
python python-scripts/run.py inspect --image crates/maa-adapter/tests/fixtures/map-exploration/minimap-ambiguous-route.png --output artifacts/python-dev/inspect.jsonl
python python-scripts/run.py replay --trace artifacts/python-dev/inspect.jsonl --check
```

`inspect` 调用已构建的引擎取得只读观察，Python 在本进程执行绿点识别和导航决策。`replay` 直接读取观察记录，不需要引擎、SDK、模型或游戏；去掉 `--check` 可对比修改脚本后的决策。支持通过 `--settings` 指定另一份应用参数。引擎不在默认目录时，`inspect` 和 `live` 可用 `--engine` 指定现成的 exe。

## 游戏实测

开发版桌面运行“空洞探索”会读取此目录的开发配置，自动接入 Python 策略；首次更新引擎后需要配套更新桌面附带的引擎，之后修改脚本无需重编桌面。每次运行以及暂停后继续时重新创建脚本进程，丢弃旧响应并加载脚本，保留追加的回放记录；运行途中不替换脚本。

也可以关闭桌面端后直接运行，不需要启动 Tauri；终端权限须与游戏匹配：

```powershell
target/debug/zzz-engine.exe windows
python python-scripts/run.py live --hwnd <十六进制游戏句柄> --data <已有账号数据目录> --duration-ms 60000 --allow-input
```

`--data` 默认是工程的 `data/`；桌面账号数据目录通常是 `$env:APPDATA/io.onedragon.zzz-auto`。使用已保存账号的战斗脚本，可通过 `--instance-id`、`--group-id` 选择账号和应用组。运行复用原生运行槽、游戏互斥、冻结战斗设置、输入恢复和运行记录。达到指定时长或按 Ctrl+C 后请求正常停止并等待松键；它们不代表区域已经收集完成。

Python 请求超过 750 毫秒、退出、输出非法 JSON、帧号不符或请求无法由当前截图核对的动作时，本次运行失败并由原生执行器收尾。停止请求在等待 Python 时仍会被检查。常规导航连续保留 W；交互、战斗、过期画面和异常时释放。

每次游戏测试的观察与决策写入 `artifacts/python-dev/<运行 ID>/trace.jsonl`，可直接交给 `replay`。运行终态及具体失败原因仍保存在原生运行记录中。开发测试会真实移动、战斗和交互；脚本回放本身不发送输入。
