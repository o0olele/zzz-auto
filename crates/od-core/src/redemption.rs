//! 全局兑换码、逐码结果与可恢复检查点。结果由画面证明，不能由按钮点击推断。
use crate::basic_daily::DailyProgress;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use ts_rs::TS;

pub const MAX_CODES: usize = 256;
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RedemptionConfig {
    pub user_codes: BTreeMap<String, String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RedemptionCode {
    pub code: String,
    pub expires_on: String,
}
pub fn validate_code(code: &str) -> Result<(), &'static str> {
    if code.is_empty()
        || code.len() > 64
        || !code
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
    {
        return Err("兑换码须为 1..64 位大写英文字母、数字、下划线或连字符");
    }
    Ok(())
}
pub fn validate_date(date: &str) -> Result<(), &'static str> {
    let parsed = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| "兑换码有效期须为 YYYY-MM-DD 的有效日期")?;
    if parsed.format("%Y-%m-%d").to_string() != date {
        return Err("兑换码有效期须为 YYYY-MM-DD 的有效日期");
    }
    Ok(())
}
pub fn samples() -> BTreeMap<String, String> {
    serde_json::from_str(include_str!(
        "../../../resources/source/redemption-codes.json"
    ))
    .expect("固定兑换码清单无效")
}
impl RedemptionConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.user_codes.len() > MAX_CODES {
            return Err("用户兑换码不能超过 256 个");
        }
        for (code, date) in &self.user_codes {
            validate_code(code)?;
            validate_date(date)?;
        }
        if self.merged().len() > MAX_CODES {
            return Err("合并后的兑换码不能超过 256 个");
        }
        Ok(())
    }
    pub fn merged(&self) -> BTreeMap<String, String> {
        let mut codes = samples();
        codes.extend(self.user_codes.clone());
        codes
    }
    pub fn pending(
        &self,
        today: &str,
        imported_used: &[String],
    ) -> Result<Vec<RedemptionCode>, &'static str> {
        self.validate()?;
        validate_date(today)?;
        validate_used(imported_used)?;
        Ok(self
            .merged()
            .into_iter()
            .filter(|(code, date)| date.as_str() >= today && !imported_used.contains(code))
            .map(|(code, expires_on)| RedemptionCode { code, expires_on })
            .collect())
    }
}
pub fn validate_used(used: &[String]) -> Result<(), &'static str> {
    if used.len() > 4096 || used.iter().collect::<BTreeSet<_>>().len() != used.len() {
        return Err("旧兑换记录超过 4096 项或存在重复");
    }
    for code in used {
        validate_code(code)?;
    }
    Ok(())
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum RedemptionOutcome {
    Redeemed,
    AlreadyRedeemed,
    Invalid,
    Expired,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RedemptionResult {
    pub code: String,
    pub outcome: RedemptionOutcome,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
pub struct RedemptionHistory {
    pub results: Vec<RedemptionResult>,
    pub unreadable: u32,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RedemptionProgress {
    pub codes: Vec<RedemptionCode>,
    pub index: usize,
    pub input_verified: bool,
    pub submitted: bool,
    pub results: Vec<RedemptionResult>,
}
impl RedemptionProgress {
    pub fn new(codes: Vec<RedemptionCode>) -> Result<Self, &'static str> {
        let value = Self {
            codes,
            ..Self::default()
        };
        value.validate()?;
        Ok(value)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.codes.len() > MAX_CODES
            || self.index > self.codes.len()
            || self.results.len() > self.codes.len()
            || !(self.results.len() == self.index || self.results.len() == self.index + 1)
            || self
                .codes
                .iter()
                .map(|c| &c.code)
                .collect::<BTreeSet<_>>()
                .len()
                != self.codes.len()
            || self
                .results
                .iter()
                .zip(&self.codes)
                .any(|(r, c)| r.code != c.code)
        {
            return Err("兑换码检查点的顺序或结果无效");
        }
        for code in &self.codes {
            validate_code(&code.code)?;
            validate_date(&code.expires_on)?;
        }
        if (self.submitted && !self.input_verified)
            || (self.results.len() > self.index && !self.submitted)
            || (self.index == self.codes.len() && self.input_verified)
        {
            return Err("兑换结果缺少本次输入和提交证明");
        }
        Ok(())
    }
    pub fn current(&self) -> Option<&RedemptionCode> {
        self.codes.get(self.index)
    }
    pub fn completed(&self) -> bool {
        !self.codes.is_empty()
            && self.index == self.codes.len()
            && self.results.len() == self.codes.len()
            && self.validate().is_ok()
    }
}
pub fn before_step(p: &mut DailyProgress) {
    if p.stage == "SubmitCode"
        && let Some(r) = p.redemption.as_mut()
    {
        r.submitted = true;
    }
}
pub fn resume(p: &mut DailyProgress) {
    let Some(r) = p.redemption.as_mut() else {
        return;
    };
    p.stage = if r.completed() {
        "Return"
    } else if r.results.len() > r.index {
        "DismissResult"
    } else if r.submitted {
        "AwaitResult"
    } else {
        r.input_verified = false;
        "Inspect"
    }
    .into();
}
pub fn observe(p: &mut DailyProgress, node: &str) -> Result<(), &'static str> {
    let r = p.redemption.as_mut().ok_or("缺少冻结的兑换码")?;
    r.validate()?;
    let stage = match (p.stage.as_str(), node) {
        ("Inspect", "World") => "OpenMenu",
        ("Inspect", "Menu") => "OpenMore",
        ("Inspect", "More") => "OpenCode",
        ("OpenMenu" | "OpenMore" | "OpenCode", "Navigated") => "Inspect",
        ("Inspect", "Input") if r.current().is_some() && !r.submitted => "FillCode",
        ("Inspect", "Result") if !r.submitted => "DismissOrphan",
        ("DismissOrphan", "Dismissed") => "Inspect",
        ("FillCode", "Filled") => "VerifyInput",
        ("VerifyInput", "Verified") => {
            r.input_verified = true;
            "SubmitCode"
        }
        ("SubmitCode", "Submitted") if r.input_verified && r.submitted => "AwaitResult",
        ("AwaitResult", "Redeemed" | "AlreadyRedeemed" | "Invalid" | "Expired")
            if r.input_verified && r.submitted && r.results.len() == r.index =>
        {
            let code = r.current().ok_or("当前兑换码不存在")?.code.clone();
            let outcome = match node {
                "Redeemed" => RedemptionOutcome::Redeemed,
                "AlreadyRedeemed" => RedemptionOutcome::AlreadyRedeemed,
                "Invalid" => RedemptionOutcome::Invalid,
                _ => RedemptionOutcome::Expired,
            };
            r.results.push(RedemptionResult { code, outcome });
            "DismissResult"
        }
        ("DismissResult", "Dismissed") if r.results.len() == r.index + 1 => {
            r.index += 1;
            r.input_verified = false;
            r.submitted = false;
            if r.completed() {
                p.completion_proven = true;
                "Return"
            } else {
                "Inspect"
            }
        }
        ("Return", "World") if r.completed() => {
            p.returned_world = true;
            "Done"
        }
        _ => return Err("兑换码未取得当前步骤的输入或结果证明"),
    };
    p.stage = stage.into();
    Ok(())
}
/// 原始记录只保留旧版“已处理”语义；并不转换为新实现的已验证成功。
pub fn import_used(value: &serde_json::Value) -> Result<Vec<String>, &'static str> {
    let mut used = BTreeSet::new();
    if !value.is_object() {
        return Err("旧兑换记录必须是对象");
    }
    if let Some(values) = value.get("used_code_list") {
        for code in values.as_array().ok_or("旧已用兑换码必须是列表")? {
            let code = code
                .as_str()
                .ok_or("旧已用兑换码必须是文字")?
                .trim()
                .to_ascii_uppercase();
            validate_code(&code)?;
            used.insert(code);
        }
    }
    let result: Vec<_> = used.into_iter().collect();
    validate_used(&result)?;
    Ok(result)
}
pub fn import_codes(value: &serde_json::Value) -> Result<BTreeMap<String, String>, &'static str> {
    if !value.is_object() {
        return Err("旧兑换码配置必须是对象");
    }
    let Some(codes) = value.get("codes") else {
        return Ok(BTreeMap::new());
    };
    let codes = codes.as_object().ok_or("旧兑换码配置的 codes 必须是对象")?;
    let mut result = BTreeMap::new();
    for (code, date) in codes {
        let code = code.trim().to_ascii_uppercase();
        validate_code(&code)?;
        let date = if let Some(s) = date.as_str() {
            s.to_owned()
        } else if let Some(n) = date.as_u64() {
            n.to_string()
        } else {
            return Err("旧兑换码有效期必须是 YYYYMMDD");
        };
        if date.len() != 8 || !date.bytes().all(|c| c.is_ascii_digit()) {
            return Err("旧兑换码有效期必须是 YYYYMMDD");
        }
        let date = format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..]);
        validate_date(&date)?;
        if result.insert(code, date).is_some() {
            return Err("旧兑换码规范化后重复");
        }
    }
    let config = RedemptionConfig { user_codes: result };
    config.validate()?;
    Ok(config.user_codes)
}

pub fn apply_history(plan: &mut crate::run_plan::GroupPlan, history: RedemptionHistory) {
    use crate::run_plan::PlanDecision;
    let processed: BTreeSet<_> = history.results.iter().map(|r| r.code.as_str()).collect();
    plan.redemption_codes
        .retain(|c| !processed.contains(c.code.as_str()));
    if let Some(item) = plan
        .items
        .iter_mut()
        .find(|i| i.enabled && i.app_id == "redemption_code")
    {
        item.decision = if plan.redemption_codes.is_empty() {
            PlanDecision::NoWork
        } else {
            PlanDecision::Run
        };
    }
    if history.unreadable > 0 {
        plan.warnings.push(format!(
            "{} 份兑换相关历史无法读取；未能证明已处理的码仍会列入本次尝试。",
            history.unreadable
        ));
    }
    plan.redemption_history = history;
    let pending = plan.items.iter().any(|i| i.decision == PlanDecision::Run);
    plan.executable = plan.blockers.is_empty() && pending;
    plan.all_done = plan.blockers.is_empty() && !pending;
}
/// v9 保留旧 v7 清单原义，增加清空文本所用的 Ctrl、A 和退格。
pub fn recovery_inputs() -> Vec<crate::InputProbeConfig> {
    let mut inputs = crate::basic_daily::recovery_inputs();
    for key in [17, 65, 8] {
        inputs.push(crate::InputProbeConfig {
            mode: crate::InputMode::Foreground,
            action: crate::InputProbeAction::Key { key },
            hold_ms: 1,
            start_delay_ms: 0,
        });
    }
    inputs
}

#[path = "redemption_pipeline.rs"]
mod definition;
pub use definition::{ocr_overrides, pipeline};
