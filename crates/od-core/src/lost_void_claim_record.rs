//! 领取检查的独立紧凑记录；绑定待领取结算，不增加挑战次数。
use crate::{
    daily::GameDay,
    lost_void_bounty::{BountyAction, BountyFlow, BountyPage, BountyReceipt, BountyRequest},
};
use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_CLAIM_SOURCES: usize = 8192;
fn date(day: &GameDay) -> Result<NaiveDate, &'static str> {
    day.validate()?;
    NaiveDate::parse_from_str(&day.date, "%Y-%m-%d").map_err(|_| "领取记录游戏日无效")
}
pub fn same_claim_week(a: &GameDay, b: &GameDay) -> Result<bool, &'static str> {
    Ok(a.region == b.region && date(a)?.iso_week() == date(b)?.iso_week())
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidClaimRecord {
    instance_id: u32,
    day: GameDay,
    receipt: BountyReceipt,
    source_ids: Vec<Uuid>,
    bounty_observed_day: Option<GameDay>,
    checked_no_available: bool,
    returned_to: BountyPage,
    return_reads: u8,
    action_intents: Vec<BountyAction>,
}
impl LostVoidClaimRecord {
    pub fn from_flow(
        instance_id: u32,
        day: GameDay,
        flow: &BountyFlow,
        mut source_ids: Vec<Uuid>,
        bounty_observed_day: Option<GameDay>,
    ) -> Result<Self, &'static str> {
        flow.validate()?;
        let receipt = flow
            .receipt()
            .ok_or("领取检查尚未确认普通大世界返回")?
            .clone();
        source_ids.sort_unstable();
        let record = Self {
            instance_id,
            day,
            receipt,
            source_ids,
            bounty_observed_day,
            checked_no_available: flow.checked(),
            returned_to: BountyPage::NormalWorld,
            return_reads: 2,
            action_intents: flow.action_intents(),
        };
        record.validate()?;
        Ok(record)
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        date(&self.day)?;
        BountyRequest {
            run_id: self.receipt.run_id,
            claim_id: self.receipt.claim_id,
            source_id: self.receipt.source_id,
            timeout_ms: 1000,
        }
        .validate()?;
        if !(1..=9999).contains(&self.instance_id)
            || !self.checked_no_available
            || self.returned_to != BountyPage::NormalWorld
            || self.return_reads != 2
            || self.source_ids.len() > MAX_CLAIM_SOURCES
            || self.source_ids.iter().any(Uuid::is_nil)
            || self.source_ids.windows(2).any(|w| w[0] >= w[1])
            || !self.source_ids.is_empty() && !self.source_ids.contains(&self.receipt.source_id)
            || self
                .receipt
                .bounty
                .is_some_and(|b| b.total != 8000 || b.current > b.total)
            || self.receipt.reward_seen && !self.receipt.attempted
            || self.action_intents.len() > 6
            || self.action_intents.windows(2).any(|w| w[0] >= w[1])
            || self.receipt.attempted != self.action_intents.contains(&BountyAction::ClaimAll)
            || self.action_intents.contains(&BountyAction::DismissReward)
                && !self.receipt.reward_seen
        {
            return Err("领取记录身份、绑定结算或返回证明不一致");
        }
        if let Some(observed) = &self.bounty_observed_day
            && (self.receipt.bounty.is_none()
                || !same_claim_week(observed, &self.day)?
                || date(observed)? > date(&self.day)?)
        {
            return Err("领取进度读数不属于完成时的游戏周");
        }
        Ok(())
    }
    pub fn instance_id(&self) -> u32 {
        self.instance_id
    }
    pub fn day(&self) -> &GameDay {
        &self.day
    }
    pub fn claim_id(&self) -> Uuid {
        self.receipt.claim_id
    }
    pub fn source_ids(&self) -> &[Uuid] {
        &self.source_ids
    }
    pub fn receipt(&self) -> &BountyReceipt {
        &self.receipt
    }
    pub fn proves_full_bounty(&self, day: &GameDay) -> Result<bool, &'static str> {
        self.validate()?;
        Ok(same_claim_week(&self.day, day)?
            && date(&self.day)? <= date(day)?
            && self.bounty_observed_day.is_some()
            && self.receipt.bounty.is_some_and(|b| b.completed()))
    }
}
