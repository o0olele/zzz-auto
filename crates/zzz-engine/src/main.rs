use anyhow::{Context, Result, ensure};
use clap::{Parser, Subcommand, ValueEnum};
use maa_adapter::{
    CaptureMethod, InferenceDevice, InputMode, InputProbeAction, InputProbeConfig, NativeSession,
    RawRecognitionCancelled, RawRecognitionControl,
};
use od_core::{InputSide, MouseButton};
use std::{
    io::{BufRead, Read},
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};
use zzz_engine::runtime::{Engine, EnginePaths};

#[derive(Parser)]
#[command(version, about = "绝区零 Rust / Maa 引擎：M1 原生集成原型")]
struct Args {
    #[arg(long, default_value = ".")]
    root: PathBuf,
    #[arg(long)]
    sdk: Option<PathBuf>,
    #[arg(long)]
    resources: Option<PathBuf>,
    #[arg(long)]
    data: Option<PathBuf>,
    #[command(subcommand)]
    command: Action,
}

#[derive(Clone, Copy, ValueEnum)]
enum Capture {
    Gdi,
    PrintWindow,
    FramePool,
    Dxgi,
    ScreenDc,
}
#[derive(Clone, Copy, ValueEnum)]
enum Input {
    Foreground,
    ForegroundLegacy,
    Background,
    BackgroundPost,
    Xbox,
    Ds4,
}

#[derive(Clone, Copy, ValueEnum)]
enum ProbeAction {
    Key,
    Mouse,
    Stick,
    Trigger,
}

#[derive(Clone, Copy, ValueEnum)]
enum Side {
    Left,
    Right,
}
impl From<Side> for InputSide {
    fn from(side: Side) -> Self {
        match side {
            Side::Left => Self::Left,
            Side::Right => Self::Right,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
enum Button {
    Left,
    Right,
    Middle,
    X1,
    X2,
}
impl From<Button> for MouseButton {
    fn from(button: Button) -> Self {
        match button {
            Button::Left => Self::Left,
            Button::Right => Self::Right,
            Button::Middle => Self::Middle,
            Button::X1 => Self::X1,
            Button::X2 => Self::X2,
        }
    }
}

#[derive(Subcommand)]
enum Action {
    /// 不加载 SDK，校验迷失之地场景、普通选队、战斗和共享导航模型。
    LostVoidResourcesCheck {
        #[arg(long)]
        directory: Option<PathBuf>,
    },
    /// 校验迷失之地全画面只读识别资源。
    LostVoidSceneBundleCheck {
        #[arg(long, default_value = "resources/generated/lost-void-scene")]
        bundle: PathBuf,
    },
    /// 从留档图片识别迷失之地画面和完整悬赏进度。
    LostVoidSceneInspect {
        #[arg(long, default_value = "resources/generated/lost-void-scene")]
        bundle: PathBuf,
        #[arg(long)]
        image: PathBuf,
        /// 同时读取通用选择候选与选择状态，仍不发送输入。
        #[arg(long)]
        choices: bool,
        #[arg(long,conflicts_with_all=["choices","store","events","strategy","matrix","npc","navigation","roster"])]
        gear: bool,
        #[arg(long,conflicts_with_all=["choices","gear","events","strategy","matrix","npc","navigation","roster"])]
        store: bool,
        #[arg(long,conflicts_with_all=["choices","gear","store","strategy","matrix","npc","navigation","roster"])]
        events: bool,
        #[arg(long,conflicts_with_all=["choices","gear","store","events","matrix","npc","navigation","roster"])]
        strategy: bool,
        #[arg(long,conflicts_with_all=["choices","gear","store","events","strategy","npc","navigation","roster"])]
        matrix: bool,
        #[arg(long,conflicts_with_all=["choices","gear","store","events","strategy","matrix","navigation","roster"])]
        npc: bool,
        #[arg(long,conflicts_with_all=["choices","gear","store","events","strategy","matrix","npc","roster"])]
        navigation: bool,
        #[arg(long,conflicts_with_all=["choices","gear","store","events","strategy","matrix","npc","navigation"])]
        roster: bool,
    },
    /// 无 SDK、无游戏输入的迷失之地配置和藏品决策预览。
    LostVoidPreview {
        #[arg(long)]
        input: Option<PathBuf>,
    },
    #[command(flatten)]
    ResourceDiagnostics(ResourceDiagnostics),
    /// 按保存计划和人工资源数估算下一项，不初始化 SDK 或写入配置。
    ChargePlanPreview(ChargePlanPreviewArgs),
    /// 使用当前账号保存的战斗设置，Ctrl+C 请求停止与输入收尾。
    Combat {
        #[arg(long)]
        hwnd: String,
        #[arg(long)]
        instance_id: u32,
        #[arg(long)]
        allow_input: bool,
        #[arg(long)]
        cancel_after_ms: Option<u64>,
    },
    CombatBundleCheck {
        #[arg(long, default_value = "resources/generated/combat")]
        bundle: PathBuf,
    },
    /// 固定截图的战斗状态检查，不发送游戏输入。
    CombatInspect {
        #[arg(long, default_value = "resources/generated/combat")]
        bundle: PathBuf,
        #[arg(long)]
        image: PathBuf,
        #[arg(long)]
        dodge_only: bool,
    },
    /// 只回放调度与脚本动作选择，不调用 SDK。
    CombatReplay {
        #[arg(long)]
        scripts: PathBuf,
        #[arg(long)]
        script: String,
        #[arg(long)]
        events: PathBuf,
    },
    /// 引擎无法读取账号配置时的离线备份恢复；必须显式指定 --data。
    ConfigRecovery {
        #[command(subcommand)]
        command: ConfigRecoveryAction,
    },
    /// 完整运行资源包的本地构建、校验、安装与离线切换；不加载 SDK。
    RuntimeResources {
        #[command(subcommand)]
        command: RuntimeResourceAction,
    },
    /// 固定模型包的只读检查与原生准备；不加载 SDK 或连接游戏。
    ModelResources {
        #[command(subcommand)]
        command: ModelResourceAction,
    },
    RecognitionCompare {
        #[arg(long)]
        cases: PathBuf,
        #[arg(long)]
        fixture_root: PathBuf,
        #[arg(long)]
        runtime_lock: PathBuf,
        #[arg(long)]
        legacy: PathBuf,
        #[arg(long)]
        current: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    RecognitionCheck {
        #[arg(long)]
        cases: PathBuf,
        #[arg(long)]
        fixture_root: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        directml: Option<i32>,
        #[arg(long, default_value_t = 5)]
        warmup: usize,
        #[arg(long, default_value_t = 30)]
        samples: usize,
        #[arg(long, value_parser = clap::value_parser!(u16).range(0..=64))]
        raw_cpu_threads: Option<u16>,
    },
    Doctor,
    // 只校验资源，不加载 SDK、连接游戏或创建用户数据；供发布前及测试机检查。
    MailBundleCheck,
    MapBundleCheck {
        #[arg(long)]
        bundle: PathBuf,
    },
    MapInspect {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        image: PathBuf,
    },
    CoffeeBundleCheck {
        #[arg(long)]
        bundle: PathBuf,
    },
    /// 只校验活跃度奖励只读包，不加载 SDK 或读取用户资源选择。
    EngagementRewardBundleCheck {
        #[arg(long)]
        bundle: PathBuf,
    },
    /// 检查固定 1080p 截图，不连接游戏或写入运行记录。
    EngagementRewardInspect {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        image: PathBuf,
    },
    SuibianTempleBundleCheck {
        #[arg(long, default_value = "resources/generated/suibian-temple")]
        bundle: PathBuf,
    },
    WorldPatrolBundleCheck {
        #[arg(long, default_value = "resources/generated/world-patrol")]
        bundle: PathBuf,
    },
    /// 只读检查空洞探索截图，不向游戏发送输入。
    MapExplorationInspect {
        #[arg(long, default_value = "resources/generated/world-patrol")]
        bundle: PathBuf,
        #[arg(long)]
        image: PathBuf,
        #[arg(long, default_value = "observation", value_parser = ["observation", "cargo"])]
        target: String,
        #[arg(long)]
        python_observation: bool,
    },
    /// 开发阶段运行 Python 空洞探索决策，实际输入和战斗复用原生运行槽。
    MapExplorationDev {
        #[arg(long)]
        hwnd: String,
        #[arg(long, default_value_t = 1)]
        instance_id: u32,
        #[arg(long, default_value = "one_dragon")]
        group_id: String,
        #[arg(long)]
        allow_input: bool,
        #[arg(long, default_value_t = 60_000)]
        cancel_after_ms: u64,
    },
    BasicDailyBundleCheck {
        #[arg(long)]
        bundle: PathBuf,
    },
    CoffeeRunBundleCheck {
        #[arg(long)]
        bundle: PathBuf,
    },
    CoffeeRunInspect {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        image: PathBuf,
        #[arg(long)]
        espresso: bool,
    },
    CoffeeInspect {
        #[arg(long)]
        bundle: PathBuf,
        #[arg(long)]
        image: PathBuf,
    },
    Applications,
    Windows,
    Serve {
        #[arg(long)]
        pipe: String,
        #[arg(long)]
        parent_pid: u32,
    },
    ServeClient {
        #[arg(long)]
        pipe: String,
        #[arg(long)]
        parent_pid: u32,
    },
    // 内部恢复入口：只接受已配对桌面的释放计划，不初始化 SDK。
    RecoverInput {
        #[arg(long)]
        pipe: String,
        #[arg(long)]
        parent_pid: u32,
    },
    Probe {
        #[arg(long)]
        fixture: PathBuf,
        #[arg(long)]
        cancel_after_ms: Option<u64>,
    },
    Capture {
        #[arg(long)]
        hwnd: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, value_enum, default_value = "frame-pool")]
        method: Capture,
    },
    Recognize {
        #[arg(long)]
        fixture: PathBuf,
        #[arg(long)]
        image: PathBuf,
        #[arg(long)]
        kind: String,
        #[arg(long)]
        params: PathBuf,
        #[arg(long)]
        directml: Option<i32>,
        #[arg(long)]
        cancel_after_ms: Option<u64>,
    },
    InputProbe(InputProbeArgs),
}

#[derive(Subcommand)]
enum ModelResourceAction {
    Status {
        #[arg(long)]
        cache: Option<PathBuf>,
        #[arg(long = "model")]
        models: Vec<String>,
    },
    Prepare {
        #[arg(long)]
        cache: Option<PathBuf>,
        #[arg(long = "model")]
        models: Vec<String>,
        #[arg(long)]
        offline: bool,
        #[arg(long, default_value = "system")]
        proxy: String,
    },
}

#[derive(Subcommand)]
enum ConfigRecoveryAction {
    /// 查看配置状态和最多 50 份备份，不改写配置。
    List,
    /// 预览指定备份的恢复内容和本次确认指纹。
    Preview {
        #[arg(long)]
        id: String,
    },
    /// 复验指纹、保留损坏原文并恢复；须先退出使用该目录的桌面和引擎。
    Restore {
        #[arg(long)]
        id: String,
        #[arg(long)]
        preview_id: String,
        #[arg(long, required = true)]
        confirm: bool,
    },
}

#[derive(Subcommand)]
enum RuntimeResourceAction {
    Catalog,
    /// 根据已完整校验的 ZIP 生成单版本发布清单，不覆盖现有文件。
    ReleaseFeed {
        #[arg(long)]
        archive: PathBuf,
        #[arg(long)]
        sha256: String,
        #[arg(long)]
        url: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// 检查显式 HTTPS 发布清单及发布方提供的清单 SHA-256。
    Discover {
        #[arg(long)]
        url: String,
        #[arg(long)]
        sha256: String,
        #[arg(long, default_value = "system")]
        proxy: String,
    },
    #[command(hide = true)]
    DiscoverManaged,
    /// 桌面内部流式入口：来源经标准输入提交，EOF 或后续输入请求取消。
    #[command(hide = true)]
    DownloadManaged {
        #[arg(long)]
        output: PathBuf,
    },
    PreviewChange(ResourceChangeArgs),
    ApplyChange(ResourceChangeArgs),
    Download {
        #[arg(long)]
        url: String,
        #[arg(long)]
        bytes: u64,
        #[arg(long)]
        sha256: String,
        #[arg(long)]
        output: PathBuf,
        #[arg(long, default_value = "system")]
        proxy: String,
    },
    Pack {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        version: String,
        #[arg(long)]
        output: PathBuf,
    },
    Check {
        #[arg(long)]
        archive: PathBuf,
        #[arg(long)]
        sha256: String,
    },
    Install {
        #[arg(long)]
        archive: PathBuf,
        #[arg(long)]
        sha256: String,
    },
    Status,
    Activate {
        #[arg(long)]
        id: String,
    },
    Rollback,
}

#[derive(clap::Args)]
struct ResourceChangeArgs {
    #[arg(
        long,
        required_unless_present_any = ["rollback", "bundled"],
        conflicts_with_all = ["rollback", "bundled"]
    )]
    id: Option<String>,
    #[arg(long, required_unless_present_any = ["id", "bundled"], conflicts_with_all = ["id", "bundled"])]
    rollback: bool,
    #[arg(long, required_unless_present_any = ["id", "rollback"], conflicts_with_all = ["id", "rollback"])]
    bundled: bool,
    #[arg(long)]
    revision: String,
}

impl ResourceChangeArgs {
    fn request(self) -> od_core::runtime_resources::RuntimeResourceRequest {
        if self.bundled {
            return od_core::runtime_resources::RuntimeResourceRequest::UseBundled {
                revision: self.revision,
            };
        }
        match self.id {
            Some(id) => od_core::runtime_resources::RuntimeResourceRequest::Activate {
                id,
                revision: self.revision,
            },
            None => od_core::runtime_resources::RuntimeResourceRequest::Rollback {
                revision: self.revision,
            },
        }
    }
}

fn capture(method: Capture) -> CaptureMethod {
    match method {
        Capture::Gdi => CaptureMethod::Gdi,
        Capture::PrintWindow => CaptureMethod::PrintWindow,
        Capture::FramePool => CaptureMethod::FramePool,
        Capture::Dxgi => CaptureMethod::Dxgi,
        Capture::ScreenDc => CaptureMethod::ScreenDc,
    }
}
fn print(value: impl serde::Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

// 单独派生参数较多的子命令，避免整个 Action 的 Clap 构建耗尽 Windows 默认栈。
#[derive(clap::Args)]
struct InputProbeArgs {
    #[arg(long)]
    hwnd: String,
    #[arg(long, value_enum, default_value = "key")]
    action: ProbeAction,
    #[arg(long, required_if_eq("action", "key"), conflicts_with_all = ["button", "side", "x", "y", "value"])]
    key: Option<i32>,
    #[arg(long, value_enum, required_if_eq("action", "mouse"), conflicts_with_all = ["side", "value"])]
    button: Option<Button>,
    #[arg(long, value_enum, required_if_eq_any = [("action", "stick"), ("action", "trigger")])]
    side: Option<Side>,
    #[arg(long, allow_hyphen_values = true, required_if_eq_any = [("action", "mouse"), ("action", "stick")])]
    x: Option<i32>,
    #[arg(long, allow_hyphen_values = true, required_if_eq_any = [("action", "mouse"), ("action", "stick")])]
    y: Option<i32>,
    #[arg(long, required_if_eq("action", "trigger"), conflicts_with_all = ["x", "y"])]
    value: Option<i32>,
    #[arg(long, default_value_t = 100)]
    hold_ms: u64,
    #[arg(long, default_value_t = 3000)]
    start_delay_ms: u32,
    #[arg(long)]
    cancel_after_ms: Option<u64>,
    #[arg(long, value_enum, default_value = "foreground")]
    mode: Input,
    #[arg(long, value_enum, default_value = "frame-pool")]
    capture: Capture,
    #[arg(long)]
    allow_input: bool,
}

#[derive(Subcommand)]
enum ResourceDiagnostics {
    /// 校验培养详情选关包，不加载 SDK。
    AgentMissionBundleCheck(AgentMissionBundleArgs),
    /// 固定图识别培养详情目标，不发送输入或证明选中。
    AgentMissionInspect(AgentMissionInspectArgs),
    /// 校验培养达成只读资源，不加载 SDK。
    AgentGoalBundleCheck(AgentGoalBundleArgs),
    /// 固定图读取结算达成文字，不证明本次挑战完成。
    AgentGoalInspect(AgentGoalInspectArgs),
    /// 校验代理人培养传送资源，不加载 SDK。
    AgentTransportBundleCheck(AgentTransportBundleArgs),
    /// 固定图核对标记、实际目标与前往，不发送游戏输入。
    AgentTransportInspect(AgentTransportInspectArgs),
    /// 校验指定等级资源，不加载 SDK。
    ChargeLevelBundleCheck(LevelBundleArgs),
    /// 固定图只读识别等级入口及列表，不证明选中或消费。
    ChargeLevelInspect(LevelInspectArgs),
    /// 使用固定截图检查培养标记，不加载 SDK 或发送输入。
    AgentMarkersInspect(AgentMarkersArgs),
    /// 校验深度追猎资源，不加载 SDK。
    DeepHuntBundleCheck(HuntBundleArgs),
    /// 固定图只读识别目标、周常与深度状态，不证明消费或成功。
    DeepHuntInspect(HuntInspectArgs),
    /// 校验专业挑战资源，不加载 SDK。
    ExpertChallengeBundleCheck(ExpertBundleArgs),
    /// 固定图只读识别专业挑战与燃竭状态，不证明消费成功。
    ExpertChallengeInspect(ExpertInspectArgs),
    /// 校验区域巡防资源，不加载 SDK。
    AreaPatrolBundleCheck(PatrolBundleArgs),
    /// 固定图只读检查区域巡防详情或结算，不能证明本次成功。
    AreaPatrolInspect(PatrolInspectArgs),
    /// 校验其他训练副本的传送资源，不加载 SDK。
    ChargeTransportBundleCheck(TrainingTransportBundleArgs),
    /// 固定图识别分类、条目与入口，不提供消费或运行成功证明。
    ChargeTransportInspect(TrainingTransportInspectArgs),
    /// 仅校验体力页面导航资源，不加载 SDK 或游戏。
    ChargeNavigationBundleCheck(ChargeNavigationBundleArgs),
    /// 离线识别已知体力页面，不能作为到达证明。
    ChargeNavigationInspect(ChargeNavigationInspectArgs),
    /// 校验合成入口资源，不加载 SDK。
    EtherSynthesisEntryBundleCheck(SynthesisEntryBundleArgs),
    /// 固定图只读检查入口，不提供实际导航或消费证明。
    EtherSynthesisEntryInspect(SynthesisEntryInspectArgs),
    /// 校验电池合成资源，不加载 SDK 或发送输入。
    EtherSynthesisBundleCheck(SynthesisBundleArgs),
    /// 固定图只读检查合成页面，不证明本次兑换成功。
    EtherSynthesisInspect(SynthesisInspectArgs),
    /// 校验模拟室出战资源，不加载 SDK 或开放游戏输入。
    SimulationDeployBundleCheck {
        #[arg(long, default_value = "resources/generated/simulation-deploy")]
        bundle: PathBuf,
    },
    /// 校验模拟室结算资源，不加载 SDK。
    SimulationSettlementBundleCheck(SettlementBundleArgs),
    /// 固定图只读检查结算和奖励，不提供本次完成证明。
    SimulationSettlementInspect(SettlementInspectArgs),
    /// 校验模拟室关卡选择资源，不加载 SDK。
    SimulationMissionBundleCheck(MissionBundleArgs),
    /// 固定图只读检查关卡列表，不执行选择或证明选中。
    SimulationMissionInspect(MissionInspectArgs),
    /// 校验快捷手册传送资源，不加载 SDK。
    CompendiumBundleCheck(CompendiumBundleArgs),
    /// 固定图只读检查快捷手册或模拟室入口，不输入或记录完成次数。
    CompendiumInspect(CompendiumInspectArgs),
    /// 校验实战模拟室独立识别与卡片资源，不加载 SDK。
    SimulationBundleCheck {
        #[arg(long, default_value = "resources/generated/simulation")]
        bundle: PathBuf,
    },
    /// 固定截图只读检查，不连接游戏、调整卡片或写入次数。
    SimulationInspect {
        #[arg(long, default_value = "resources/generated/simulation")]
        bundle: PathBuf,
        #[arg(long)]
        image: PathBuf,
    },
    /// 校验副本内恢复电量共用资源，不加载 SDK。
    RestoreChargeBundleCheck {
        #[arg(long, default_value = "resources/generated/restore-charge")]
        bundle: PathBuf,
    },
    /// 检查固定恢复弹窗截图，不连接游戏或写入运行记录。
    RestoreChargeInspect {
        #[arg(long, default_value = "resources/generated/restore-charge")]
        bundle: PathBuf,
        #[arg(long)]
        image: PathBuf,
    },
}

// 子命令参数单独派生，限制大型枚举的 Clap 调试版解析栈占用。
#[derive(clap::Args)]
struct AgentTransportBundleArgs {
    #[arg(long, default_value = "resources/generated/agent-transport")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct AgentGoalBundleArgs {
    #[arg(long, default_value = "resources/generated/agent-goal")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct AgentMissionBundleArgs {
    #[arg(long, default_value = "resources/generated/agent-mission")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct AgentMissionInspectArgs {
    #[arg(long, default_value = "resources/generated/agent-mission")]
    bundle: PathBuf,
    #[arg(long)]
    mission_type: String,
    #[arg(long)]
    image: PathBuf,
}
impl AgentMissionInspectArgs {
    fn request(&self) -> Result<od_core::agent_mission::AgentMissionRequest> {
        let r = od_core::agent_mission::AgentMissionRequest {
            mission_type: self.mission_type.clone(),
            timeout_ms: 30_000,
        };
        r.validate().map_err(anyhow::Error::msg)?;
        Ok(r)
    }
}
#[derive(clap::Args)]
struct AgentGoalInspectArgs {
    #[arg(long, default_value = "resources/generated/agent-goal")]
    bundle: PathBuf,
    #[arg(long)]
    image: PathBuf,
}
#[derive(clap::Args)]
struct AgentTransportInspectArgs {
    #[arg(long, default_value = "resources/generated/agent-transport")]
    bundle: PathBuf,
    #[arg(long)]
    category: String,
    #[arg(long)]
    image: PathBuf,
}
impl AgentTransportInspectArgs {
    fn request(&self) -> Result<od_core::agent_transport::AgentTransportRequest> {
        let request = od_core::agent_transport::AgentTransportRequest {
            plan: od_core::charge_plan::ChargePlanItem {
                category_name: self.category.clone(),
                mission_type_name: "代理人方案培养".into(),
                mission_name: (self.category == "实战模拟室").then(|| "代理人方案培养".into()),
                ..Default::default()
            },
            timeout_ms: 30_000,
        };
        request.validate().map_err(anyhow::Error::msg)?;
        Ok(request)
    }
}
#[derive(clap::Args)]
struct LevelBundleArgs {
    #[arg(long, default_value = "resources/generated/charge-level")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct LevelInspectArgs {
    #[arg(long, default_value = "resources/generated/charge-level")]
    bundle: PathBuf,
    #[arg(long)]
    image: PathBuf,
}
#[derive(Clone, Copy, ValueEnum)]
enum MarkerArea {
    Training,
    Hunt,
    Simulation,
}
impl From<MarkerArea> for maa_adapter::agent_plan::AgentMarkerArea {
    fn from(area: MarkerArea) -> Self {
        match area {
            MarkerArea::Training => Self::Training,
            MarkerArea::Hunt => Self::Hunt,
            MarkerArea::Simulation => Self::Simulation,
        }
    }
}
#[derive(clap::Args)]
struct AgentMarkersArgs {
    #[arg(long)]
    image: PathBuf,
    #[arg(long, value_enum)]
    area: MarkerArea,
}
#[derive(clap::Args)]
struct PatrolBundleArgs {
    #[arg(long, default_value = "resources/generated/area-patrol")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct ExpertBundleArgs {
    #[arg(long, default_value = "resources/generated/expert-challenge")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct HuntBundleArgs {
    #[arg(long, default_value = "resources/generated/deep-hunt")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct HuntInspectArgs {
    #[arg(long, default_value = "resources/generated/deep-hunt")]
    bundle: PathBuf,
    #[arg(long)]
    mission_type: String,
    #[arg(long)]
    image: PathBuf,
}
#[derive(clap::Args)]
struct ExpertInspectArgs {
    #[arg(long, default_value = "resources/generated/expert-challenge")]
    bundle: PathBuf,
    #[arg(long)]
    image: PathBuf,
}
#[derive(clap::Args)]
struct PatrolInspectArgs {
    #[arg(long, default_value = "resources/generated/area-patrol")]
    bundle: PathBuf,
    #[arg(long)]
    image: PathBuf,
}
#[derive(clap::Args)]
struct SynthesisBundleArgs {
    #[arg(long, default_value = "resources/generated/ether-synthesis")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct ChargeNavigationBundleArgs {
    #[arg(long, default_value = "resources/generated/charge-navigation")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct TrainingTransportBundleArgs {
    #[arg(long, default_value = "resources/generated/charge-transport")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct TrainingTransportInspectArgs {
    #[arg(long, default_value = "resources/generated/charge-transport")]
    bundle: PathBuf,
    #[arg(long)]
    image: PathBuf,
    #[arg(long)]
    category: String,
    #[arg(long)]
    mission_type: String,
}
impl TrainingTransportInspectArgs {
    fn request(&self) -> Result<od_core::charge_transport::TrainingTransportRequest> {
        let request = od_core::charge_transport::TrainingTransportRequest {
            plan: od_core::charge_plan::ChargePlanItem {
                category_name: self.category.clone(),
                mission_type_name: self.mission_type.clone(),
                mission_name: None,
                ..Default::default()
            },
            timeout_ms: 30_000,
        };
        request.validate().map_err(anyhow::Error::msg)?;
        Ok(request)
    }
}
#[derive(clap::Args)]
struct ChargeNavigationInspectArgs {
    #[arg(long, default_value = "resources/generated/charge-navigation")]
    bundle: PathBuf,
    #[arg(long, default_value = "resources/generated/compendium")]
    compendium_bundle: PathBuf,
    #[arg(long, default_value = "resources/generated/ether-synthesis-entry")]
    entry_bundle: PathBuf,
    #[arg(long, default_value = "resources/generated/ether-synthesis")]
    synthesis_bundle: PathBuf,
    /// 显式增加区域详情返回识别能力，旧导航资源保持原定义。
    #[arg(long)]
    area_patrol_bundle: Option<PathBuf>,
    #[arg(long)]
    expert_challenge_bundle: Option<PathBuf>,
    #[arg(long)]
    deep_hunt_bundle: Option<PathBuf>,
    #[arg(long)]
    image: PathBuf,
}
#[derive(clap::Args)]
struct SynthesisEntryBundleArgs {
    #[arg(long, default_value = "resources/generated/ether-synthesis-entry")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct SynthesisEntryInspectArgs {
    #[arg(long, default_value = "resources/generated/ether-synthesis-entry")]
    bundle: PathBuf,
    #[arg(long, default_value = "resources/generated/compendium")]
    compendium_bundle: PathBuf,
    #[arg(long, default_value = "resources/generated/ether-synthesis")]
    synthesis_bundle: PathBuf,
    #[arg(long)]
    image: PathBuf,
}
#[derive(clap::Args)]
struct SynthesisInspectArgs {
    #[arg(long, default_value = "resources/generated/ether-synthesis")]
    bundle: PathBuf,
    #[arg(long)]
    image: PathBuf,
}
#[derive(clap::Args)]
struct SettlementBundleArgs {
    #[arg(long, default_value = "resources/generated/simulation-settlement")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct SettlementInspectArgs {
    #[arg(long, default_value = "resources/generated/simulation-settlement")]
    bundle: PathBuf,
    #[arg(long)]
    image: PathBuf,
}
#[derive(clap::Args)]
struct CompendiumBundleArgs {
    #[arg(long, default_value = "resources/generated/compendium")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct MissionBundleArgs {
    #[arg(long, default_value = "resources/generated/simulation-mission")]
    bundle: PathBuf,
}
#[derive(clap::Args)]
struct MissionInspectArgs {
    #[arg(long, default_value = "resources/generated/simulation-mission")]
    bundle: PathBuf,
    #[arg(long)]
    image: PathBuf,
    #[arg(long, default_value = "基础材料")]
    mission_type: String,
    #[arg(long, default_value = "调查专项")]
    mission_name: String,
}
#[derive(clap::Args)]
struct CompendiumInspectArgs {
    #[arg(long, default_value = "resources/generated/compendium")]
    bundle: PathBuf,
    #[arg(long)]
    image: PathBuf,
    #[arg(long, default_value = "基础材料")]
    mission_type: String,
    #[arg(long)]
    read_resources: bool,
}

#[derive(clap::Args)]
struct ChargePlanPreviewArgs {
    #[arg(long)]
    instance_id: u32,
    #[arg(long, default_value = "one_dragon")]
    group_id: String,
    #[arg(long)]
    charge: Option<u32>,
    #[arg(long)]
    backup: Option<u32>,
    #[arg(long)]
    ether: Option<u32>,
    #[arg(long)]
    double_reward_left: Option<u8>,
}

fn main() -> Result<()> {
    // 大型 CLI future 的构造也会占用栈；Windows 主线程默认栈不足。
    std::thread::Builder::new()
        .name("zzz-engine-main".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?
                .block_on(Box::pin(run()))
        })?
        .join()
        .map_err(|_| anyhow::anyhow!("引擎主线程异常退出"))?
}

async fn run() -> Result<()> {
    let args = Args::parse();
    if let Action::ConfigRecovery { command } = args.command {
        // 在资源选择、SDK、游戏控制与 Engine 初始化之前分流。
        let data = args
            .data
            .context("离线恢复必须显式提供 --data <绝对数据目录>")?;
        let recovery = od_core::config::ConfigRecovery::new(&data)?;
        return match command {
            ConfigRecoveryAction::List => print(recovery.catalog()?),
            ConfigRecoveryAction::Preview { id } => print(recovery.preview(&id)?),
            ConfigRecoveryAction::Restore {
                id,
                preview_id,
                confirm,
            } => {
                ensure!(confirm, "离线恢复须提供 --confirm");
                print(recovery.restore(&id, &preview_id)?)
            }
        };
    }
    let root = args.root.canonicalize().context("工程或程序根目录不存在")?;
    let explicit_resources = args.resources.is_some();
    let mut paths = EnginePaths {
        sdk: args.sdk.unwrap_or_else(|| root.join("sdk/maa/bin")),
        resources: args.resources.unwrap_or_else(|| root.join("resources")),
        data: args.data.unwrap_or_else(|| root.join("data")),
    };
    // 托管服务在取得所有权锁后解析；开发者显式资源路径继续用于离线检查。
    if !explicit_resources
        && !matches!(
            args.command,
            Action::RuntimeResources { .. }
                | Action::ModelResources { .. }
                | Action::Serve { .. }
                | Action::ServeClient { .. }
                | Action::RecoverInput { .. }
                | Action::Applications
                | Action::Windows
                | Action::RecognitionCompare { .. }
                | Action::Combat { .. }
                | Action::CombatReplay { .. }
                | Action::CombatBundleCheck { .. }
                | Action::CombatInspect { .. }
                | Action::MapBundleCheck { .. }
                | Action::CoffeeBundleCheck { .. }
                | Action::EngagementRewardBundleCheck { .. }
                | Action::EngagementRewardInspect { .. }
                | Action::SuibianTempleBundleCheck { .. }
                | Action::WorldPatrolBundleCheck { .. }
                | Action::MapExplorationInspect { .. }
                | Action::MapExplorationDev { .. }
                | Action::BasicDailyBundleCheck { .. }
                | Action::CoffeeRunBundleCheck { .. }
                | Action::CoffeeRunInspect { .. }
                | Action::ChargePlanPreview(_)
                | Action::LostVoidPreview { .. }
                | Action::LostVoidSceneBundleCheck { .. }
                | Action::LostVoidResourcesCheck { .. }
                | Action::LostVoidSceneInspect { .. }
                | Action::ResourceDiagnostics(ResourceDiagnostics::EtherSynthesisBundleCheck(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::EtherSynthesisInspect(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::EtherSynthesisEntryBundleCheck(
                    ..
                ))
                | Action::ResourceDiagnostics(ResourceDiagnostics::EtherSynthesisEntryInspect(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::ChargeNavigationBundleCheck(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::ChargeNavigationInspect(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::ChargeTransportBundleCheck(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::AreaPatrolBundleCheck(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::ExpertChallengeBundleCheck(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::AgentTransportBundleCheck(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::AgentGoalBundleCheck(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::AgentMissionBundleCheck(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::AgentMissionInspect(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::AgentGoalInspect(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::AgentTransportInspect(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::ChargeLevelBundleCheck(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::ChargeLevelInspect(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::AgentMarkersInspect(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::DeepHuntBundleCheck(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::DeepHuntInspect(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::ExpertChallengeInspect(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::AreaPatrolInspect(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::ChargeTransportInspect(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::RestoreChargeBundleCheck { .. })
                | Action::ResourceDiagnostics(ResourceDiagnostics::RestoreChargeInspect { .. })
                | Action::ResourceDiagnostics(ResourceDiagnostics::SimulationBundleCheck { .. })
                | Action::ResourceDiagnostics(ResourceDiagnostics::SimulationInspect { .. })
                | Action::ResourceDiagnostics(ResourceDiagnostics::CompendiumBundleCheck(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::CompendiumInspect(..))
                | Action::ResourceDiagnostics(ResourceDiagnostics::SimulationMissionBundleCheck(
                    ..
                ))
                | Action::ResourceDiagnostics(ResourceDiagnostics::SimulationMissionInspect(..))
        )
    {
        paths.resources = zzz_engine::runtime_resources::resolve(&paths.data, &paths.resources)?;
    }
    match args.command {
        Action::LostVoidResourcesCheck { ref directory } => {
            return print(zzz_engine::lost_void_resources::validate(
                directory.as_deref().unwrap_or(&paths.resources),
            )?);
        }
        Action::LostVoidSceneBundleCheck { ref bundle } => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"bundle":maa_adapter::lost_void_scene::validate_lost_void_scene_bundle(bundle)?}),
            );
        }
        Action::LostVoidSceneInspect { ref bundle, .. } => {
            maa_adapter::lost_void_scene::validate_lost_void_scene_bundle(bundle)?;
        }
        Action::LostVoidPreview { input } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&zzz_engine::lost_void::preview_file(
                    input.as_deref()
                )?)?
            );
            return Ok(());
        }
        Action::ChargePlanPreview(ChargePlanPreviewArgs {
            instance_id,
            group_id,
            charge,
            backup,
            ether,
            double_reward_left,
        }) => {
            let resources = match (charge, backup, ether) {
                (None, None, None) => None,
                (Some(charge), Some(backup), Some(ether)) => {
                    Some(od_core::charge_plan::ChargeResources {
                        charge,
                        backup,
                        ether,
                    })
                }
                _ => anyhow::bail!("人工资源估算须同时提供 --charge、--backup 和 --ether"),
            };
            let config = od_core::config::ConfigStore::new(&paths.data)?.read()?;
            return print(
                od_core::charge_plan::preview_group(
                    &config,
                    instance_id,
                    &group_id,
                    config.revision,
                    chrono::Utc::now(),
                    resources,
                    double_reward_left,
                )
                .map_err(anyhow::Error::msg)?,
            );
        }
        Action::CombatReplay {
            scripts,
            script,
            events,
        } => {
            #[derive(serde::Deserialize)]
            #[serde(deny_unknown_fields)]
            struct Frame {
                at: f64,
                #[serde(default)]
                updates: Vec<od_core::conditional::StateUpdate>,
                #[serde(default)]
                complete: bool,
                #[serde(default)]
                pause: bool,
                #[serde(default)]
                resume: bool,
            }
            ensure!(
                events.metadata()?.len() <= 8 * 1024 * 1024,
                "回放事件超过 8 MiB"
            );
            let frames: Vec<Frame> = serde_json::from_slice(&std::fs::read(events)?)?;
            ensure!(frames.len() <= 10000, "回放帧数超过上限");
            let script = od_core::conditional::Script::load(&scripts, &script)?;
            let mut scheduler = od_core::conditional::Scheduler::new(script);
            let mut output = Vec::new();
            let mut action_count = 0;
            for frame in frames {
                if frame.pause {
                    scheduler.pause();
                }
                if frame.resume {
                    scheduler.resume();
                }
                if frame.complete
                    && let Some(active) = scheduler.active()
                {
                    scheduler.complete(active.generation)?;
                }
                use od_core::conditional::Decision;
                let decision = scheduler.tick(frame.at, &frame.updates)?;
                let row = match decision {
                    Decision::Idle => serde_json::json!({"at":frame.at,"decision":"idle"}),
                    Decision::Interrupt => {
                        serde_json::json!({"at":frame.at,"decision":"interrupt"})
                    }
                    Decision::Start(ref execution) | Decision::Replace(ref execution) => {
                        action_count += execution.actions.len();
                        ensure!(
                            action_count <= 32768,
                            "回放输出动作超过 32768 条，请分段回放"
                        );
                        serde_json::json!({"at":frame.at,"decision":if matches!(decision,Decision::Replace(_)){"replace"}else{"start"},"generation":execution.generation,"scene":execution.scene,"handler":execution.label,"actions":execution.actions})
                    }
                };
                output.push(row);
            }
            return print(serde_json::json!({"accepted":false,"input_sent":false,"events":output}));
        }
        Action::CombatBundleCheck { ref bundle } => {
            let bundle =
                maa_adapter::combat_vision::CombatBundle::load(&std::path::absolute(bundle)?)?;
            return print(
                serde_json::json!({"valid":true,"accepted":false,"agents":bundle.spec.agents.len(),"templates":bundle.spec.templates.len(),"scripts":bundle.scripts,"dodge_scripts":bundle.dodge_scripts,"protocol_version":od_protocol::PROTOCOL_VERSION}),
            );
        }
        Action::CombatInspect { ref bundle, .. } => {
            maa_adapter::combat_vision::CombatBundle::load(&std::path::absolute(bundle)?)?;
        }
        Action::MapExplorationDev {
            hwnd,
            instance_id,
            group_id,
            allow_input,
            cancel_after_ms,
        } => {
            ensure!(cfg!(debug_assertions), "Python 测试仅供开发版使用");
            ensure!(allow_input, "游戏实测必须显式提供 --allow-input");
            ensure!(
                (1000..=1_800_000).contains(&cancel_after_ms),
                "测试时长须为 1 秒到 30 分钟"
            );
            ensure!(
                zzz_engine::python_script::DevelopmentConfig::read()?.is_some(),
                "请在 python-scripts/development.json 启用 Python 开发脚本"
            );
            let engine = Engine::with_resources(paths, explicit_resources)?;
            let ready = std::time::Instant::now();
            while !engine.snapshot().sdk.ready {
                ensure!(
                    ready.elapsed() < Duration::from_secs(15),
                    "{}",
                    engine.snapshot().sdk.message
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let od_protocol::Payload::AccountConfig(config) =
                engine.execute(od_protocol::Command::AccountConfig).await?
            else {
                anyhow::bail!("无法读取测试账号");
            };
            engine
                .execute(od_protocol::Command::Attach {
                    hwnd,
                    capture_method: config.general.capture_method,
                })
                .await?;
            let od_protocol::Payload::GroupPlan(plan) = engine
                .execute(od_protocol::Command::PreviewStandalone {
                    instance_id,
                    group_id: group_id.clone(),
                    app_id: "map_exploration".into(),
                })
                .await?
            else {
                anyhow::bail!("无法预览空洞探索计划");
            };
            ensure!(
                plan.executable,
                "空洞探索预检失败：{}",
                plan.blockers.join("；")
            );
            let revision = plan
                .exploration_combat
                .as_ref()
                .context("缺少冻结战斗脚本")?
                .script_revision
                .clone();
            let od_protocol::Payload::Run(run) = engine
                .execute(od_protocol::Command::StartStandaloneTrial {
                    combat_revision: Some(revision),
                    instance_id,
                    group_id,
                    app_id: "map_exploration".into(),
                    revision: config.revision,
                    game_day: plan.game_day.date,
                    allow_input: true,
                    confirm_account: true,
                })
                .await?
            else {
                anyhow::bail!("无法启动空洞探索测试");
            };
            eprintln!("Python 空洞探索测试 {} 已启动；Ctrl+C 停止并松键", run.id);
            let signal = tokio::signal::ctrl_c();
            tokio::pin!(signal);
            let started = std::time::Instant::now();
            let mut stopping = false;
            let mut previous = String::new();
            loop {
                if let Some(current) = engine.snapshot().run
                    && current.id == run.id
                {
                    if let Some(progress) = current.daily.first()
                        && progress.stage != previous
                    {
                        previous = progress.stage.clone();
                        eprintln!(
                            "{}",
                            serde_json::json!({"run_id":run.id,"stage":progress.stage,"inputs_released":progress.inputs_released})
                        );
                    }
                    if current.state.is_terminal() {
                        engine.shutdown().await?;
                        let failed = current.state == od_core::RunState::Failed;
                        print(current)?;
                        ensure!(!failed, "Python 空洞探索测试失败，具体原因见运行记录");
                        return Ok(());
                    }
                }
                if !stopping && started.elapsed().as_millis() >= u128::from(cancel_after_ms) {
                    engine
                        .execute(od_protocol::Command::Stop { run_id: run.id })
                        .await?;
                    stopping = true;
                }
                tokio::select! {
                    _=&mut signal, if !stopping => {engine.execute(od_protocol::Command::Stop {run_id:run.id}).await?;stopping=true;},
                    _=tokio::time::sleep(Duration::from_millis(100))=>{}
                }
            }
        }
        Action::Combat {
            hwnd,
            instance_id,
            allow_input,
            cancel_after_ms,
        } => {
            ensure!(allow_input, "战斗必须显式提供 --allow-input");
            let engine = Engine::with_resources(paths, explicit_resources)?;
            let ready = std::time::Instant::now();
            while !engine.snapshot().sdk.ready {
                ensure!(
                    ready.elapsed() < Duration::from_secs(15),
                    "{}",
                    engine.snapshot().sdk.message
                );
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            let od_protocol::Payload::AccountConfig(config) =
                engine.execute(od_protocol::Command::AccountConfig).await?
            else {
                anyhow::bail!("无法读取战斗账号");
            };
            engine
                .execute(od_protocol::Command::Attach {
                    hwnd,
                    capture_method: config.general.capture_method,
                })
                .await?;
            let od_protocol::Payload::CombatScripts(scripts) =
                engine.execute(od_protocol::Command::CombatScripts).await?
            else {
                anyhow::bail!("引擎未返回战斗脚本目录");
            };
            let od_protocol::Payload::Run(run) = engine
                .execute(od_protocol::Command::StartCombat {
                    instance_id,
                    revision: config.revision,
                    script_revision: scripts.revision,
                    allow_input: true,
                    confirm_account: true,
                })
                .await?
            else {
                anyhow::bail!("引擎未返回战斗运行");
            };
            let signal = tokio::signal::ctrl_c();
            tokio::pin!(signal);
            let started = std::time::Instant::now();
            let mut stopping = false;
            loop {
                if let Some(current) = engine.snapshot().run
                    && current.id == run.id
                    && current.state.is_terminal()
                {
                    engine.shutdown().await?;
                    return print(current);
                }
                if !stopping
                    && cancel_after_ms.is_some_and(|ms| started.elapsed().as_millis() >= ms as u128)
                {
                    engine
                        .execute(od_protocol::Command::Stop { run_id: run.id })
                        .await?;
                    stopping = true;
                }
                tokio::select! {_=&mut signal,if !stopping=>{engine.execute(od_protocol::Command::Stop {run_id:run.id}).await?;stopping=true;},_=tokio::time::sleep(Duration::from_millis(100))=>{}}
            }
        }
        Action::RuntimeResources { command } => {
            use zzz_engine::runtime_resources;
            return match command {
                RuntimeResourceAction::ReleaseFeed {
                    archive,
                    sha256,
                    url,
                    output,
                } => print(zzz_engine::runtime_feed::write_feed(
                    &archive, &sha256, url, &output,
                )?),
                RuntimeResourceAction::DiscoverManaged => {
                    tokio::task::spawn_blocking(zzz_engine::runtime_feed::managed).await?
                }
                RuntimeResourceAction::Discover { url, sha256, proxy } => {
                    let proxy = od_core::download_proxy::DownloadProxy::from_cli(proxy)
                        .map_err(anyhow::Error::msg)?;
                    let cancel = Arc::new(AtomicBool::new(false));
                    let signal_cancel = cancel.clone();
                    let signal = tokio::spawn(async move {
                        if tokio::signal::ctrl_c().await.is_ok() {
                            signal_cancel.store(true, std::sync::atomic::Ordering::Release);
                        }
                    });
                    let result = tokio::task::spawn_blocking(move || {
                        zzz_engine::runtime_feed::discover(
                            &od_core::runtime_feed::RuntimeFeedSource { url, sha256 },
                            proxy,
                            &cancel,
                        )
                    })
                    .await?;
                    signal.abort();
                    print(result?.0)
                }
                RuntimeResourceAction::DownloadManaged { output } => {
                    tokio::task::spawn_blocking(move || {
                        zzz_engine::runtime_download::managed(&output)
                    })
                    .await?
                }
                RuntimeResourceAction::Download {
                    url,
                    bytes,
                    sha256,
                    output,
                    proxy,
                } => {
                    let proxy = od_core::download_proxy::DownloadProxy::from_cli(proxy)
                        .map_err(anyhow::Error::msg)?;
                    let cancel = Arc::new(AtomicBool::new(false));
                    let interrupt = cancel.clone();
                    let signal = tokio::spawn(async move {
                        if tokio::signal::ctrl_c().await.is_ok() {
                            interrupt.store(true, std::sync::atomic::Ordering::Release);
                        }
                    });
                    let result = tokio::task::spawn_blocking(move || {
                        let source = zzz_engine::runtime_download::ResourceDownloadSource {
                            url,
                            bytes,
                            sha256,
                        };
                        let mut previous = ("", 0_u64);
                        zzz_engine::runtime_download::download_with_proxy(
                            &source,
                            &output,
                            proxy,
                            &cancel,
                            |progress| {
                                let megabytes = progress.completed_bytes / (1024 * 1024);
                                if previous != (progress.phase, megabytes) {
                                    eprintln!(
                                        "{}",
                                        serde_json::to_string(&progress).expect("资源进度可序列化")
                                    );
                                    previous = (progress.phase, megabytes);
                                }
                            },
                        )
                    })
                    .await;
                    signal.abort();
                    print(result??)
                }
                RuntimeResourceAction::Catalog => print(runtime_resources::catalog(&paths.data)?),
                RuntimeResourceAction::PreviewChange(change) => {
                    print(runtime_resources::preview_change(
                        &paths.data,
                        &paths.resources,
                        &change.request(),
                    )?)
                }
                RuntimeResourceAction::ApplyChange(change) => {
                    print(runtime_resources::apply_change(
                        &paths.data,
                        &paths.resources,
                        &change.request(),
                    )?)
                }
                RuntimeResourceAction::Pack {
                    source,
                    version,
                    output,
                } => print(runtime_resources::pack(&source, &version, &output)?),
                RuntimeResourceAction::Check { archive, sha256 } => {
                    print(runtime_resources::check_archive(&archive, &sha256)?)
                }
                RuntimeResourceAction::Install { archive, sha256 } => print(
                    serde_json::json!({"id": runtime_resources::install(&paths.data, &archive, &sha256)?, "activated": false}),
                ),
                RuntimeResourceAction::Status => {
                    print(runtime_resources::status(&paths.data, &paths.resources)?)
                }
                RuntimeResourceAction::Activate { id } => print(runtime_resources::activate(
                    &paths.data,
                    &paths.resources,
                    &id,
                )?),
                RuntimeResourceAction::Rollback => {
                    print(runtime_resources::rollback(&paths.data, &paths.resources)?)
                }
            };
        }
        Action::ModelResources { command } => {
            use od_platform::model_cache::{self, CacheState};
            match command {
                ModelResourceAction::Status { cache, models } => {
                    let cache = cache.unwrap_or_else(|| root.join("sdk/models"));
                    let models = model_cache::model_cache_status(&cache, &models)?;
                    return print(serde_json::json!({
                        "ready": models.iter().all(|model| model.files == CacheState::Valid),
                        "cache": cache, "models": models,
                    }));
                }
                ModelResourceAction::Prepare {
                    cache,
                    models,
                    offline,
                    proxy,
                } => {
                    let downloader = od_platform::download::ConfiguredHttpsDownloader::new(
                        od_core::download_proxy::DownloadProxy::from_cli(proxy)
                            .map_err(anyhow::Error::msg)?,
                    )?;
                    let cache = cache.unwrap_or_else(|| root.join("sdk/models"));
                    let cancel = Arc::new(AtomicBool::new(false));
                    let interrupt = cancel.clone();
                    let signal = tokio::spawn(async move {
                        if tokio::signal::ctrl_c().await.is_ok() {
                            interrupt.store(true, std::sync::atomic::Ordering::Release);
                        }
                    });
                    let output_cache = cache.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        let mut previous = (String::new(), String::new(), 0_u64);
                        model_cache::prepare_models(
                            &cache,
                            &models,
                            offline,
                            &downloader,
                            &cancel,
                            |progress| {
                                let megabytes = progress.completed_bytes / (1024 * 1024);
                                if previous.0 != progress.id
                                    || previous.1 != progress.phase
                                    || previous.2 != megabytes
                                {
                                    eprintln!(
                                        "{}",
                                        serde_json::to_string(&progress).expect("资源进度可序列化")
                                    );
                                    previous = (progress.id, progress.phase, megabytes);
                                }
                            },
                        )
                    })
                    .await;
                    signal.abort();
                    return print(
                        serde_json::json!({"ready": true, "cache": output_cache, "models": result??}),
                    );
                }
            }
        }
        Action::RecognitionCompare {
            cases,
            fixture_root,
            runtime_lock,
            legacy,
            current,
            output,
        } => {
            let report = zzz_engine::legacy_comparison::run(
                &cases,
                &fixture_root,
                &runtime_lock,
                &legacy,
                &current,
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
            print(
                serde_json::json!({"report": output, "comparisons": report.comparisons.len(), "equal": report.comparisons.iter().filter(|case| case.equal_within_tolerance).count(), "all_outputs_equal": report.all_outputs_equal, "all_human_expectations_passed": report.all_human_expectations_passed, "all_performance_targets_passed":report.all_performance_targets_passed,"acceptance": report.acceptance}),
            )?;
            ensure!(
                report.all_outputs_equal,
                "旧版输出比较未通过，请查看 {}",
                output.display()
            );
            ensure!(
                report.all_performance_targets_passed != Some(false),
                "离线识别 P95 目标未通过，请查看 {}",
                output.display()
            );
            return Ok(());
        }
        Action::MailBundleCheck => {
            let bundle =
                maa_adapter::validate_mail_bundle(&paths.resources.join("generated/mail"))?;
            return print(
                serde_json::json!({"valid":true,"bundle":bundle,"sdk_version":maa_adapter::SDK_VERSION,"engine_version":env!("CARGO_PKG_VERSION"),"protocol_version":od_protocol::PROTOCOL_VERSION}),
            );
        }
        Action::CoffeeBundleCheck { ref bundle } => {
            return print(
                serde_json::json!({"valid":true,"read_only":true,"bundle":maa_adapter::validate_coffee_bundle(bundle)?}),
            );
        }
        Action::EngagementRewardBundleCheck { ref bundle } => {
            return print(
                serde_json::json!({"valid":true,"read_only":true,"bundle":maa_adapter::validate_engagement_reward_bundle(bundle)?}),
            );
        }
        Action::EngagementRewardInspect { ref bundle, .. } => {
            maa_adapter::validate_engagement_reward_bundle(bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::SimulationSettlementBundleCheck(
            SettlementBundleArgs { ref bundle },
        )) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"completion_verified":false,
                "bundle":maa_adapter::simulation_settlement::validate_simulation_settlement_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::ChargeTransportBundleCheck(
            TrainingTransportBundleArgs { ref bundle },
        )) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"selection_verified":false,"navigation_verified":false,"completion_verified":false,
                "bundle":maa_adapter::charge_transport::validate_charge_transport_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::ChargeTransportInspect(ref args)) => {
            args.request()?;
            maa_adapter::charge_transport::validate_charge_transport_bundle(&args.bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AreaPatrolBundleCheck(
            PatrolBundleArgs { ref bundle },
        )) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,
                "completion_verified":false,"bundle":maa_adapter::area_patrol::validate_area_patrol_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AreaPatrolInspect(ref args)) => {
            maa_adapter::area_patrol::validate_area_patrol_bundle(&args.bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AgentTransportBundleCheck(
            AgentTransportBundleArgs { ref bundle },
        )) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"navigation_verified":false,"selection_verified":false,"completion_verified":false,"bundle":maa_adapter::agent_transport::validate_agent_transport_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AgentTransportInspect(ref args)) => {
            args.request()?;
            maa_adapter::agent_transport::validate_agent_transport_bundle(&args.bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AgentGoalBundleCheck(
            AgentGoalBundleArgs { ref bundle },
        )) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"completion_verified":false,
                "bundle":maa_adapter::agent_goal::validate_agent_goal_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AgentGoalInspect(ref args)) => {
            maa_adapter::agent_goal::validate_agent_goal_bundle(&args.bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AgentMissionBundleCheck(
            AgentMissionBundleArgs { ref bundle },
        )) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"selection_verified":false,"completion_verified":false,"bundle":maa_adapter::agent_mission::validate_agent_mission_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AgentMissionInspect(ref args)) => {
            args.request()?;
            maa_adapter::agent_mission::validate_agent_mission_bundle(&args.bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::ChargeLevelBundleCheck(
            LevelBundleArgs { ref bundle },
        )) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"selection_verified":false,"completion_verified":false,"bundle":maa_adapter::charge_level::validate_charge_level_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::ChargeLevelInspect(ref args)) => {
            maa_adapter::charge_level::validate_charge_level_bundle(&args.bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AgentMarkersInspect(ref args)) => {
            let area = args.area.into();
            return print(
                serde_json::json!({"accepted":false,"input_sent":false,"navigation_verified":false,"selection_verified":false,"completion_verified":false,"area":area,"sources":maa_adapter::agent_plan::sources()?,"markers":maa_adapter::agent_plan::inspect_agent_markers_file(&args.image, area)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::DeepHuntBundleCheck(HuntBundleArgs {
            ref bundle,
        })) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"completion_verified":false,"bundle":maa_adapter::deep_hunt::validate_deep_hunt_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::DeepHuntInspect(ref args)) => {
            anyhow::ensure!(
                od_core::charge_transport::canonical_name("恶名狩猎", &args.mission_type).is_some(),
                "未知的恶名狩猎目标"
            );
            maa_adapter::deep_hunt::validate_deep_hunt_bundle(&args.bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::ExpertChallengeBundleCheck(
            ExpertBundleArgs { ref bundle },
        )) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"completion_verified":false,
                "bundle":maa_adapter::expert_challenge::validate_expert_challenge_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::ExpertChallengeInspect(ref args)) => {
            maa_adapter::expert_challenge::validate_expert_challenge_bundle(&args.bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::ChargeNavigationBundleCheck(
            ChargeNavigationBundleArgs { ref bundle },
        )) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"navigation_verified":false,"completion_verified":false,
                "bundle":maa_adapter::charge_navigation::validate_charge_navigation_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::ChargeNavigationInspect(
            ChargeNavigationInspectArgs {
                ref bundle,
                ref compendium_bundle,
                ref entry_bundle,
                ref synthesis_bundle,
                ref area_patrol_bundle,
                ref expert_challenge_bundle,
                ref deep_hunt_bundle,
                ..
            },
        )) => {
            maa_adapter::charge_navigation::validate_charge_navigation_bundle(bundle)?;
            maa_adapter::compendium::validate_compendium_bundle(compendium_bundle)?;
            maa_adapter::ether_synthesis_entry::validate_ether_synthesis_entry_bundle(
                entry_bundle,
            )?;
            maa_adapter::ether_synthesis::validate_ether_synthesis_bundle(synthesis_bundle)?;
            if let Some(p) = area_patrol_bundle {
                maa_adapter::area_patrol::validate_area_patrol_bundle(p)?;
            }
            if let Some(p) = expert_challenge_bundle {
                maa_adapter::expert_challenge::validate_expert_challenge_bundle(p)?;
            }
            if let Some(p) = deep_hunt_bundle {
                maa_adapter::deep_hunt::validate_deep_hunt_bundle(p)?;
            }
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::EtherSynthesisEntryBundleCheck(
            SynthesisEntryBundleArgs { ref bundle },
        )) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"navigation_verified":false,
                "bundle":maa_adapter::ether_synthesis_entry::validate_ether_synthesis_entry_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::EtherSynthesisEntryInspect(
            SynthesisEntryInspectArgs {
                ref bundle,
                ref compendium_bundle,
                ref synthesis_bundle,
                ..
            },
        )) => {
            maa_adapter::ether_synthesis_entry::validate_ether_synthesis_entry_bundle(bundle)?;
            maa_adapter::compendium::validate_compendium_bundle(compendium_bundle)?;
            maa_adapter::ether_synthesis::validate_ether_synthesis_bundle(synthesis_bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::EtherSynthesisBundleCheck(
            SynthesisBundleArgs { ref bundle },
        )) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"completion_verified":false,
                "bundle":maa_adapter::ether_synthesis::validate_ether_synthesis_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::EtherSynthesisInspect(
            SynthesisInspectArgs { ref bundle, .. },
        )) => {
            maa_adapter::ether_synthesis::validate_ether_synthesis_bundle(bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::SimulationDeployBundleCheck {
            ref bundle,
        }) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"selection_verified":false,
                "bundle":maa_adapter::simulation_deploy::validate_simulation_deploy_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::SimulationSettlementInspect(
            SettlementInspectArgs { ref bundle, .. },
        )) => {
            maa_adapter::simulation_settlement::validate_simulation_settlement_bundle(bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::SimulationMissionBundleCheck(
            MissionBundleArgs { ref bundle },
        )) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,"selection_verified":false,
                "bundle":maa_adapter::simulation_mission::validate_simulation_mission_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::SimulationMissionInspect(
            MissionInspectArgs {
                ref bundle,
                ref mission_type,
                ref mission_name,
                ..
            },
        )) => {
            od_core::simulation_mission::MissionRequest {
                mission_type: mission_type.clone(),
                mission_name: mission_name.clone(),
                timeout_ms: 30_000,
            }
            .validate()
            .map_err(anyhow::Error::msg)?;
            maa_adapter::simulation_mission::validate_simulation_mission_bundle(bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::RestoreChargeBundleCheck {
            ref bundle,
        }) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,
            "bundle":maa_adapter::restore_charge::validate_restore_charge_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::SimulationBundleCheck { ref bundle }) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,
            "bundle":maa_adapter::simulation::validate_simulation_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::CompendiumBundleCheck(
            CompendiumBundleArgs { ref bundle },
        )) => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"input_sent":false,
            "bundle":maa_adapter::compendium::validate_compendium_bundle(bundle)?}),
            );
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::CompendiumInspect(
            CompendiumInspectArgs {
                ref bundle,
                ref mission_type,
                ..
            },
        )) => {
            od_core::compendium::TransportRequest {
                mission_type: mission_type.clone(),
                timeout_ms: 30_000,
            }
            .validate()
            .map_err(anyhow::Error::msg)?;
            maa_adapter::compendium::validate_compendium_bundle(bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::SimulationInspect {
            ref bundle, ..
        }) => {
            maa_adapter::simulation::validate_simulation_bundle(bundle)?;
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::RestoreChargeInspect {
            ref bundle,
            ..
        }) => {
            maa_adapter::restore_charge::validate_restore_charge_bundle(bundle)?;
        }
        Action::SuibianTempleBundleCheck { ref bundle } => {
            let bundle = maa_adapter::suibian_temple::validate_suibian_temple_bundle(bundle)?;
            let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(
                bundle.join("suibian-temple-manifest.json"),
            )?)?;
            return print(
                serde_json::json!({"valid":true,"accepted":false,"capabilities":manifest["capabilities"],"bundle":bundle}),
            );
        }
        Action::WorldPatrolBundleCheck { ref bundle } => {
            let bundle = maa_adapter::world_patrol::validate_bundle(bundle)?;
            return print(serde_json::json!({"valid":true,"accepted":false,"bundle":bundle}));
        }
        Action::BasicDailyBundleCheck { ref bundle } => {
            return print(
                serde_json::json!({"valid":true,"accepted":false,"bundle":maa_adapter::validate_basic_daily_bundle(bundle)?}),
            );
        }
        Action::CoffeeRunBundleCheck { ref bundle } => {
            return print(
                serde_json::json!({"valid":true,"prototype_only":true,"bundle":maa_adapter::validate_coffee_run_bundle(bundle)?}),
            );
        }
        Action::CoffeeRunInspect { ref bundle, .. } => {
            maa_adapter::validate_coffee_run_bundle(bundle)?;
        }
        Action::MapBundleCheck { ref bundle } => {
            return print(
                serde_json::json!({"valid":true,"prototype_only":true,"bundle":maa_adapter::validate_map_bundle(bundle)?}),
            );
        }
        Action::MapInspect { ref bundle, .. } => {
            maa_adapter::validate_map_bundle(bundle)?;
        }
        Action::CoffeeInspect { ref bundle, .. } => {
            maa_adapter::validate_coffee_bundle(bundle)?;
        }
        Action::Applications => return print(od_core::applications()?),
        Action::Windows => return print(od_platform::game_windows()?),
        Action::Serve { pipe, parent_pid } => {
            let mut token = String::new();
            std::io::stdin().lock().take(256).read_line(&mut token)?;
            let token = token.trim();
            ensure!(
                token.len() == 64 && token.bytes().all(|b| b.is_ascii_hexdigit()),
                "配对密钥格式无效"
            );
            let engine = Engine::with_resources(paths, explicit_resources)?;
            return zzz_engine::transport::serve_managed(engine, &pipe, token, parent_pid).await;
        }
        Action::ServeClient { pipe, parent_pid } => {
            return zzz_engine::transport::serve_connected(
                paths,
                &pipe,
                parent_pid,
                explicit_resources,
            )
            .await;
        }
        Action::RecoverInput { pipe, parent_pid } => {
            return zzz_engine::transport::recover_connected(&pipe, parent_pid).await;
        }
        _ => {}
    }
    let sdk = maa_adapter::load_sdk(&paths.sdk, &paths.data.join("maa-log"))?;
    match args.command {
        Action::Doctor => print(sdk),
        Action::MapExplorationInspect {
            bundle,
            image,
            target,
            python_observation,
        } => {
            let bundle = maa_adapter::world_patrol::validate_bundle(&bundle)?;
            let mut native = NativeSession::for_fixture(&image, &bundle, InferenceDevice::Cpu)?;
            native.load_world_patrol_bundle(&bundle)?;
            let target = if target == "cargo" {
                od_core::map_exploration::ExplorationTarget::Cargo
            } else {
                od_core::map_exploration::ExplorationTarget::Observation
            };
            print(if python_observation {
                native.inspect_map_exploration_script(target)?
            } else {
                native.inspect_map_exploration(target)?
            })
        }
        Action::CombatInspect {
            bundle,
            image,
            dodge_only,
        } => {
            let bundle =
                maa_adapter::combat_vision::CombatBundle::load(&std::path::absolute(bundle)?)?;
            let native = NativeSession::for_fixture(&image, &bundle.root, InferenceDevice::Cpu)?;
            let mut recognizer = maa_adapter::combat_vision::CombatRecognizer::new(bundle);
            let frame = native
                .combat_frame(&AtomicBool::new(false))?
                .context("离线战斗截图被取消")?;
            let result = recognizer.observe(
                &native,
                &frame,
                0.0,
                &Default::default(),
                dodge_only,
                &AtomicBool::new(false),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"frame_age_ms":frame.age_ms(),"observation":result}),
            )
        }
        Action::EngagementRewardInspect { bundle, image } => {
            let native = NativeSession::for_fixture(&image, &bundle, InferenceDevice::Cpu)?;
            print(native.inspect_engagement_reward_fixture(&image)?)
        }
        Action::LostVoidSceneInspect {
            bundle,
            image,
            choices,
            gear,
            store,
            events,
            strategy,
            matrix,
            npc,
            navigation,
            roster,
        } => {
            let mut native = NativeSession::for_fixture(&image, &bundle, InferenceDevice::Cpu)?;
            if roster {
                let recognizer =
                    native.load_combat_bundle(&paths.resources.join("generated/combat"))?;
                let mut reader =
                    maa_adapter::lost_void_roster::LostVoidHudReader::new(&recognizer)?;
                let frame = native
                    .combat_frame(&AtomicBool::new(false))?
                    .context("离线 HUD 截图被取消")?;
                let observation = native.inspect_lost_void_roster(
                    &frame,
                    &mut reader,
                    &AtomicBool::new(false),
                    Duration::from_secs(120),
                )?;
                return print(
                    serde_json::json!({"accepted":false,"input_sent":false,"observation":observation,"candidates":reader.diagnostics()}),
                );
            }
            if navigation {
                let mut recognizer =
                    native.load_combat_bundle(&paths.resources.join("generated/combat"))?;
                native.load_challenge_detector(&paths.resources.join("generated/basic-daily"))?;
                let frame = native
                    .combat_frame(&AtomicBool::new(false))?
                    .context("导航留档读取被取消")?;
                let observation = native.inspect_lost_void_navigation(
                    &frame,
                    &mut recognizer,
                    &AtomicBool::new(false),
                    Duration::from_secs(120),
                )?;
                return print(
                    serde_json::json!({"accepted":false,"input_sent":false,"observation":observation}),
                );
            }
            if npc {
                native.load_challenge_detector(&paths.resources.join("generated/basic-daily"))?;
                let observation = native.inspect_lost_void_npc(
                    &maa_adapter::RecognitionImage::load(&image)?,
                    true,
                    &AtomicBool::new(false),
                    Duration::from_secs(120),
                )?;
                return print(
                    serde_json::json!({"accepted":false,"input_sent":false,"observation":observation}),
                );
            }
            if matrix {
                let request = od_core::lost_void_matrix::LostVoidMatrixRequest {
                    run_id: uuid::Uuid::new_v4(),
                    challenge: Default::default(),
                    teams: Default::default(),
                    timeout_ms: 120_000,
                };
                let observation = native.inspect_lost_void_matrix(
                    &maa_adapter::RecognitionImage::load(&image)?,
                    request,
                    &AtomicBool::new(false),
                    Duration::from_secs(120),
                )?;
                return print(
                    serde_json::json!({"accepted":false,"input_sent":false,"observation":observation}),
                );
            }
            if strategy {
                let observation = native.inspect_lost_void_strategy(
                    &maa_adapter::RecognitionImage::load(&image)?,
                    &AtomicBool::new(false),
                    Duration::from_secs(120),
                )?;
                return print(
                    serde_json::json!({"accepted":false,"input_sent":false,"observation":observation}),
                );
            }
            if events {
                let observation = native.inspect_lost_void_event(
                    &maa_adapter::RecognitionImage::load(&image)?,
                    &AtomicBool::new(false),
                    Duration::from_secs(30),
                )?;
                return print(
                    serde_json::json!({"accepted":false,"input_sent":false,"observation":observation}),
                );
            }
            if gear {
                let observation = native.inspect_lost_void_gear(
                    &maa_adapter::RecognitionImage::load(&image)?,
                    &AtomicBool::new(false),
                    Duration::from_secs(30),
                )?;
                return print(
                    serde_json::json!({"accepted":false,"input_sent":false,"observation":observation}),
                );
            }
            if store {
                let observation = native.inspect_lost_void_store(
                    &maa_adapter::RecognitionImage::load(&image)?,
                    &AtomicBool::new(false),
                    Duration::from_secs(30),
                )?;
                return print(
                    serde_json::json!({"accepted":false,"input_sent":false,"observation":observation}),
                );
            }
            if choices {
                let observation = native.inspect_lost_void_choice(
                    &maa_adapter::RecognitionImage::load(&image)?,
                    &AtomicBool::new(false),
                    Duration::from_secs(30),
                )?;
                return print(
                    serde_json::json!({"accepted":false,"input_sent":false,"observation":observation}),
                );
            }
            let observation = native.inspect_lost_void_scene(
                &maa_adapter::RecognitionImage::load(&image)?,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::ChargeTransportInspect(args)) => {
            let request = args.request()?;
            let native =
                NativeSession::for_fixture(&args.image, &args.bundle, InferenceDevice::Cpu)?;
            let observation = native.inspect_charge_transport(
                &maa_adapter::RecognitionImage::load(&args.image)?,
                &request,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"navigation_verified":false,"selection_verified":false,"completion_verified":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AreaPatrolInspect(args)) => {
            let native =
                NativeSession::for_fixture(&args.image, &args.bundle, InferenceDevice::Cpu)?;
            let observation = native.inspect_area_patrol(
                &maa_adapter::RecognitionImage::load(&args.image)?,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"completion_verified":false,
                "observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AgentTransportInspect(args)) => {
            let request = args.request()?;
            let native =
                NativeSession::for_fixture(&args.image, &args.bundle, InferenceDevice::Cpu)?;
            let observation = native.inspect_agent_transport(
                &maa_adapter::RecognitionImage::load(&args.image)?,
                &request,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"navigation_verified":false,"selection_verified":false,"completion_verified":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AgentTransportBundleCheck(..)) => {
            unreachable!("只读校验已完成")
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AgentGoalBundleCheck(..)) => {
            unreachable!("只读校验已完成")
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AgentMissionBundleCheck(..)) => {
            unreachable!("只读校验已完成")
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AgentMissionInspect(args)) => {
            let native =
                NativeSession::for_fixture(&args.image, &args.bundle, InferenceDevice::Cpu)?;
            let observation = native.inspect_agent_mission(
                &maa_adapter::RecognitionImage::load(&args.image)?,
                &args.request()?,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"selection_verified":false,"completion_verified":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::AgentGoalInspect(args)) => {
            let native =
                NativeSession::for_fixture(&args.image, &args.bundle, InferenceDevice::Cpu)?;
            let observation = native.inspect_agent_goal(
                &maa_adapter::RecognitionImage::load(&args.image)?,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"completion_verified":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::ChargeLevelInspect(args)) => {
            let mut native =
                NativeSession::for_fixture(&args.image, &args.bundle, InferenceDevice::Cpu)?;
            native.load_charge_level_bundle(&args.bundle)?;
            let observation = native.inspect_charge_level(
                &maa_adapter::RecognitionImage::load(&args.image)?,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"selection_verified":false,"completion_verified":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(
            ResourceDiagnostics::AgentMarkersInspect(..)
            | ResourceDiagnostics::ChargeLevelBundleCheck(..),
        ) => unreachable!("只读诊断已完成"),
        Action::ResourceDiagnostics(ResourceDiagnostics::DeepHuntInspect(args)) => {
            let native =
                NativeSession::for_fixture(&args.image, &args.bundle, InferenceDevice::Cpu)?;
            let observation = native.inspect_deep_hunt(
                &maa_adapter::RecognitionImage::load(&args.image)?,
                &args.mission_type,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"completion_verified":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::ExpertChallengeInspect(args)) => {
            let native =
                NativeSession::for_fixture(&args.image, &args.bundle, InferenceDevice::Cpu)?;
            let observation = native.inspect_expert_challenge(
                &maa_adapter::RecognitionImage::load(&args.image)?,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"completion_verified":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::ChargeNavigationInspect(
            ChargeNavigationInspectArgs {
                bundle,
                compendium_bundle,
                entry_bundle,
                synthesis_bundle,
                area_patrol_bundle,
                expert_challenge_bundle,
                deep_hunt_bundle,
                image,
            },
        )) => {
            let mut native =
                NativeSession::for_fixture(&image, &compendium_bundle, InferenceDevice::Cpu)?;
            native.load_compendium_bundle(&compendium_bundle)?;
            native.load_charge_navigation_bundle(&bundle)?;
            native.load_ether_synthesis_entry_bundle(&entry_bundle)?;
            native.load_ether_synthesis_bundle(&synthesis_bundle)?;
            if let Some(p) = area_patrol_bundle {
                native.load_area_patrol_bundle(&p)?;
            }
            if let Some(p) = expert_challenge_bundle {
                native.load_expert_challenge_bundle(&p)?;
            }
            if let Some(p) = deep_hunt_bundle {
                native.load_deep_hunt_bundle(&p)?;
            }
            let observation = native.inspect_charge_navigation(
                &maa_adapter::RecognitionImage::load(&image)?,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"navigation_verified":false,"completion_verified":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::EtherSynthesisEntryInspect(
            SynthesisEntryInspectArgs {
                bundle,
                compendium_bundle,
                synthesis_bundle,
                image,
            },
        )) => {
            let mut native =
                NativeSession::for_fixture(&image, &compendium_bundle, InferenceDevice::Cpu)?;
            native.load_compendium_bundle(&compendium_bundle)?;
            native.load_ether_synthesis_entry_bundle(&bundle)?;
            native.load_ether_synthesis_bundle(&synthesis_bundle)?;
            let observation = native.inspect_ether_synthesis_entry(
                &maa_adapter::RecognitionImage::load(&image)?,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"navigation_verified":false,"completion_verified":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::EtherSynthesisInspect(
            SynthesisInspectArgs { bundle, image },
        )) => {
            let native = NativeSession::for_fixture(&image, &bundle, InferenceDevice::Cpu)?;
            let observation = native.inspect_ether_synthesis(
                &maa_adapter::RecognitionImage::load(&image)?,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"completion_verified":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::SimulationSettlementInspect(
            SettlementInspectArgs { bundle, image },
        )) => {
            let native = NativeSession::for_fixture(&image, &bundle, InferenceDevice::Cpu)?;
            let observation = native.inspect_simulation_settlement(
                &maa_adapter::RecognitionImage::load(&image)?,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"completion_verified":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::SimulationMissionInspect(
            MissionInspectArgs {
                bundle,
                image,
                mission_type,
                mission_name,
            },
        )) => {
            let native = NativeSession::for_fixture(&image, &bundle, InferenceDevice::Cpu)?;
            let observation = native.inspect_simulation_mission(
                &maa_adapter::RecognitionImage::load(&image)?,
                &od_core::simulation_mission::MissionRequest {
                    mission_type,
                    mission_name,
                    timeout_ms: 30_000,
                },
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"selection_verified":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::SimulationInspect { bundle, image }) => {
            let native = NativeSession::for_fixture(&image, &bundle, InferenceDevice::Cpu)?;
            let observation = native.inspect_simulation(
                &maa_adapter::RecognitionImage::load(&image)?,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::CompendiumInspect(
            CompendiumInspectArgs {
                bundle,
                image,
                mission_type,
                read_resources,
            },
        )) => {
            let native = NativeSession::for_fixture(&image, &bundle, InferenceDevice::Cpu)?;
            if read_resources {
                return print(
                    serde_json::json!({"accepted":false,"input_sent":false,"observation":
                    native.inspect_charge_resources(&maa_adapter::RecognitionImage::load(&image)?, &AtomicBool::new(false), Duration::from_secs(30))?}),
                );
            }
            let observation = native.inspect_compendium(
                &maa_adapter::RecognitionImage::load(&image)?,
                &od_core::compendium::TransportRequest {
                    mission_type,
                    timeout_ms: 30_000,
                },
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"observation":observation}),
            )
        }
        Action::ResourceDiagnostics(ResourceDiagnostics::RestoreChargeInspect {
            bundle,
            image,
        }) => {
            let native = NativeSession::for_fixture(&image, &bundle, InferenceDevice::Cpu)?;
            let prepared = maa_adapter::RecognitionImage::load(&image)?;
            let observation = native.inspect_restore_charge(
                &prepared,
                &AtomicBool::new(false),
                Duration::from_secs(30),
            )?;
            print(
                serde_json::json!({"accepted":false,"input_sent":false,"observation":observation}),
            )
        }
        Action::CoffeeInspect { bundle, image } => {
            let native = NativeSession::for_fixture(&image, &bundle, InferenceDevice::Cpu)?;
            print(native.inspect_coffee_fixture(&image)?)
        }
        Action::CoffeeRunInspect {
            bundle,
            image,
            espresso,
        } => {
            let native = NativeSession::for_fixture(&image, &bundle, InferenceDevice::Cpu)?;
            let config = od_core::coffee_run::CoffeeConfig {
                kind: if espresso {
                    od_core::coffee_run::CoffeeKind::Espresso
                } else {
                    od_core::coffee_run::CoffeeKind::Tinman
                },
                ..Default::default()
            };
            print(native.inspect_coffee_run_fixture(&image, config)?)
        }
        Action::MapInspect { bundle, image } => {
            let native = NativeSession::for_fixture(&image, &bundle, InferenceDevice::Cpu)?;
            print(native.inspect_map_fixture(&image)?)
        }
        Action::RecognitionCheck {
            cases,
            fixture_root,
            output,
            directml,
            warmup,
            samples,
            raw_cpu_threads,
        } => {
            let report = zzz_engine::recognition::run_with_options(
                &paths.resources,
                &fixture_root,
                &cases,
                &paths.data.join("maa-log"),
                zzz_engine::recognition::CheckOptions {
                    directml,
                    warmup,
                    samples,
                    raw_cpu_threads: raw_cpu_threads.map(usize::from),
                },
            )?;
            if let Some(parent) = output
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&output, serde_json::to_vec_pretty(&report)?)?;
            print(
                serde_json::json!({"report":output,"cases":report.devices.iter().map(|device| serde_json::json!({"device":device.device,"successful":device.cases.iter().filter(|case| case.errors.is_empty()).count(),"failed":device.cases.iter().filter(|case| !case.errors.is_empty()).count(),"expectation_failed":device.cases.iter().filter(|case| case.expectation_passed == Some(false)).count(),"error":device.error})).collect::<Vec<_>>(),"device_comparisons_passed":report.comparisons.iter().filter(|case| case.equal_within_tolerance).count(),"legacy_compatible":report.legacy_compatible,"acceptance":report.acceptance}),
            )?;
            ensure!(
                report.basic_checks_passed(),
                "识别能力、预期、稳定性或设备输出对照未通过，请查看 {}",
                output.display()
            );
            Ok(())
        }
        Action::Probe {
            fixture,
            cancel_after_ms,
        } => {
            let mut native =
                NativeSession::for_fixture(&fixture, &paths.resources, InferenceDevice::Cpu)?;
            let cancel = Arc::new(AtomicBool::new(false));
            if let Some(delay) = cancel_after_ms {
                let flag = cancel.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(delay));
                    flag.store(true, std::sync::atomic::Ordering::Release);
                });
            }
            print(native.health_check(cancel, Duration::from_secs(10))?)
        }
        Action::Capture {
            hwnd,
            output,
            method,
        } => {
            use base64::Engine;
            let window = od_platform::parse_game_window(&hwnd)?;
            let native = NativeSession::for_game(
                &window,
                &paths.resources,
                capture(method),
                InputMode::Foreground,
                InferenceDevice::Cpu,
            )?;
            let screenshot = native.screenshot()?;
            std::fs::write(
                &output,
                base64::engine::general_purpose::STANDARD.decode(&screenshot.png_base64)?,
            )?;
            print(
                serde_json::json!({"path": output, "width": screenshot.width, "height": screenshot.height, "elapsed_ms": screenshot.elapsed_ms, "capture_method": screenshot.capture_method, "quality": screenshot.quality}),
            )
        }
        Action::Recognize {
            fixture,
            image,
            kind,
            params,
            directml,
            cancel_after_ms,
        } => {
            let raw = matches!(kind.as_str(), "LegacyFlashRaw" | "LegacyYoloDetect");
            ensure!(
                raw || cancel_after_ms.is_none(),
                "--cancel-after-ms 只支持原始闪光与 YOLO 推理"
            );
            let device = directml.map(InferenceDevice::DirectMl).unwrap_or_default();
            let native = NativeSession::for_fixture(&fixture, &paths.resources, device)?;
            let params = serde_json::from_slice(&std::fs::read(params)?)?;
            if !raw {
                return print(native.recognize(&image, &kind, &params)?);
            }
            let control = RawRecognitionControl::default();
            let interrupt = control.clone();
            let signal = tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok()
                    && let Err(error) = interrupt.cancel()
                {
                    eprintln!("原始识别停止请求失败：{error:#}");
                }
            });
            let timer = if cancel_after_ms == Some(0) {
                control.cancel()?;
                None
            } else {
                cancel_after_ms.map(|delay| {
                    let interrupt = control.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_millis(delay)).await;
                        if let Err(error) = interrupt.cancel() {
                            eprintln!("原始识别停止请求失败：{error:#}");
                        }
                    })
                })
            };
            let result = native.recognize_timed_cancellable(&image, &kind, &params, &control);
            signal.abort();
            if let Some(timer) = timer {
                timer.abort();
            }
            match result {
                Ok(measurement) => print(measurement.detail),
                Err(error) => {
                    if let Some(cancelled) = error.downcast_ref::<RawRecognitionCancelled>() {
                        print(serde_json::json!({"status":"cancelled", "error":cancelled}))?;
                    }
                    Err(error)
                }
            }
        }
        Action::InputProbe(InputProbeArgs {
            hwnd,
            action,
            key,
            button,
            side,
            x,
            y,
            value,
            hold_ms,
            start_delay_ms,
            cancel_after_ms,
            mode,
            capture: method,
            allow_input,
        }) => {
            ensure!(
                allow_input,
                "输入探针会操作游戏，必须显式指定 --allow-input"
            );
            ensure!((1..=2000).contains(&hold_ms), "按住时间须为 1..2000ms");
            let action = match action {
                ProbeAction::Key => InputProbeAction::Key {
                    key: key.context("缺少 --key")?,
                },
                ProbeAction::Mouse => {
                    ensure!(
                        key.is_none() && side.is_none() && value.is_none(),
                        "鼠标动作不接受按键、手柄或力度参数"
                    );
                    InputProbeAction::MouseButton {
                        button: button.context("缺少 --button")?.into(),
                        x: x.context("缺少 --x")?,
                        y: y.context("缺少 --y")?,
                    }
                }
                ProbeAction::Stick => {
                    ensure!(
                        key.is_none() && button.is_none() && value.is_none(),
                        "摇杆动作不接受按键、鼠标或力度参数"
                    );
                    InputProbeAction::Stick {
                        stick: side.context("缺少 --side")?.into(),
                        x: x.context("缺少 --x")?,
                        y: y.context("缺少 --y")?,
                    }
                }
                ProbeAction::Trigger => {
                    ensure!(
                        key.is_none() && button.is_none() && x.is_none() && y.is_none(),
                        "扳机动作不接受按键、鼠标或坐标参数"
                    );
                    InputProbeAction::Trigger {
                        trigger: side.context("缺少 --side")?.into(),
                        value: value.context("缺少 --value")?,
                    }
                }
            };
            let mode = match mode {
                Input::Foreground => InputMode::Foreground,
                Input::ForegroundLegacy => InputMode::ForegroundLegacy,
                Input::Background => InputMode::Background,
                Input::BackgroundPost => InputMode::BackgroundPost,
                Input::Xbox => InputMode::Xbox,
                Input::Ds4 => InputMode::Ds4,
            };
            let config = InputProbeConfig {
                mode,
                action,
                hold_ms: hold_ms as u32,
                start_delay_ms,
            };
            config.validate().map_err(anyhow::Error::msg)?;
            let window = od_platform::parse_game_window(&hwnd)?;
            ensure!(
                window.width == 1920 && window.height == 1080,
                "游戏客户区须为 1920×1080"
            );
            let mut native = NativeSession::for_game(
                &window,
                &paths.resources,
                capture(method),
                mode,
                InferenceDevice::Cpu,
            )?;
            let cancel = Arc::new(AtomicBool::new(false));
            let interrupt = cancel.clone();
            let signal = tokio::spawn(async move {
                if tokio::signal::ctrl_c().await.is_ok() {
                    interrupt.store(true, std::sync::atomic::Ordering::Release);
                }
            });
            let timer = cancel_after_ms.map(|delay| {
                let flag = cancel.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    flag.store(true, std::sync::atomic::Ordering::Release);
                })
            });
            let result = native.input_probe(config, cancel);
            signal.abort();
            if let Some(timer) = timer {
                timer.abort();
            }
            print(result?)
        }
        _ => unreachable!(),
    }
}
