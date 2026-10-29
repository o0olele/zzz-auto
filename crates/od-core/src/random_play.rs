//! 录像店经营：冻结入口与宣传员规则，提交营业前后采用不同的恢复路径。
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const THEMES: [&str; 16] = [
    "纪实", "怀旧", "冒险", "幻想", "喜剧", "动作", "惊悚", "悬疑", "访谈", "都市", "时尚", "灾难",
    "悲剧", "亲情", "广告", "爱情",
];
pub const RANDOM_AGENT: &str = "随机";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ShopLocation {
    #[default]
    Counter,
    Failume,
    Buyast,
}
impl ShopLocation {
    pub fn area(self) -> &'static str {
        match self {
            Self::Counter => "录像店",
            Self::Failume => "澄辉坪",
            Self::Buyast => "布亚斯特城区",
        }
    }
    pub fn point(self) -> &'static str {
        match self {
            Self::Counter => "柜台",
            _ => "录像店营业点",
        }
    }
    pub fn needs_approach(self) -> bool {
        self != Self::Failume
    }
    pub fn label(self) -> String {
        format!("{} - {}", self.area(), self.point())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RandomPlayConfig {
    pub transport_point: ShopLocation,
    pub agent_name_1: String,
    pub agent_name_2: String,
    pub turn_dx: f64,
}
impl Default for RandomPlayConfig {
    fn default() -> Self {
        Self {
            transport_point: ShopLocation::Counter,
            agent_name_1: RANDOM_AGENT.into(),
            agent_name_2: RANDOM_AGENT.into(),
            turn_dx: -5.5,
        }
    }
}
impl RandomPlayConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        for name in [&self.agent_name_1, &self.agent_name_2] {
            if name.trim() != name
                || name.is_empty()
                || name.chars().count() > 40
                || name.chars().any(char::is_control)
            {
                return Err("宣传员名称须为 1..40 个字符，不含首尾空白或控制字符");
            }
        }
        crate::navigation::TurnCalibration {
            pixels_per_degree: self.turn_dx,
        }
        .validate()
    }
    pub fn random(&self) -> bool {
        self.agent_name_1 == RANDOM_AGENT || self.agent_name_2 == RANDOM_AGENT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ShopOutcome {
    AlreadyRunning,
    Started,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RandomPlayProgress {
    pub config: RandomPlayConfig,
    pub game_day: String,
    pub promoter_ready: bool,
    pub promoter_scrolls: u8,
    pub themes: Vec<String>,
    pub theme_index: u8,
    pub theme_scrolls: u8,
    pub recommended_attempted: bool,
    pub video_ready: bool,
    pub interaction_attempted: bool,
    pub transport_retries: u8,
    pub start_attempted: bool,
    pub confirmation_attempted: bool,
    pub outcome: Option<ShopOutcome>,
}
impl RandomPlayProgress {
    pub fn new(config: RandomPlayConfig, game_day: String) -> Result<Self, &'static str> {
        config.validate()?;
        crate::redemption::validate_date(&game_day)?;
        Ok(Self {
            config,
            game_day,
            promoter_ready: false,
            promoter_scrolls: 0,
            themes: vec![],
            theme_index: 0,
            theme_scrolls: 0,
            recommended_attempted: false,
            video_ready: false,
            interaction_attempted: false,
            transport_retries: 0,
            start_attempted: false,
            confirmation_attempted: false,
            outcome: None,
        })
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.config.validate()?;
        crate::redemption::validate_date(&self.game_day)?;
        let unique: std::collections::BTreeSet<_> = self.themes.iter().collect();
        if !(self.themes.is_empty() || self.themes.len() == 3)
            || unique.len() != self.themes.len()
            || self.themes.iter().any(|t| !THEMES.contains(&t.as_str()))
            || usize::from(self.theme_index) > self.themes.len()
            || self.promoter_scrolls > 2
            || self.theme_scrolls > 16
            || self.transport_retries > 1
            || (self.video_ready && (!self.promoter_ready || self.themes.len() != 3))
            || (self.start_attempted && !self.setup_ready())
            || (self.confirmation_attempted && !self.start_attempted)
            || (self.outcome == Some(ShopOutcome::Started) && !self.confirmation_attempted)
        {
            return Err("录像店检查点缺少选择或营业提交证明");
        }
        Ok(())
    }
    pub fn setup_ready(&self) -> bool {
        self.promoter_ready && self.video_ready
    }
    /// 与旧版 YYYYMMDD 最后一位的轮换规则相同，使用冻结的游戏日。
    pub fn random_slot(&self) -> u8 {
        (self.game_day.as_bytes().last().copied().unwrap_or(b'0') - b'0') % 2 + 1
    }
    pub fn candidates(&self) -> [&str; 2] {
        if self.random_slot() == 1 {
            [&self.config.agent_name_1, &self.config.agent_name_2]
        } else {
            [&self.config.agent_name_2, &self.config.agent_name_1]
        }
    }
    pub fn current_theme(&self) -> Option<&str> {
        self.themes
            .get(usize::from(self.theme_index))
            .map(String::as_str)
    }
    pub fn set_themes(&mut self, themes: Vec<String>) -> Result<(), &'static str> {
        let mut next = self.clone();
        next.themes = themes;
        next.theme_index = 0;
        next.theme_scrolls = 0;
        next.validate()?;
        *self = next;
        Ok(())
    }
    fn clear_setup(&mut self) {
        self.promoter_ready = false;
        self.promoter_scrolls = 0;
        self.themes.clear();
        self.theme_index = 0;
        self.theme_scrolls = 0;
        self.video_ready = false;
        self.recommended_attempted = false;
        self.start_attempted = false;
    }
}

/// 未识别的主题按旧版顺序补齐；结果去重，始终返回三个有效主题。
pub fn resolve_themes(readings: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for text in readings {
        // 固定主题均为两个不同汉字；此长度下 LCS 与旧 difflib 的匹配块长度相同。
        // 使用双方总长度评分，避免一长串含主题字样的 OCR 噪声被当作主题。
        let recognized: Vec<_> = text.chars().collect();
        if recognized.is_empty() || recognized.len() > 4 {
            continue;
        }
        let best = THEMES
            .iter()
            .filter_map(|theme| {
                let mut lengths = vec![0usize; recognized.len() + 1];
                for a in theme.chars() {
                    let mut diagonal = 0;
                    for (index, b) in recognized.iter().enumerate() {
                        let previous = lengths[index + 1];
                        lengths[index + 1] = if a == *b {
                            diagonal + 1
                        } else {
                            lengths[index + 1].max(lengths[index])
                        };
                        diagonal = previous;
                    }
                }
                let score = 2.0 * lengths[recognized.len()] as f64 / (2 + recognized.len()) as f64;
                (score >= 0.6).then_some((score, theme))
            })
            .max_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(b.1)));
        if let Some((_, theme)) = best
            && !result.iter().any(|t| t == theme)
        {
            result.push((*theme).to_string());
            if result.len() == 3 {
                return result;
            }
        }
    }
    for theme in THEMES {
        if !result.iter().any(|t| t == theme) {
            result.push(theme.into());
        }
        if result.len() == 3 {
            break;
        }
    }
    result
}

pub fn import(
    value: &serde_json::Value,
    turn_dx: Option<f64>,
) -> Result<RandomPlayConfig, &'static str> {
    if !value.is_object() {
        return Err("旧录像店配置必须是对象");
    }
    let mut config = RandomPlayConfig::default();
    if let Some(value) = value.get("transport_point") {
        config.transport_point = [
            ShopLocation::Counter,
            ShopLocation::Failume,
            ShopLocation::Buyast,
        ]
        .into_iter()
        .find(|place| value.as_str() == Some(place.label().as_str()))
        .ok_or("旧录像店传送点不是三个支持的营业入口")?;
    }
    for (key, output) in [
        ("agent_name_1", &mut config.agent_name_1),
        ("agent_name_2", &mut config.agent_name_2),
    ] {
        if let Some(value) = value.get(key) {
            *output = value.as_str().ok_or("旧宣传员名称不是文本")?.to_string();
        }
    }
    if let Some(dx) = turn_dx {
        config.turn_dx = dx;
    }
    config.validate()?;
    Ok(config)
}

pub fn resume(p: &mut crate::basic_daily::DailyProgress) {
    let Some(shop) = p.random_play.as_mut() else {
        return;
    };
    if shop.outcome.is_some() {
        p.stage = "Return".into();
    } else if shop.confirmation_attempted {
        p.stage = "AfterStart".into();
    } else {
        shop.clear_setup();
        p.stage = if p.transport_attempted && !p.route_confirmed {
            "WaitArrival"
        } else {
            "Inspect"
        }
        .into();
    }
}

pub fn before_step(p: &mut crate::basic_daily::DailyProgress) {
    let Some(shop) = p.random_play.as_mut() else {
        return;
    };
    match p.stage.as_str() {
        "Interact" => shop.interaction_attempted = true,
        "TryRecommended" => shop.recommended_attempted = true,
        "StartBusiness" => shop.start_attempted = true,
        "ConfirmBusiness" => shop.confirmation_attempted = true,
        _ => {}
    }
}

/// 只在只读的入口等待失败后，允许一次重新定位；前进或营业提交不重放。
pub fn retry_entry(p: &mut crate::basic_daily::DailyProgress) -> bool {
    let Some(shop) = p.random_play.as_mut() else {
        return false;
    };
    if p.stage != "WaitInteraction"
        || shop.transport_retries != 0
        || shop.start_attempted
        || !p.inputs_released
    {
        return false;
    }
    shop.transport_retries = 1;
    p.stage = "RetryInspect".into();
    true
}

pub fn observe(p: &mut crate::basic_daily::DailyProgress, node: &str) -> Result<(), &'static str> {
    let s = p.random_play.as_mut().ok_or("缺少录像店冻结设置")?;
    s.validate()?;
    let next = match (p.stage.as_str(), node) {
        ("Inspect" | "RetryInspect" | "WaitInteraction", "Book") => "CloseBook",
        ("CloseBook", "BookClosed") => "Inspect",
        ("Inspect" | "RetryInspect" | "WaitInteraction", "Narration") => "AdvanceNarration",
        ("AdvanceNarration", "Advanced") => "WaitInteraction",
        ("Inspect" | "WaitInteraction", "Options") => "ViewBusiness",
        ("RetryInspect", "Options") => "DeclineInteraction",
        ("DeclineInteraction", "Declined") => "RetryInspect",
        ("ViewBusiness", "Viewed") => "WaitInteraction",
        ("RetryInspect", "World") => {
            p.route_confirmed = false;
            p.transport_attempted = false;
            p.approach_attempted = false;
            s.interaction_attempted = false;
            "OpenMap"
        }
        ("Inspect", "World") if !p.route_confirmed && !p.transport_attempted => "OpenMap",
        ("Inspect", "World") if !p.route_confirmed => "WaitArrival",
        ("Inspect", "World") if s.interaction_attempted => "WaitInteraction",
        ("Inspect", "World")
            if s.config.transport_point.needs_approach() && !p.approach_attempted =>
        {
            "TurnEast"
        }
        ("Inspect", "World") => "Interact",
        ("Inspect" | "OpenMap", "MapOpened") => "SelectArea",
        ("Inspect" | "SelectArea", "AreaSelected") => "SelectPoint",
        ("Inspect" | "SelectPoint", "PointSelected") if !p.transport_attempted => {
            "ConfirmTransport"
        }
        ("ConfirmTransport", "Transported") => "WaitArrival",
        ("WaitArrival", "Arrived") => {
            p.route_confirmed = true;
            if s.config.transport_point.needs_approach() {
                "TurnEast"
            } else {
                "Interact"
            }
        }
        ("TurnEast", "Aligned") if p.route_confirmed && !p.approach_attempted => "Approach",
        ("Approach", "Moved") => "Interact",
        ("Interact", "Interacted") => "WaitInteraction",
        (
            "Inspect" | "RetryInspect" | "WaitInteraction" | "AfterStart" | "CheckSetup",
            "Running",
        ) => {
            s.outcome = Some(if s.confirmation_attempted {
                ShopOutcome::Started
            } else {
                ShopOutcome::AlreadyRunning
            });
            p.completion_proven = true;
            "Return"
        }
        ("Inspect" | "RetryInspect" | "WaitInteraction" | "AfterStart", "Started") => {
            s.outcome = Some(if s.confirmation_attempted {
                ShopOutcome::Started
            } else {
                ShopOutcome::AlreadyRunning
            });
            p.completion_proven = true;
            "DismissStarted"
        }
        ("DismissStarted", "Dismissed") => "Return",
        ("Inspect" | "WaitInteraction" | "RetryInspect", "Main") if !s.promoter_ready => {
            "OpenPromoters"
        }
        ("Inspect" | "WaitInteraction" | "ReturnMain" | "RetryInspect", "Main") => {
            if !s.promoter_ready {
                "OpenPromoters"
            } else if s.themes.is_empty() {
                "ReadThemes"
            } else if s.theme_index == 3 || s.recommended_attempted {
                "CheckSetup"
            } else {
                "OpenVideos"
            }
        }
        ("Inspect", "Videos" | "Promoters" | "Filter") => "ReturnMain",
        ("OpenPromoters", "Promoters") => "ChoosePromoter",
        ("ChoosePromoter", "PromoterAlready") => {
            s.promoter_ready = true;
            "ReturnMain"
        }
        ("ChoosePromoter" | "SelectFallback", "PromoterSelected") => "ConfirmPromoter",
        ("ChoosePromoter", "PromoterMissing") => {
            if s.promoter_scrolls < 2 {
                "ScrollPromoters"
            } else {
                "SelectFallback"
            }
        }
        ("ScrollPromoters", "PromotersScrolled") if s.promoter_scrolls < 2 => {
            s.promoter_scrolls += 1;
            "ChoosePromoter"
        }
        ("ConfirmPromoter", "PromoterConfirmed") => {
            s.promoter_ready = true;
            "ReturnMain"
        }
        ("ReadThemes", "ThemesRead") if s.promoter_ready && s.themes.len() == 3 => "OpenVideos",
        ("OpenVideos", "Videos") if !s.recommended_attempted => "TryRecommended",
        ("OpenVideos", "Videos") => "OpenFilter",
        ("TryRecommended", "Recommended") => "ReturnMain",
        ("TryRecommended", "NoRecommendation") => "OpenFilter",
        ("OpenFilter", "Filter") if s.current_theme().is_some() => "SelectTheme",
        ("SelectTheme", "ThemeMissing") if s.theme_scrolls < 16 => "ScrollThemes",
        ("ScrollThemes", "ThemesScrolled") if s.theme_scrolls < 16 => {
            s.theme_scrolls += 1;
            "SelectTheme"
        }
        ("SelectTheme", "ThemeChosen") => "CloseFilter",
        ("CloseFilter", "FilterClosed") => "SelectVideo",
        ("SelectVideo", "VideoChosen") => "CheckShelf",
        ("CheckShelf", "CanShelf") => "ShelfVideo",
        ("CheckShelf", "OnShelf") | ("ShelfVideo", "ShelfApplied")
            if s.current_theme().is_some() =>
        {
            s.theme_index += 1;
            s.theme_scrolls = 0;
            if s.theme_index == 3 {
                "ReturnMain"
            } else {
                "OpenFilter"
            }
        }
        ("CheckSetup", "SetupReady") if s.promoter_ready && s.themes.len() == 3 => {
            s.video_ready = true;
            "StartBusiness"
        }
        ("CheckSetup", "SetupMissing") if s.theme_index < 3 => "OpenVideos",
        ("StartBusiness", "StartConfirm") if s.setup_ready() && s.start_attempted => {
            "ConfirmBusiness"
        }
        ("Inspect" | "RetryInspect" | "WaitInteraction", "StartConfirm")
            if !s.confirmation_attempted =>
        {
            "CancelStart"
        }
        ("CancelStart", "StartCancelled") => {
            s.clear_setup();
            "Inspect"
        }
        ("ConfirmBusiness", "StartSubmitted") if s.setup_ready() && s.confirmation_attempted => {
            "AfterStart"
        }
        ("Return", "World") if s.outcome.is_some() => {
            p.returned_world = true;
            "Done"
        }
        _ => return Err("录像店未取得当前步骤所需的画面或选择证明"),
    };
    p.stage = next.into();
    Ok(())
}

#[path = "random_play_pipeline.rs"]
mod definition;
pub use definition::{overrides, overrides_for_schema, pipeline, pipeline_for_schema};

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ShopAgent {
    pub id: String,
    pub name: String,
    pub templates: Vec<String>,
}
pub fn agents() -> Vec<ShopAgent> {
    serde_json::from_str(include_str!(
        "../../../resources/source/random-play/agents.json"
    ))
    .expect("固定录像店代理人清单无效")
}
/// 旧资源包的固定名册不能随新代理人的加入而扩大。
pub fn agents_for_schema(schema: u64) -> Vec<ShopAgent> {
    if schema >= 8 {
        agents()
    } else {
        serde_json::from_str(include_str!(
            "../../../resources/source/random-play/agents-v1.json"
        ))
        .expect("旧代理人名册无效")
    }
}
pub fn roster_sources() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../resources/daily/roster-source-lock-v2.json"
    ))
    .expect("固定名册来源无效")
}
#[path = "random_play_assets.rs"]
mod assets;
pub use assets::source_images;
pub fn templates_for_schema(schema: u64) -> &'static std::collections::BTreeMap<String, Vec<u8>> {
    if schema >= 8 {
        return templates();
    }
    static LEGACY: std::sync::OnceLock<std::collections::BTreeMap<String, Vec<u8>>> =
        std::sync::OnceLock::new();
    LEGACY.get_or_init(|| {
        let names: std::collections::BTreeSet<_> = agents_for_schema(7)
            .into_iter()
            .flat_map(|a| {
                a.templates
                    .into_iter()
                    .map(|t| format!("image/random-play/avatar_{t}.png"))
            })
            .chain(std::iter::once("image/random-play/map.png".into()))
            .collect();
        templates()
            .iter()
            .filter(|(name, _)| names.contains(*name))
            .map(|(name, bytes)| (name.clone(), bytes.clone()))
            .collect()
    })
}
pub fn templates() -> &'static std::collections::BTreeMap<String, Vec<u8>> {
    static CACHE: std::sync::OnceLock<std::collections::BTreeMap<String, Vec<u8>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        source_images()
            .into_iter()
            .map(|(name, raw, mask)| {
                let mut image = image::load_from_memory(raw)
                    .expect("固定模板 PNG 无效")
                    .to_rgb8();
                let mask = image::load_from_memory(mask)
                    .expect("固定模板掩码 PNG 无效")
                    .to_luma8();
                assert_eq!(
                    image.dimensions(),
                    mask.dimensions(),
                    "固定模板与掩码尺寸不符"
                );
                for (pixel, mask) in image.pixels_mut().zip(mask.pixels()) {
                    assert!(matches!(mask[0], 0 | 255), "固定模板掩码不是二值图");
                    if mask[0] == 0 {
                        *pixel = image::Rgb([0, 255, 0]);
                    } else {
                        assert_ne!(pixel.0, [0, 255, 0], "有效模板像素与掩码色重合");
                    }
                }
                let mut png = std::io::Cursor::new(Vec::new());
                image
                    .write_to(&mut png, image::ImageFormat::Png)
                    .expect("固定模板编码失败");
                (format!("image/random-play/{name}.png"), png.into_inner())
            })
            .collect()
    })
}
