//! 迷失之地同帧观察和界面分支。观察不包含执行权限或完成证明。
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LostVoidScreen {
    Unknown,
    World,
    NormalWorld,
    Handbook,
    TransportConfirm,
    EntryStandard,
    EntryPeriodic,
    Purge,
    TaskForce,
    Matrix,
    MatrixTeam,
    Deploy,
    Strategy,
    CommonChoice,
    Event,
    Gear,
    GearConfirm,
    Store,
    Lottery,
    RouteChange,
    BattleResult,
    BattleFail,
    Modal,
}
pub fn level_content(screen: LostVoidScreen) -> bool {
    matches!(
        screen,
        LostVoidScreen::Event
            | LostVoidScreen::CommonChoice
            | LostVoidScreen::Gear
            | LostVoidScreen::GearConfirm
            | LostVoidScreen::Store
            | LostVoidScreen::Lottery
            | LostVoidScreen::RouteChange
            | LostVoidScreen::BattleResult
            | LostVoidScreen::BattleFail
    )
}
/// 当前目录及留档中的明确旧条目；按钮文字和描述不能当作副本标题。
pub const HANDBOOK_MISSIONS: [&str; 7] = [
    "迷失之地",
    "溯影行动",
    "断层之谜",
    "枯萎之都",
    "旧都列车",
    "施工废墟",
    "巨厦遗骸",
];
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BountyProgress {
    pub current: u16,
    pub total: u16,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LotteryTickets {
    pub remaining: u8,
    pub total: u8,
}
pub fn parse_lottery_tickets(text: &str) -> Option<LotteryTickets> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let text = text.strip_prefix("开始")?;
    let (a, b) = text.split_once('/')?;
    if a.is_empty()
        || b.is_empty()
        || a.len() > 2
        || b.len() > 2
        || !a.bytes().chain(b.bytes()).all(|c| c.is_ascii_digit())
    {
        return None;
    }
    let (remaining, total) = (a.parse().ok()?, b.parse().ok()?);
    (remaining <= total && (1..=32).contains(&total)).then_some(LotteryTickets { remaining, total })
}
impl BountyProgress {
    pub fn completed(self) -> bool {
        self.total == 8000 && self.current == self.total
    }
}
/// 完整比例才能形成读数，不能用单个 8000 或 OCR 缺字证明完成。
pub fn parse_bounty(tokens: &[String]) -> Option<BountyProgress> {
    let mut candidates = Vec::new();
    let parse = |text: &str| {
        let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let text = text
            .strip_prefix("悬赏委托完成进度:")
            .or_else(|| text.strip_prefix("悬赏委托完成进度："))
            .unwrap_or(&text);
        let (a, b) = text.split_once('/')?;
        if a.is_empty() || a.len() > 4 || !a.bytes().all(|b| b.is_ascii_digit()) || b != "8000" {
            return None;
        }
        let current: u16 = a.parse().ok()?;
        (current <= 8000).then_some(BountyProgress {
            current,
            total: 8000,
        })
    };
    for token in tokens {
        if let Some(progress) = parse(token) {
            candidates.push(progress);
        }
    }
    if candidates.is_empty() {
        // 允许同一行被拆成“8000”“/”“8000”，但不能丢弃夹杂的未知文字。
        if let Some(p) = parse(&tokens.join("")) {
            candidates.push(p);
        }
    }
    (candidates.len() == 1).then(|| candidates[0])
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChoiceKind {
    Artifact(u8),
    Gear,
    GearBranch,
    Gain,
    GearGain,
    GearUpgrade,
}
pub fn parse_choice_title(text: &str) -> Option<ChoiceKind> {
    let t: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    Some(match t.as_str() {
        "请选择1张卡牌" | "请选择1项" | "请选择1枚鸣徽" => ChoiceKind::Artifact(1),
        "请选择2项" | "请选择2枚鸣徽" | "请选择两枚鸣徽" | "两枚鸣徽" => {
            ChoiceKind::Artifact(2)
        }
        "请选择一枚鸣徽" => ChoiceKind::Artifact(1),
        "请选择1个武备" => ChoiceKind::Gear,
        "请选择战术棱镜方案强化的方向" => ChoiceKind::GearBranch,
        "获得战利品" => ChoiceKind::Gain,
        "获得武备" => ChoiceKind::GearGain,
        "武备已升级" => ChoiceKind::GearUpgrade,
        _ => return None,
    })
}
pub fn parse_selection_count(text: &str) -> Option<(u8, u8)> {
    let text: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let text = text
        .strip_prefix("确定")?
        .replace('（', "(")
        .replace('）', ")");
    let inner = text.strip_prefix('(').and_then(|t| t.strip_suffix(')'))?;
    let (a, b) = inner.split_once('/')?;
    if !a.bytes().all(|c| c.is_ascii_digit()) || !b.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (a, b) = (a.parse().ok()?, b.parse().ok()?);
    (a <= b && (1..=4).contains(&b)).then_some((a, b))
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidObservation {
    pub screen: LostVoidScreen,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_overlay: Option<crate::lost_void_entry::EntryOverlayKind>,
    pub bounty: Option<BountyProgress>,
    pub choice: Option<ChoiceKind>,
    pub selection_count: Option<(u8, u8)>,
    pub title: Option<String>,
    pub tutorial: bool,
    pub confirm: bool,
    pub finish: bool,
    pub next: bool,
    pub handbook: Option<LostVoidHandbook>,
    pub missions: Vec<crate::lost_void::LostVoidMission>,
    pub lottery: Option<LotteryTickets>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LostVoidHandbook {
    pub combat: bool,
    pub categories: Vec<String>,
    pub selected_category: Option<String>,
    pub types: Vec<String>,
    pub target_go: bool,
}
impl Default for LostVoidObservation {
    fn default() -> Self {
        Self {
            screen: LostVoidScreen::Unknown,
            entry_overlay: None,
            bounty: None,
            choice: None,
            selection_count: None,
            title: None,
            tutorial: false,
            confirm: false,
            finish: false,
            next: false,
            handbook: None,
            missions: vec![],
            lottery: None,
        }
    }
}

pub fn templates() -> Vec<(&'static str, &'static [u8])> {
    static CACHE: OnceLock<BTreeMap<&'static str, Vec<u8>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        [
            (
                "event-option",
                include_bytes!("../../../resources/lost-void/scene/event-option-raw.png")
                    .as_slice(),
                include_bytes!("../../../resources/lost-void/scene/event-option-mask.png")
                    .as_slice(),
            ),
            (
                "event-black",
                include_bytes!("../../../resources/lost-void/scene/event-black-raw.png").as_slice(),
                include_bytes!("../../../resources/lost-void/scene/event-black-mask.png")
                    .as_slice(),
            ),
            (
                "world-tab",
                include_bytes!("../../../resources/lost-void/scene/world-tab-raw.png").as_slice(),
                include_bytes!("../../../resources/lost-void/scene/world-tab-mask.png").as_slice(),
            ),
            (
                "branch-a",
                include_bytes!("../../../resources/lost-void/scene/branch-a-raw.png").as_slice(),
                include_bytes!("../../../resources/lost-void/scene/branch-a-mask.png").as_slice(),
            ),
            (
                "branch-b",
                include_bytes!("../../../resources/lost-void/scene/branch-b-raw.png").as_slice(),
                include_bytes!("../../../resources/lost-void/scene/branch-b-mask.png").as_slice(),
            ),
            (
                "store-gold",
                include_bytes!("../../../resources/lost-void/scene/store-gold-raw.png").as_slice(),
                include_bytes!("../../../resources/lost-void/scene/store-gold-mask.png").as_slice(),
            ),
            (
                "store-blood",
                include_bytes!("../../../resources/lost-void/scene/store-blood_drop-raw.png")
                    .as_slice(),
                include_bytes!("../../../resources/lost-void/scene/store-blood_drop-mask.png")
                    .as_slice(),
            ),
            (
                "battle-menu",
                include_bytes!("../../../resources/lost-void/scene/battle-menu-raw.png").as_slice(),
                include_bytes!("../../../resources/lost-void/scene/battle-menu-mask.png")
                    .as_slice(),
            ),
            (
                "reward-evaluation",
                include_bytes!("../../../resources/lost-void/scene/challenge_reward_eval-raw.png")
                    .as_slice(),
                include_bytes!("../../../resources/lost-void/scene/challenge_reward_eval-mask.png")
                    .as_slice(),
            ),
            (
                "reward-money",
                include_bytes!("../../../resources/lost-void/scene/critical_reward_money-raw.png")
                    .as_slice(),
                include_bytes!("../../../resources/lost-void/scene/critical_reward_money-mask.png")
                    .as_slice(),
            ),
        ]
        .into_iter()
        .map(|(name, raw, mask)| {
            let mut image = image::load_from_memory(raw)
                .expect("固定迷失之地模板")
                .to_rgb8();
            let mask = image::load_from_memory(mask)
                .expect("固定迷失之地掩码")
                .to_luma8();
            assert_eq!(image.dimensions(), mask.dimensions());
            for (pixel, mask) in image.pixels_mut().zip(mask.pixels()) {
                assert!(matches!(mask[0], 0 | 255));
                if mask[0] == 0 {
                    *pixel = image::Rgb([0, 255, 0]);
                } else {
                    assert_ne!(pixel.0, [0, 255, 0]);
                }
            }
            let mut png = std::io::Cursor::new(Vec::new());
            image
                .write_to(&mut png, image::ImageFormat::Png)
                .expect("固定模板编码");
            (name, png.into_inner())
        })
        .collect()
    });
    let mut result: Vec<_> = cache
        .iter()
        .map(|(name, bytes)| (*name, bytes.as_slice()))
        .collect();
    result.push(("loading-lore-logo", crate::lost_void_loading::lore_logo()));
    result.extend(
        crate::basic_daily::templates()
            .into_iter()
            .filter(|(n, _)| {
                [
                    "back",
                    "world_function",
                    "world_preset",
                    "compendium",
                    "close",
                ]
                .contains(n)
            }),
    );
    result
}
pub fn pipeline() -> Value {
    let mut p = Value::Object(serde_json::Map::new());
    for (name, roi) in [
        ("Header", [0, 0, 1920, 150]),
        ("Footer", [0, 940, 1920, 140]),
        ("Title", [560, 130, 1250, 180]),
        ("ResultTitle", [581, 24, 773, 265]),
        ("ResultTitleLine", [680, 112, 562, 122]),
        ("ResultConfirm", [936, 855, 402, 142]),
        ("ResultFinish", [1566, 1002, 264, 50]),
        ("ResultRewards", crate::lost_void_settlement::REWARD_PANEL),
        ("FailureButtons", [400, 830, 1120, 110]),
        ("RecoveryTabs", [1420, 12, 430, 80]),
        ("RecoveryActions", [1260, 980, 595, 80]),
        ("RecoveryQuestion", [550, 350, 820, 240]),
        ("RecoveryConfirm", [1020, 584, 240, 100]),
        ("RecoveryCancel", [710, 584, 240, 100]),
        ("BountyOpen", [1181, 19, 165, 46]),
        ("BountyPanelTitle", [320, 180, 1150, 110]),
        ("BountyClaim", [1342, 316, 212, 40]),
        ("BountyRewardTitle", [400, 180, 1120, 400]),
        ("BountyRewardConfirm", [882, 708, 202, 50]),
        ("CommonTitle", [700, 130, 650, 120]),
        ("CommonConfirm", [855, 765, 310, 210]),
        ("CommonNames", [80, 370, 1831, 240]),
        ("CommonMarkers", [80, 210, 1831, 201]),
        ("LotteryStart", [804, 904, 312, 66]),
        ("GearCarry", [1732, 846, 180, 50]),
        ("GearName", [452, 459, 1052, 108]),
        ("GearHeader", [186, 28, 100, 50]),
        ("ModalConfirm", [1000, 580, 260, 140]),
        ("StoreRefresh", [1678, 1002, 152, 48]),
        ("StoreNames", [90, 360, 1820, 180]),
        ("StorePrices", [90, 735, 1830, 45]),
        ("StoreBuy", [90, 780, 1820, 60]),
        ("StoreBalance", [1630, 20, 155, 60]),
        ("StoreHealth", [1224, 20, 546, 65]),
        ("StoreRefreshCost", [1480, 998, 150, 60]),
        ("StoreSpecial", [142, 108, 347, 74]),
        ("EntryCards", [650, 220, 1130, 600]),
        ("EntryBounty", [736, 48, 402, 44]),
        ("MissionBounty", [1200, 48, 310, 44]),
        ("Modal", [560, 290, 800, 470]),
        ("StrategyNames", [160, 150, 1600, 510]),
        ("StrategyConfirm", [760, 850, 1100, 210]),
        ("PrepareTitle", [980, 190, 830, 95]),
        ("PrepareFooter", [10, 980, 1840, 90]),
        ("PrepareHeader", [200, 15, 210, 80]),
        ("PrepareNext", [1590, 985, 265, 80]),
        ("MatrixTabs", [160, 10, 810, 80]),
        ("MatrixLabels", [1010, 100, 840, 470]),
        ("MatrixAgentsHeader", [188, 96, 778, 70]),
        ("MatrixList", [185, 156, 781, 903]),
        ("MatrixMain", [1028, 162, 605, 137]),
        ("MatrixSupport", [1018, 350, 163, 159]),
        ("MatrixProperty", [1300, 400, 74, 34]),
        ("MatrixStart", [1633, 998, 178, 54]),
        ("MatrixConfirm", [920, 540, 110, 260]),
        ("Tutorial", [1400, 20, 470, 150]),
        ("Tabs", [708, 140, 928, 80]),
        ("Categories", [164, 287, 265, 583]),
        ("Types", [498, 308, 350, 576]),
        ("Go", [1312, 334, 334, 556]),
        ("Interaction", [480, 100, 960, 720]),
        ("NavFocus", [480, 100, 960, 958]),
        ("NavHint", [1454, 30, 360, 338]),
        ("LoadingNow", [1161, 724, 759, 331]),
        ("LoadingLoreTitle", [45, 205, 650, 60]),
        ("LoadingLoreBody", [45, 260, 650, 250]),
        ("Speaker", [860, 798, 200, 48]),
        ("Dialogue", [406, 878, 1060, 100]),
        ("EventOptions", [1438, 536, 476, 294]),
        ("EventPanelText", [560, 290, 800, 300]),
        ("EventChallengeButton", [882, 604, 194, 46]),
        ("DeployPartyCount", [1640, 10, 220, 90]),
    ] {
        p[format!("LostVoid.{name}")] = json!({"recognition":"OCR","model":"ppocrv5","threshold":0.6,"roi":roi,"action":"DoNothing","next":[],"on_error":[],"pre_delay":0,"post_delay":0});
    }
    // 与已固定 common_deploy.yml 同源；仍全部为只读节点。
    for (name, node) in crate::simulation_deploy_pipeline::pipeline()
        .as_object()
        .unwrap()
    {
        p[format!("LostVoid.{}", name.replace("Deploy.Area.", "Deploy"))] = node.clone();
    }
    for (name, roi, template) in [
        ("WorldTab", [0, 482, 102, 118], "world-tab"),
        ("BattleMenu", [58, 40, 84, 70], "battle-menu"),
        ("Back", [58, 10, 106, 90], "back"),
        ("WorldFunction", [1776, 28, 93, 89], "world_function"),
        ("WorldPreset", [1225, 44, 280, 58], "world_preset"),
        ("HandbookIcon", [1440, 28, 249, 90], "compendium"),
        ("EventOptionIcon", [1438, 536, 476, 294], "event-option"),
        ("EventBlackButton", [860, 585, 192, 144], "event-black"),
        ("LoadingLogo", [45, 55, 530, 150], "loading-lore-logo"),
        ("BountyPanelClose", [1526, 198, 56, 50], "close"),
        ("EntryOverlayClose", [1234, 160, 366, 190], "close"),
    ] {
        p[format!("LostVoid.{name}")] = json!({"recognition":"TemplateMatch","roi":roi,"template":[format!("lost-void-scene/{template}.png")],"threshold":0.7,"method":5,"green_mask":template != "compendium","action":"DoNothing","next":[],"on_error":[]});
    }
    p["LostVoid.SelectedTab"] = p["LostVoid.Tabs"].clone();
    p["LostVoid.EventBlackButton"]["threshold"] = json!(0.9);
    p["LostVoid.LoadingLogo"]["threshold"] = json!(0.92);
    p["LostVoid.LoadingLogo"]["green_mask"] = json!(false);
    for (name, template) in [("BranchA", "branch-a"), ("BranchB", "branch-b")] {
        p[format!("LostVoid.{name}")] = json!({"recognition":"TemplateMatch","roi":[80,210,1831,620],"template":[format!("lost-void-scene/{template}.png")],"threshold":0.9,"method":5,"green_mask":true,"action":"DoNothing","next":[],"on_error":[]});
    }
    p["LostVoid.CommonNameLine"] = p["LostVoid.CommonNames"].clone();
    p["LostVoid.MatrixUP"] = p["LostVoid.MatrixList"].clone();
    p["LostVoid.MatrixUP"]["only_rec"] = json!(true);
    p["LostVoid.MatrixUP"]["threshold"] = json!(0.8);
    p["LostVoid.MatrixUP"]["expected"] = json!(["(?i)^UP$"]);
    p["LostVoid.CommonNameLine"]["only_rec"] = json!(true);
    p["LostVoid.CommonEnabled"] = p["LostVoid.CommonConfirm"].clone();
    p["LostVoid.CommonEnabled"]["color_filter"] = json!("LostVoid.WhiteText");
    for name in [
        "StrategyConfirm",
        "PrepareNext",
        "MatrixStart",
        "RecoveryActions",
        "RecoveryConfirm",
        "BountyClaim",
        "BountyRewardConfirm",
    ] {
        p[format!("LostVoid.{name}Enabled")] = p[format!("LostVoid.{name}")].clone();
        p[format!("LostVoid.{name}Enabled")]["color_filter"] = json!("LostVoid.WhiteText");
    }
    p["LostVoid.GearEnabled"] = p["LostVoid.GearCarry"].clone();
    p["LostVoid.GearEnabled"]["color_filter"] = json!("LostVoid.WhiteText");
    for name in ["StorePrices", "StoreBuy", "StoreRefreshCost"] {
        p[format!("LostVoid.{name}")]["color_filter"] = json!("LostVoid.WhiteText");
    }
    p["LostVoid.StoreRefreshEnabled"] = p["LostVoid.StoreRefresh"].clone();
    p["LostVoid.StoreRefreshEnabled"]["color_filter"] = json!("LostVoid.WhiteText");
    for (node, base) in [
        ("StorePriceLine", "StorePrices"),
        ("StoreBuyLine", "StoreBuy"),
        ("StoreCostLine", "StoreRefreshCost"),
        ("StoreRefreshLine", "StoreRefresh"),
    ] {
        p[format!("LostVoid.{node}")] = p[format!("LostVoid.{base}")].clone();
        p[format!("LostVoid.{node}")]
            .as_object_mut()
            .expect("商店单行节点")
            .remove("color_filter");
        p[format!("LostVoid.{node}")]["only_rec"] = json!(true);
    }
    for (name, template) in [("StoreGold", "store-gold"), ("StoreBlood", "store-blood")] {
        p[format!("LostVoid.{name}")] = json!({"recognition":"TemplateMatch","roi":[1485,1000,65,58],"template":[format!("lost-void-scene/{template}.png")],"threshold":0.7,"method":5,"green_mask":true,"action":"DoNothing","next":[],"on_error":[]});
    }
    p["LostVoid.WhiteText"] = json!({"recognition":"ColorMatch","enabled":false,"action":"DoNothing","next":[],"on_error":[],"roi":[855,765,310,210],"method":4,"lower":[200,200,200],"upper":[255,255,255]});
    p["LostVoid.BountyClaimDisabled"] = p["LostVoid.BountyClaim"].clone();
    p["LostVoid.BountyClaimDisabled"]["color_filter"] = json!("LostVoid.BountyGrayText");
    p["LostVoid.BountyGrayText"] = json!({"recognition":"ColorMatch","enabled":false,"action":"DoNothing","next":[],"on_error":[],"roi":[1342,316,212,40],"method":4,"lower":[80,80,80],"upper":[180,180,180]});
    for (name, template) in [
        ("RewardEvaluation", "reward-evaluation"),
        ("RewardMoney", "reward-money"),
    ] {
        p[format!("LostVoid.{name}")] = json!({"recognition":"TemplateMatch","roi":crate::lost_void_settlement::REWARD_GRID,"template":[format!("lost-void-scene/{template}.png")],"threshold":0.7,"method":5,"green_mask":true,"action":"DoNothing","next":[],"on_error":[]});
    }
    p["LostVoid.ResultRewards"]["threshold"] = json!(0.8);
    p["LostVoid.ResultTitleLine"]["only_rec"] = json!(true);
    p["LostVoid.SelectedTab"]["color_filter"] = json!("LostVoid.TabColor");
    p["LostVoid.TabColor"] = json!({"recognition":"ColorMatch","enabled":false,"action":"DoNothing","next":[],"on_error":[],"roi":[708,140,928,80],"method":4,"lower":[200,160,0],"upper":[255,255,60]});
    p["LostVoid.CategoryColor"] = json!({"recognition":"ColorMatch","enabled":false,"action":"DoNothing","next":[],"on_error":[],"roi":[164,287,265,583],"method":4,"lower":[0,70,180],"upper":[100,220,255]});
    p
}

/// 独立画面包中的固定名册头像；复用已固定的原图和掩码，不读取用户仓库。
pub fn avatar_templates() -> Vec<(&'static str, &'static [u8])> {
    crate::random_play::templates()
        .iter()
        .filter(|(name, _)| name.starts_with("image/random-play/avatar_"))
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect()
}
