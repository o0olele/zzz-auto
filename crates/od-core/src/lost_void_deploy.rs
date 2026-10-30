//! 本次普通出战 / 矩阵开始挑战，到真实局内页面交接；不提供整场完成证明。
use crate::{
    lost_void::LostVoidMission as M,
    lost_void_classic_teams::LostVoidClassicTeamsReceipt,
    lost_void_matrix::{LostVoidMatrixReceipt, MatrixView},
    lost_void_scene::LostVoidScreen as S,
    lost_void_team_plan::{LostVoidTeamKind, LostVoidTeamPlan},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn count_source() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../resources/lost-void/deployment-count-source.json"
    ))
    .expect("固定普通出战人数来源")
}

/// 普通出战页右上角的能力计数：分子为激活数，分母为实际代理人数。
/// 只接受一个完整三字形计数；PP-OCR 的斜线→1 别名已在原图及参考选队器核实。
/// 不从头像缺失或单个数字补值。
pub fn observed_party_count(words: &[String]) -> Option<u8> {
    let mut counts = vec![];
    for word in words {
        let text: String = word
            .chars()
            .filter(|c| !c.is_whitespace())
            .map(|c| if c == '／' { '/' } else { c })
            .collect();
        let chars: Vec<_> = text.chars().collect();
        if !(text.contains('/') || chars.len() == 3 && chars[1] == '1') {
            continue;
        }
        let [active @ '0'..='3', '/' | '1', count @ '0'..='3'] = chars.as_slice() else {
            return None;
        };
        if active > count {
            return None;
        }
        counts.push(*count as u8 - b'0');
    }
    (counts.len() == 1).then(|| counts[0])
}

/// 识别明确的缺员继续出战正文。测试中的句子是语义组合样例，不声称实机原文。
pub fn short_party_notice(words: &[String]) -> bool {
    let text: String = words
        .join("")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if text.len() > 2048 || text.chars().any(char::is_control) {
        return false;
    }
    let contains = |terms: &[&str]| terms.iter().any(|term| text.contains(term));
    !contains(&[
        "等级",
        "培养",
        "养成",
        "支付",
        "购买",
        "消耗",
        "兑换",
        "菲林",
        "电量",
        "体力",
        "仓库",
        "武备",
        "藏品",
        "装备",
        "音擎",
        "驱动盘",
        "悬赏",
        "领取",
        "奖励",
        "商店",
        "是否退出",
        "确认退出",
        "确定退出",
        "是否取消",
        "确认取消",
        "确定取消",
        "终止本次",
        "重新开始",
        "是否返回",
        "确认返回",
        "确定返回",
        "不能继续",
        "无法继续",
        "禁止出战",
        "不允许出战",
        "不可继续",
        "不继续",
        "不再出战",
        "停止出战",
    ]) && contains(&["队伍", "编队", "队员", "代理人"])
        && contains(&[
            "人数", "数量", "满员", "成员", "空位", "空缺", "名", "足够", "1人", "2人", "3人",
            "一人", "二人", "三人",
        ])
        && contains(&[
            "不足",
            "未满",
            "不满",
            "少于",
            "较少",
            "过少",
            "数量少",
            "人数少",
            "空位",
            "空缺",
            "缺员",
            "缺少",
            "未达",
            "未选择足够",
        ])
        && ((contains(&["是否", "确认", "确定", "仍要"])
            && contains(&["继续", "出战", "战斗", "挑战"]))
            || contains(&["继续吗", "继续？", "继续?"]))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeploymentParty {
    Classic {
        receipt: LostVoidClassicTeamsReceipt,
    },
    Matrix {
        receipt: LostVoidMatrixReceipt,
    },
}
impl DeploymentParty {
    pub fn run_id(&self) -> Uuid {
        match self {
            Self::Classic { receipt } => receipt.run_id,
            Self::Matrix { receipt } => receipt.run_id,
        }
    }
    pub fn mission(&self) -> M {
        match self {
            Self::Classic { receipt } => receipt.mission,
            Self::Matrix { .. } => M::MatrixAction,
        }
    }
    pub fn plan(&self) -> &LostVoidTeamPlan {
        match self {
            Self::Classic { receipt } => &receipt.plan,
            Self::Matrix { receipt } => &receipt.plan,
        }
    }
    /// None 必须保持未知，不能从 HUD 中识别到的部分头像补值。
    pub fn expected_count(&self) -> Option<u8> {
        match self {
            Self::Classic { receipt } => receipt.slot.as_ref().map(|s| s.members),
            Self::Matrix { receipt } => Some(receipt.main_agents.len() as u8),
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.run_id().is_nil()
            || self.plan().script_name.is_empty()
            || self.plan().script_name.chars().any(char::is_control)
        {
            return Err("迷失之地出战身份或脚本无效");
        }
        let known = crate::random_play::agents();
        let ids = &self.plan().expected_agents;
        if ids.iter().enumerate().any(|(i, id)| {
            id != crate::predefined_teams::UNKNOWN_AGENT
                && (!known.iter().any(|a| a.id == *id) || ids[..i].contains(id))
        }) || self.plan().up_agents.len() > 3
            || !self.plan().up_agents.is_empty() && self.plan().up_agents.len() != 3
            || self.plan().up_agents.iter().enumerate().any(|(i, id)| {
                !known.iter().any(|a| a.id == *id) || self.plan().up_agents[..i].contains(id)
            })
        {
            return Err("迷失之地出战计划的代理人身份无效");
        }
        if self.expected_count().is_some_and(|count| {
            ids.iter().enumerate().any(|(i, id)| {
                i >= usize::from(count) && id != crate::predefined_teams::UNKNOWN_AGENT
            })
        }) || matches!(&self.plan().kind, LostVoidTeamKind::MatrixManual { agents } if agents != ids)
        {
            return Err("迷失之地实际人数或手选成员与冻结计划矛盾");
        }
        match self {
            Self::Classic { receipt } => {
                if receipt.mission == M::MatrixAction {
                    return Err("普通出战不能使用矩阵副本");
                }
                match (&receipt.plan.kind, &receipt.slot) {
                    (LostVoidTeamKind::KeepCurrent, None) => {}
                    (LostVoidTeamKind::Predefined { index }, Some(slot))
                        if *index < 20
                            && slot.index == *index
                            && (1..=3).contains(&slot.members)
                            && crate::team_selection_slots::valid_identity(slot) => {}
                    _ => return Err("普通出战人数缺少本次预备编队槽位证明"),
                }
            }
            Self::Matrix { receipt } => {
                if receipt.main_agents.len() != 3
                    || !receipt.plan.verify_roster(&receipt.main_agents)?
                    || !crate::random_play::agents()
                        .iter()
                        .any(|a| a.id == receipt.support_agent)
                    || receipt.support_rules.is_empty()
                    || receipt.support_rules.len() > 4
                    || receipt.support_rules.iter().enumerate().any(|(i, r)| {
                        crate::lost_void_matrix::matrix_property(r).as_ref() != Some(r)
                            || receipt.support_rules[..i].contains(r)
                    })
                    || matches!(receipt.plan.kind, LostVoidTeamKind::KeepCurrent)
                {
                    return Err("矩阵出战缺少完整主战、协战及属性证明");
                }
            }
        }
        Ok(())
    }
    fn matches_matrix(&self, v: &MatrixView) -> bool {
        if let Self::Matrix { receipt } = self {
            v.validate().is_ok()
                && v.screen == S::MatrixTeam
                && !v.tutorial
                && v.start_enabled
                && v.main_agents().as_ref() == Some(&receipt.main_agents)
                && v.support_agent() == Some(receipt.support_agent.as_str())
                && v.support_rules == receipt.support_rules
        } else {
            false
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidDeployRequest {
    pub party: DeploymentParty,
    pub timeout_ms: u32,
}
impl LostVoidDeployRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.party.validate()?;
        if !(1000..=300_000).contains(&self.timeout_ms) {
            return Err("迷失之地出战预算无效");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeployPage {
    Unknown,
    Classic,
    Matrix,
    LowLevel,
    ShortParty,
    Blocked,
    InventoryFull,
    Inside(S),
    Failed,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidDeployView {
    pub page: DeployPage,
    pub matrix: Option<MatrixView>,
    #[serde(default)]
    pub member_count: Option<u8>,
}
impl LostVoidDeployView {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.matrix.is_some() != (self.page == DeployPage::Matrix)
            || matches!(self.page, DeployPage::Inside(s) if !arrival(s))
            || self
                .member_count
                .is_some_and(|c| c > 3 || self.page != DeployPage::Classic)
        {
            return Err("迷失之地出战页面与区域数据不一致");
        }
        if let Some(v) = &self.matrix {
            v.validate()?;
        }
        Ok(())
    }
}
fn arrival(s: S) -> bool {
    matches!(s, S::World | S::Gear | S::Event | S::CommonChoice)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeployAction {
    Deploy,
    StartMatrix,
    ConfirmLevel,
    ConfirmParty,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Dispatch {
    Intent,
    Posted,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidDeployReceipt {
    pub party: DeploymentParty,
    pub arrival: S,
    pub member_count: u8,
}
impl LostVoidDeployReceipt {
    pub fn validate(&self) -> Result<(), &'static str> {
        self.party.validate()?;
        if !(1..=3).contains(&self.member_count)
            || !arrival(self.arrival)
            || self
                .party
                .expected_count()
                .is_some_and(|c| c != self.member_count)
            || self
                .party
                .plan()
                .expected_agents
                .iter()
                .skip(usize::from(self.member_count))
                .any(|id| id != crate::predefined_teams::UNKNOWN_AGENT)
        {
            return Err("迷失之地出战回执缺少一致的实际人数证明");
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidDeployFlow {
    request: LostVoidDeployRequest,
    launch: Option<Dispatch>,
    level: Option<Dispatch>,
    #[serde(default)]
    short_party: Option<Dispatch>,
    offered: Option<DeployAction>,
    ready_reads: u8,
    member_count: Option<u8>,
    level_reads: u8,
    #[serde(default)]
    short_reads: u8,
    arrival: Option<S>,
    arrival_reads: u8,
    receipt: Option<LostVoidDeployReceipt>,
}
impl LostVoidDeployFlow {
    pub fn new(request: LostVoidDeployRequest) -> Result<Self, &'static str> {
        request.validate()?;
        Ok(Self {
            request,
            launch: None,
            level: None,
            short_party: None,
            offered: None,
            ready_reads: 0,
            member_count: None,
            level_reads: 0,
            short_reads: 0,
            arrival: None,
            arrival_reads: 0,
            receipt: None,
        })
    }
    pub fn request(&self) -> &LostVoidDeployRequest {
        &self.request
    }
    pub fn receipt(&self) -> Option<&LostVoidDeployReceipt> {
        self.receipt.as_ref()
    }
    pub fn completed(&self) -> bool {
        self.receipt.is_some()
    }
    pub fn launched(&self) -> bool {
        self.launch.is_some()
    }
    fn launch_action(&self) -> DeployAction {
        if self.request.party.mission() == M::MatrixAction {
            DeployAction::StartMatrix
        } else {
            DeployAction::Deploy
        }
    }
    fn ready_count(&self, v: &LostVoidDeployView) -> Option<u8> {
        match &self.request.party {
            DeploymentParty::Classic { .. } if v.page == DeployPage::Classic => {
                match (self.request.party.expected_count(), v.member_count) {
                    (Some(expected), Some(actual)) if expected != actual => None,
                    (Some(expected), _) => Some(expected),
                    (None, observed) => observed,
                }
                .filter(|c| (1..=3).contains(c))
            }
            DeploymentParty::Matrix { .. } => v
                .matrix
                .as_ref()
                .filter(|v| self.request.party.matches_matrix(v))
                .map(|_| 3),
            _ => None,
        }
    }
    fn ready(&self, v: &LostVoidDeployView) -> bool {
        self.member_count.is_some() && self.ready_count(v) == self.member_count
    }
    fn small_party(&self) -> bool {
        matches!(self.member_count, Some(1 | 2))
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        self.request.validate()?;
        if self.ready_reads > 2
            || self.level_reads > 2
            || self.short_reads > 2
            || self.arrival_reads > 2
            || self.member_count.is_some() != (self.launch.is_some() || self.ready_reads > 0)
            || self.member_count.is_some_and(|c| {
                !(1..=3).contains(&c) || self.request.party.expected_count().is_some_and(|e| e != c)
            })
            || self.level.is_some() && self.launch.is_none()
            || (self.short_party.is_some() || self.short_reads > 0)
                && (self.launch.is_none() || !self.small_party())
            || self.arrival.is_some() != (self.arrival_reads > 0)
            || self.arrival.is_some_and(|s| !arrival(s))
            || self.arrival_reads > 0 && self.launch.is_none()
            || self.receipt.is_some() != (self.arrival_reads == 2)
            || self.receipt.as_ref().is_some_and(|r| {
                r.validate().is_err()
                    || r.party != self.request.party
                    || Some(r.arrival) != self.arrival
                    || Some(r.member_count) != self.member_count
            })
            || self.offered.is_some_and(|a| {
                if a == DeployAction::ConfirmLevel {
                    self.launch.is_none() || self.level.is_some() || self.level_reads != 2
                } else if a == DeployAction::ConfirmParty {
                    self.launch.is_none()
                        || !self.small_party()
                        || self.short_party.is_some()
                        || self.short_reads != 2
                } else {
                    a != self.launch_action() || self.launch.is_some() || self.ready_reads != 2
                }
            })
        {
            return Err("迷失之地出战检查点状态不一致");
        }
        Ok(())
    }
    pub fn invalidate_observation(&mut self) {
        self.offered = None;
        self.ready_reads = 0;
        self.level_reads = 0;
        self.short_reads = 0;
        self.arrival = None;
        self.arrival_reads = 0;
        self.receipt = None;
        if self.launch.is_none() {
            self.member_count = None;
        }
    }
    pub fn next(&mut self, v: &LostVoidDeployView) -> Result<Option<DeployAction>, &'static str> {
        self.validate()?;
        v.validate()?;
        if self.completed() {
            return Ok(None);
        }
        self.offered = None;
        if matches!(v.page, DeployPage::InventoryFull | DeployPage::Failed) {
            return Err("迷失之地出战遇到仓库容量或失败 / 结算页面");
        }
        if self.launch.is_none() {
            let count = self.ready_count(v);
            self.ready_reads = match count {
                Some(_) if count == self.member_count => (self.ready_reads + 1).min(2),
                Some(_) => 1,
                None => 0,
            };
            self.member_count = count;
            if self.ready_reads == 2 {
                self.offered = Some(self.launch_action());
            }
        } else if let DeployPage::Inside(s) = v.page {
            self.level_reads = 0;
            self.short_reads = 0;
            self.arrival_reads = if self.arrival == Some(s) {
                (self.arrival_reads + 1).min(2)
            } else {
                1
            };
            self.arrival = Some(s);
            if self.arrival_reads == 2 {
                self.receipt = Some(LostVoidDeployReceipt {
                    party: self.request.party.clone(),
                    arrival: s,
                    member_count: self.member_count.ok_or("出战缺少已冻结的实际人数")?,
                });
            }
        } else {
            self.arrival = None;
            self.arrival_reads = 0;
            self.level_reads = if v.page == DeployPage::LowLevel && self.level.is_none() {
                (self.level_reads + 1).min(2)
            } else {
                0
            };
            self.short_reads = if v.page == DeployPage::ShortParty
                && self.short_party.is_none()
                && self.small_party()
            {
                (self.short_reads + 1).min(2)
            } else {
                0
            };
            if self.level_reads == 2 {
                self.offered = Some(DeployAction::ConfirmLevel);
            } else if self.short_reads == 2 {
                self.offered = Some(DeployAction::ConfirmParty);
            }
        }
        self.validate()?;
        Ok(self.offered)
    }
    pub fn allows(&self, a: DeployAction, v: &LostVoidDeployView) -> bool {
        self.validate().is_ok()
            && v.validate().is_ok()
            && self.offered == Some(a)
            && if a == DeployAction::ConfirmLevel {
                self.launch.is_some() && self.level.is_none() && v.page == DeployPage::LowLevel
            } else if a == DeployAction::ConfirmParty {
                self.launch.is_some()
                    && self.small_party()
                    && self.short_party.is_none()
                    && v.page == DeployPage::ShortParty
            } else {
                self.launch.is_none() && a == self.launch_action() && self.ready(v)
            }
    }
    /// prepare 后用于原生输入的重新核对，不能仅依赖保存前的 offered。
    pub fn prepared_allows(&self, a: DeployAction, v: &LostVoidDeployView) -> bool {
        self.validate().is_ok()
            && v.validate().is_ok()
            && self.offered.is_none()
            && if a == DeployAction::ConfirmLevel {
                self.launch.is_some()
                    && self.level == Some(Dispatch::Intent)
                    && v.page == DeployPage::LowLevel
            } else if a == DeployAction::ConfirmParty {
                self.launch.is_some()
                    && self.small_party()
                    && self.short_party == Some(Dispatch::Intent)
                    && v.page == DeployPage::ShortParty
            } else {
                a == self.launch_action()
                    && self.launch == Some(Dispatch::Intent)
                    && self.level.is_none()
                    && self.short_party.is_none()
                    && self.ready(v)
            }
    }
    pub fn prepare(&mut self, a: DeployAction) -> Result<(), &'static str> {
        self.validate()?;
        if self.offered != Some(a) {
            return Err("迷失之地出战动作没有新帧许可");
        }
        self.offered = None;
        if a == DeployAction::ConfirmLevel {
            self.level = Some(Dispatch::Intent);
        } else if a == DeployAction::ConfirmParty {
            self.short_party = Some(Dispatch::Intent);
        } else {
            self.launch = Some(Dispatch::Intent);
        }
        self.validate()
    }
    pub fn posted(&mut self, a: DeployAction) -> Result<(), &'static str> {
        let pending = if a == DeployAction::ConfirmLevel {
            &mut self.level
        } else if a == DeployAction::ConfirmParty {
            &mut self.short_party
        } else if a == self.launch_action() {
            &mut self.launch
        } else {
            return Err("迷失之地出战动作不匹配");
        };
        if *pending != Some(Dispatch::Intent) {
            return Err("迷失之地出战缺少待投递意图");
        }
        *pending = Some(Dispatch::Posted);
        self.validate()
    }
    pub fn not_posted(&mut self, a: DeployAction) -> Result<(), &'static str> {
        let pending = if a == DeployAction::ConfirmLevel {
            &mut self.level
        } else if a == DeployAction::ConfirmParty {
            &mut self.short_party
        } else if a == self.launch_action() {
            &mut self.launch
        } else {
            return Err("迷失之地出战动作不匹配");
        };
        if *pending != Some(Dispatch::Intent) {
            return Err("迷失之地出战缺少待投递意图");
        }
        *pending = None;
        self.invalidate_observation();
        self.validate()
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.validate()?;
        if self.launch.is_some() {
            self.launch = Some(Dispatch::Posted);
        }
        if self.level.is_some() {
            self.level = Some(Dispatch::Posted);
        }
        if self.short_party.is_some() {
            self.short_party = Some(Dispatch::Posted);
        }
        self.invalidate_observation();
        self.validate()
    }
}
