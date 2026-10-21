//! 托管提交与后态分开保存，暂停只能重新确认后态，不能再次停止或重新开始。
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[path = "suibian_temple_pipeline.rs"]
mod definition;
pub use definition::pipeline;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TempleHostingProgress {
    pub stage: String,
    /// Tasker 明确未投递的意图回执；只允许对应的一次意图撤销，不能撤回已提交动作。
    #[serde(default)]
    pub not_submitted: Vec<String>,
    pub stop_attempted: bool,
    pub stop_confirm_attempted: bool,
    pub claim_attempted: bool,
    pub reward_confirm_attempted: bool,
    pub reward_seen: bool,
    pub stopped: bool,
    pub start_attempted: bool,
    pub start_confirm_attempted: bool,
    pub running: bool,
}
impl Default for TempleHostingProgress {
    fn default() -> Self {
        Self {
            stage: "Inspect".into(),
            not_submitted: vec![],
            stop_attempted: false,
            stop_confirm_attempted: false,
            claim_attempted: false,
            reward_confirm_attempted: false,
            reward_seen: false,
            stopped: false,
            start_attempted: false,
            start_confirm_attempted: false,
            running: false,
        }
    }
}
impl TempleHostingProgress {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.not_submitted.len() > 64
            || self.not_submitted.iter().any(|stage| {
                !matches!(
                    stage.as_str(),
                    "Stop" | "ConfirmStop" | "Claim" | "ConfirmReward" | "Start" | "ConfirmStart"
                )
            })
            || !matches!(
                self.stage.as_str(),
                "Inspect"
                    | "OpenLog"
                    | "Stop"
                    | "WaitStop"
                    | "ConfirmStop"
                    | "Claim"
                    | "WaitClaim"
                    | "ConfirmReward"
                    | "WaitReward"
                    | "Start"
                    | "WaitStart"
                    | "ConfirmStart"
                    | "ReturnEntry"
                    | "Done"
            )
            || (self.stop_confirm_attempted && !self.stop_attempted)
            || (self.start_attempted && !self.stopped)
            || (self.start_confirm_attempted && !self.start_attempted)
            || (self.running && !self.start_attempted)
            || (self.reward_seen && !self.stop_attempted && !self.claim_attempted)
            || (self.reward_confirm_attempted && !self.reward_seen)
            || (matches!(self.stage.as_str(), "WaitStop" | "ConfirmStop") && !self.stop_attempted)
            || (self.stage == "WaitClaim" && !self.claim_attempted)
            || (matches!(self.stage.as_str(), "ConfirmReward" | "WaitReward") && !self.reward_seen)
            || (self.stage == "WaitReward" && !self.reward_confirm_attempted)
            || (matches!(self.stage.as_str(), "WaitStart" | "ConfirmStart")
                && !self.start_attempted)
            || (self.running && !matches!(self.stage.as_str(), "ReturnEntry" | "Done"))
            || (matches!(self.stage.as_str(), "ReturnEntry" | "Done") && !self.running)
            || (matches!(self.stage.as_str(), "Start" | "WaitStart" | "ConfirmStart")
                && !self.stopped)
        {
            return Err("随便观托管检查点缺少提交或后态证明");
        }
        Ok(())
    }
    pub fn completed(&self) -> bool {
        self.stage == "Done" && self.running && self.validate().is_ok()
    }
    pub fn entry(&self) -> String {
        format!("Hosting.{}", self.stage)
    }
    pub fn check_update(&self, next: &Self) -> Result<(), &'static str> {
        self.validate()?;
        next.validate()?;
        if !next.not_submitted.starts_with(&self.not_submitted)
            || next.not_submitted.len() > self.not_submitted.len() + 1
        {
            return Err("未投递回执不能删除或批量添加");
        }
        let receipt = if next.not_submitted.len() > self.not_submitted.len() {
            next.not_submitted.last().map(String::as_str)
        } else {
            None
        };
        let mut reverted = None;
        for (name, old, new) in [
            ("Stop", self.stop_attempted, next.stop_attempted),
            (
                "ConfirmStop",
                self.stop_confirm_attempted,
                next.stop_confirm_attempted,
            ),
            ("Claim", self.claim_attempted, next.claim_attempted),
            ("RewardSeen", self.reward_seen, next.reward_seen),
            (
                "ConfirmReward",
                self.reward_confirm_attempted,
                next.reward_confirm_attempted,
            ),
            ("Stopped", self.stopped, next.stopped),
            ("Start", self.start_attempted, next.start_attempted),
            (
                "ConfirmStart",
                self.start_confirm_attempted,
                next.start_confirm_attempted,
            ),
            ("Running", self.running, next.running),
        ] {
            if old && !new {
                if receipt != Some(name)
                    || self.stage != name
                    || next.stage != self.stage
                    || reverted.is_some()
                {
                    return Err("已保存的托管提交或结果不能撤销");
                }
                reverted = Some(name);
            }
        }
        if receipt != reverted {
            return Err("未投递回执必须对应本步骤尚未投递的意图");
        }
        if self.completed() && self != next {
            return Err("已完成托管不能重放");
        }
        Ok(())
    }
    /// 只在调用前已取消 / 耗尽预算，或 SDK 明确返回 task_id=None 时调用。已投递后的取消不能走此路径。
    pub fn not_posted(&mut self, before: &Self) -> Result<(), &'static str> {
        before.check_update(self)?;
        if before.stage != self.stage || before.not_submitted != self.not_submitted {
            return Err("未投递回执与意图不匹配");
        }
        let mut expected = before.clone();
        expected.before_step()?;
        if expected != *self {
            return Err("未投递期间发生了其他推进");
        }
        let mut next = before.clone();
        if next != *self {
            next.not_submitted.push(self.stage.clone());
        }
        self.check_update(&next)?;
        *self = next;
        Ok(())
    }
    fn edit(
        &mut self,
        change: impl FnOnce(&mut Self) -> Result<(), &'static str>,
    ) -> Result<(), &'static str> {
        self.validate()?;
        let mut next = self.clone();
        change(&mut next)?;
        self.check_update(&next)?;
        *self = next;
        Ok(())
    }
    pub fn before_step(&mut self) -> Result<(), &'static str> {
        self.edit(|s| {
            let attempted = match s.stage.as_str() {
                "Stop" => Some(&mut s.stop_attempted),
                "ConfirmStop" => Some(&mut s.stop_confirm_attempted),
                "Claim" => Some(&mut s.claim_attempted),
                "ConfirmReward" => Some(&mut s.reward_confirm_attempted),
                "Start" => Some(&mut s.start_attempted),
                "ConfirmStart" => Some(&mut s.start_confirm_attempted),
                _ => None,
            };
            if let Some(attempted) = attempted {
                if *attempted {
                    return Err("不能重放托管提交");
                }
                *attempted = true;
            }
            Ok(())
        })
    }
    pub fn observe(&mut self, node: &str) -> Result<(), &'static str> {
        let node = node.strip_prefix("Hosting.").ok_or("不是托管步骤的节点")?;
        self.edit(|s| {
            let next = match (s.stage.as_str(), node) {
                ("Inspect" | "OpenLog", "EntryManaged" | "EntryManual") => "OpenLog",
                ("Inspect" | "OpenLog", "LogRunning") if !s.stopped => "Stop",
                ("Inspect" | "OpenLog", "LogStopped") => {
                    s.stopped = true;
                    "Start"
                }
                ("Inspect" | "OpenLog", "Claimable") => "Claim",
                ("Stop", "StopClicked") if s.stop_attempted => "WaitStop",
                ("ConfirmStop", "StopConfirmed") if s.stop_confirm_attempted => "WaitStop",
                ("Claim", "ClaimClicked") if s.claim_attempted => "WaitClaim",
                ("ConfirmReward", "RewardConfirmed") if s.reward_confirm_attempted => "WaitReward",
                ("Start", "StartClicked") if s.start_attempted => "WaitStart",
                ("ConfirmStart", "StartConfirmed") if s.start_confirm_attempted => "WaitStart",
                ("WaitStop", "LogRunning" | "StopPrompt") if s.stop_attempted => {
                    if node == "StopPrompt" && !s.stop_confirm_attempted {
                        "ConfirmStop"
                    } else {
                        "WaitStop"
                    }
                }
                ("WaitStop" | "WaitClaim" | "WaitReward", "Claimable")
                    if s.stop_attempted || s.claim_attempted =>
                {
                    if s.claim_attempted {
                        "WaitClaim"
                    } else {
                        "Claim"
                    }
                }
                ("WaitStop" | "WaitClaim" | "WaitReward", "Reward")
                    if s.stop_attempted || s.claim_attempted =>
                {
                    s.reward_seen = true;
                    if s.reward_confirm_attempted {
                        "WaitReward"
                    } else {
                        "ConfirmReward"
                    }
                }
                ("WaitStop" | "WaitClaim" | "WaitReward", "LogStopped") => {
                    s.stopped = true;
                    "Start"
                }
                ("WaitStop" | "WaitClaim" | "WaitReward", "EntryManual") => {
                    s.stopped = true;
                    "OpenLog"
                }
                ("WaitStart", "StartPrompt") if s.start_attempted => {
                    if s.start_confirm_attempted {
                        "WaitStart"
                    } else {
                        "ConfirmStart"
                    }
                }
                ("WaitStart", "LogStopped") if s.start_attempted => "WaitStart",
                ("WaitStart", "LogRunning") if s.start_attempted => {
                    s.running = true;
                    "ReturnEntry"
                }
                ("WaitStart" | "ReturnEntry", "EntryManaged") if s.start_attempted => {
                    s.running = true;
                    "Done"
                }
                ("ReturnEntry", "LogRunning") if s.running => "ReturnEntry",
                _ => return Err("托管节点不属于当前步骤或缺少提交证明"),
            };
            s.stage = next.into();
            Ok(())
        })
    }
    pub fn resume(&mut self) -> Result<(), &'static str> {
        self.edit(|s| {
            if s.completed() {
                return Ok(());
            }
            s.stage = if s.running {
                "ReturnEntry"
            } else if s.start_attempted {
                "WaitStart"
            } else if s.stopped {
                "Inspect"
            } else if s.reward_confirm_attempted {
                "WaitReward"
            } else if s.claim_attempted {
                "WaitClaim"
            } else if s.stop_attempted {
                "WaitStop"
            } else {
                "Inspect"
            }
            .into();
            Ok(())
        })
    }
}
