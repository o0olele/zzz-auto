//! 驱动盘拆解的冻结设置与检查点；筛选不确定时不进入拆解确认。
use crate::basic_daily::DailyProgress;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum DismantleGrade {
    B,
    #[default]
    AAndBelow,
    SAndBelow,
}
impl DismantleGrade {
    pub fn label(self) -> &'static str {
        match self {
            Self::B => "B",
            Self::AAndBelow => "A及以下",
            Self::SAndBelow => "S及以下",
        }
    }
    pub fn node(self) -> &'static str {
        match self {
            Self::B => "Disc.GradeB",
            Self::AAndBelow => "Disc.GradeA",
            Self::SAndBelow => "Disc.GradeS",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DismantleConfig {
    pub grade: DismantleGrade,
    pub include_abandoned: bool,
}
impl DismantleConfig {
    pub fn options() -> serde_json::Value {
        serde_json::json!([
            {"value":DismantleGrade::B,"label":"B"},
            {"value":DismantleGrade::AAndBelow,"label":"A及以下"},
            {"value":DismantleGrade::SAndBelow,"label":"S及以下"}
        ])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DismantleProgress {
    pub config: DismantleConfig,
    pub filter_reset: bool,
    pub grade_selected: bool,
    pub abandoned_selected: bool,
    pub filter_applied: bool,
    pub dismantle_attempted: bool,
    pub confirmation_attempted: bool,
    pub obtained: bool,
    pub empty_selection: bool,
    pub returned_storage: bool,
}
impl DismantleProgress {
    pub fn new(config: DismantleConfig) -> Self {
        Self {
            config,
            filter_reset: false,
            grade_selected: false,
            abandoned_selected: false,
            filter_applied: false,
            dismantle_attempted: false,
            confirmation_attempted: false,
            obtained: false,
            empty_selection: false,
            returned_storage: false,
        }
    }
    pub fn selection_ready(&self) -> bool {
        self.filter_reset
            && self.grade_selected
            && (!self.config.include_abandoned || self.abandoned_selected)
            && self.filter_applied
    }
    pub fn completed(&self) -> bool {
        self.returned_storage
            && self.selection_ready()
            && ((self.obtained && self.confirmation_attempted)
                || (self.empty_selection && !self.confirmation_attempted))
    }
    fn clear_selection(&mut self) {
        self.filter_reset = false;
        self.grade_selected = false;
        self.abandoned_selected = false;
        self.filter_applied = false;
        self.dismantle_attempted = false;
    }
}

pub fn resume(p: &mut DailyProgress) {
    let Some(d) = p.dismantle.as_mut() else {
        return;
    };
    // 已提交的拆解不能重复投递。提交前中断则重新打开筛选并从重置开始。
    if d.obtained || d.empty_selection {
        p.stage = if d.returned_storage {
            "Return"
        } else {
            "ReturnStorage"
        }
        .into();
    } else if d.confirmation_attempted {
        p.stage = "AfterConfirm".into();
    } else {
        d.clear_selection();
        p.stage = "Inspect".into();
    }
}
pub fn before_step(p: &mut DailyProgress) {
    if let Some(d) = p.dismantle.as_mut() {
        if p.stage == "Dismantle" {
            d.dismantle_attempted = true;
        }
        if p.stage == "ConfirmDismantle" {
            d.confirmation_attempted = true;
        }
    }
}
pub fn observe(p: &mut DailyProgress, node: &str) -> Result<(), &'static str> {
    let d = p.dismantle.as_mut().ok_or("缺少冻结的驱动盘拆解设置")?;
    let next = match (p.stage.as_str(), node) {
        ("Inspect", "World") => "OpenMenu",
        ("Inspect", "Menu") => "OpenStorage",
        ("Inspect", "Wengine") => "SelectDiscs",
        ("Inspect", "Storage") => "OpenDismantle",
        ("OpenMenu" | "OpenStorage" | "SelectDiscs" | "OpenDismantle", "Navigated") => "Inspect",
        ("Inspect", "Page" | "Empty" | "Selected") if !d.confirmation_attempted => "OpenFilter",
        ("Inspect" | "OpenFilter", "Filter") if !d.confirmation_attempted => "ResetFilter",
        ("ResetFilter", "FilterReset") => {
            d.clear_selection();
            d.filter_reset = true;
            "SelectGrade"
        }
        ("SelectGrade", "GradeSelected") if d.filter_reset => {
            d.grade_selected = true;
            if d.config.include_abandoned {
                "SelectAbandoned"
            } else {
                "ApplyFilter"
            }
        }
        ("SelectAbandoned", "AbandonedSelected") if d.filter_reset && d.grade_selected => {
            d.abandoned_selected = true;
            "ApplyFilter"
        }
        ("ApplyFilter", "FilterApplied")
            if d.filter_reset
                && d.grade_selected
                && (!d.config.include_abandoned || d.abandoned_selected) =>
        {
            d.filter_applied = true;
            "CheckSelection"
        }
        ("CheckSelection", "Empty") if d.selection_ready() && !d.confirmation_attempted => {
            d.empty_selection = true;
            p.completion_proven = true;
            "ReturnStorage"
        }
        ("CheckSelection", "Selected") if d.selection_ready() => "Dismantle",
        ("Dismantle", "Empty") if d.selection_ready() && !d.confirmation_attempted => {
            d.empty_selection = true;
            p.completion_proven = true;
            "ReturnStorage"
        }
        ("Dismantle", "OpenedConfirm") if d.selection_ready() => "CheckConfirm",
        ("CheckConfirm", "Confirm") if d.selection_ready() && d.dismantle_attempted => {
            "ConfirmDismantle"
        }
        // 手工停在确认页，或者暂停后选择已失去证明时，先关闭并重新筛选。
        ("Inspect", "Confirm") if !d.confirmation_attempted => "CancelConfirm",
        ("CancelConfirm", "Cancelled") => {
            d.clear_selection();
            "Inspect"
        }
        ("ConfirmDismantle", "Submitted") if d.selection_ready() && d.confirmation_attempted => {
            "AfterConfirm"
        }
        ("AfterConfirm", "Obtained") if d.confirmation_attempted => {
            d.obtained = true;
            p.completion_proven = true;
            "DismissObtained"
        }
        ("DismissObtained", "Dismissed") if d.obtained => "ReturnStorage",
        ("ReturnStorage", "Obtained") if d.obtained => "DismissObtained",
        ("ReturnStorage", "Storage") if d.obtained || d.empty_selection => {
            d.returned_storage = true;
            "Return"
        }
        ("Return", "World") if d.completed() => {
            p.returned_world = true;
            "Done"
        }
        _ => return Err("驱动盘拆解未取得当前步骤的画面或筛选证明"),
    };
    p.stage = next.into();
    Ok(())
}

/// 旧版组配置优先由导入器选择，此处只转换已知字段，未知字段仍在原文归档中。
pub fn import(value: &serde_json::Value) -> Result<DismantleConfig, &'static str> {
    if !value.is_object() {
        return Err("旧驱动盘拆解配置必须是对象");
    }
    let grade = match value.get("dismantle_level") {
        None => DismantleGrade::AAndBelow,
        Some(v) if v == "B" => DismantleGrade::B,
        Some(v) if v == "A及以下" => DismantleGrade::AAndBelow,
        Some(v) if v == "S及以下" => DismantleGrade::SAndBelow,
        _ => return Err("旧驱动盘拆解等级不是 B / A及以下 / S及以下"),
    };
    let include_abandoned = match value.get("dismantle_abandon") {
        None => false,
        Some(v) => v.as_bool().ok_or("旧驱动盘全选已弃置设置不是布尔值")?,
    };
    Ok(DismantleConfig {
        grade,
        include_abandoned,
    })
}

#[path = "dismantle_pipeline.rs"]
mod definition;
pub use definition::pipeline;
