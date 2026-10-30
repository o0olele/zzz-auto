//! 首次输入前编译全部可能脚本；选队后只使用冻结结果，不提供截图或通关证明。
use crate::{combat::CombatRun, simulation_deploy::PreparedDeployment};
use anyhow::{Context, Result, ensure};
use maa_adapter::combat_vision::CombatBundle;
use od_core::{
    app_combat::AppCombatPlan,
    combat::CombatConfig,
    lost_void::{LostVoidChallenge, LostVoidMission},
    lost_void_team_plan::{LostVoidTeamPlan, plan_team, potential_agents, potential_scripts},
    pipeline::PipelineControl,
    predefined_teams::TeamConfig,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LostVoidCombatSeed {
    pub run_id: Uuid,
    pub mission: LostVoidMission,
    pub challenge: LostVoidChallenge,
    pub teams: TeamConfig,
    pub settings: CombatConfig,
    pub script_revision: String,
    pub up_completed_this_week: bool,
}
impl LostVoidCombatSeed {
    fn combat_plan(&self, name: &str) -> AppCombatPlan {
        let mut config = self.settings.clone();
        config.script = name.into();
        config.dodge_only = false;
        config.start_delay_ms = 0;
        AppCombatPlan {
            config,
            script_revision: self.script_revision.clone(),
        }
    }
    /// 完整准备的前置条件；独立于尚未读取的本周 UP 及实际出战人数。
    pub fn validate_roster_settings(&self) -> Result<()> {
        ensure!(
            self.settings.mode == od_core::InputMode::Foreground,
            "迷失之地队伍准备目前要求前台键鼠"
        );
        let next = self
            .settings
            .binding("切换角色-下一个")
            .map_err(anyhow::Error::msg)?;
        let previous = self
            .settings
            .binding("切换角色-上一个")
            .map_err(anyhow::Error::msg)?;
        od_core::lost_void_roster::validate_switch_inputs(&next, &previous)
            .map_err(anyhow::Error::msg)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.run_id.is_nil(), "迷失之地战斗准备缺少运行身份");
        self.challenge.validate().map_err(anyhow::Error::msg)?;
        self.teams.validate().map_err(anyhow::Error::msg)?;
        self.settings.validate().map_err(anyhow::Error::msg)?;
        self.combat_plan(&self.challenge.auto_battle)
            .validate()
            .map_err(anyhow::Error::msg)
    }
}

pub struct LostVoidResourcePaths<'a> {
    pub scene: &'a Path,
    pub daily: Option<&'a Path>,
    pub combat: &'a Path,
}

/// 场景、选队及战斗准备；层流程另用 with_npc_detector 固定导航资源。
pub struct FrozenLostVoidPreparation {
    seed: LostVoidCombatSeed,
    prepared: BTreeMap<String, PreparedDeployment>,
    agent_ids: BTreeSet<String>,
    agent_kinds: BTreeMap<String, String>,
    scene: PathBuf,
    daily: Option<PathBuf>,
    combat: PathBuf,
    npc_detector: Option<PathBuf>,
    hashes: Vec<(PathBuf, Vec<u8>)>,
}
impl FrozenLostVoidPreparation {
    pub fn load(
        seed: LostVoidCombatSeed,
        paths: LostVoidResourcePaths<'_>,
        data: &Path,
    ) -> Result<Self> {
        seed.validate()?;
        let scene = maa_adapter::lost_void_scene::validate_lost_void_scene_bundle(paths.scene)?;
        let daily = if seed.mission == LostVoidMission::MatrixAction {
            None
        } else {
            let root = maa_adapter::validate_basic_daily_bundle(
                paths.daily.context("普通迷失之地准备缺少按槽位选队资源")?,
            )?;
            let manifest: serde_json::Value =
                serde_json::from_slice(&std::fs::read(root.join("basic-daily-manifest.json"))?)?;
            ensure!(
                manifest["schema_version"] == 10,
                "迷失之地普通选队需要 schema 10 日常资源"
            );
            Some(root)
        };
        // 先保存清单，再编译；最后复验避免准备期间切换资源。
        let hashes = std::iter::once(scene.join("lost_void_scene-manifest.json"))
            .chain(daily.iter().map(|p| p.join("basic-daily-manifest.json")))
            .map(|path| Ok((path.clone(), Sha256::digest(std::fs::read(&path)?).to_vec())))
            .collect::<Result<Vec<_>>>()?;
        let bundle = CombatBundle::load(paths.combat)?;
        ensure!(
            crate::combat::script_catalog(&bundle, data)?.revision == seed.script_revision,
            "迷失之地战斗脚本或资源修订已变化"
        );
        let agent_ids = potential_agents(
            seed.mission,
            &seed.challenge,
            &seed.teams,
            seed.up_completed_this_week,
        )
        .map_err(anyhow::Error::msg)?;
        let available: BTreeSet<_> = bundle.spec.agents.iter().map(|a| a.id.as_str()).collect();
        ensure!(
            available.len() == bundle.spec.agents.len(),
            "迷失之地战斗资源代理人身份重复"
        );
        for id in &agent_ids {
            ensure!(
                available.contains(id.as_str()),
                "迷失之地候选主战缺少战斗资源：{id}"
            );
        }
        let scripts = potential_scripts(
            seed.mission,
            &seed.challenge,
            &seed.teams,
            seed.up_completed_this_week,
        )
        .map_err(anyhow::Error::msg)?;
        let mut prepared = BTreeMap::new();
        for script in scripts {
            prepared.insert(
                script.clone(),
                PreparedDeployment::load(seed.combat_plan(&script), &bundle.root, data)?,
            );
        }
        ensure!(!prepared.is_empty(), "迷失之地没有可冻结的战斗脚本");
        ensure!(
            crate::combat::script_catalog(&bundle, data)?.revision == seed.script_revision,
            "迷失之地候选脚本编译期间修订已变化"
        );
        let frozen = Self {
            seed,
            prepared,
            agent_ids,
            agent_kinds: bundle
                .spec
                .agents
                .iter()
                .map(|a| (a.id.clone(), a.kind.clone()))
                .collect(),
            scene,
            daily,
            combat: bundle.root.clone(),
            npc_detector: None,
            hashes,
        };
        frozen.verify()?;
        Ok(frozen)
    }
    pub fn seed(&self) -> &LostVoidCombatSeed {
        &self.seed
    }
    /// 应用首次输入前已编译的候选可用于新挑战；只改变身份与本周 UP 使用状态。
    pub fn for_next_challenge(&self, run_id: Uuid, up_completed_this_week: bool) -> Result<Self> {
        ensure!(
            !run_id.is_nil() && run_id != self.seed.run_id,
            "新挑战须使用独立身份"
        );
        self.verify()?;
        let mut seed = self.seed.clone();
        seed.run_id = run_id;
        seed.up_completed_this_week = up_completed_this_week;
        seed.validate()?;
        let agents = potential_agents(
            seed.mission,
            &seed.challenge,
            &seed.teams,
            seed.up_completed_this_week,
        )
        .map_err(anyhow::Error::msg)?;
        ensure!(
            agents.is_subset(&self.agent_ids),
            "新挑战候选代理人未在应用首次输入前冻结"
        );
        let scripts = potential_scripts(
            seed.mission,
            &seed.challenge,
            &seed.teams,
            seed.up_completed_this_week,
        )
        .map_err(anyhow::Error::msg)?;
        ensure!(
            scripts.iter().all(|name| self.prepared.contains_key(name)),
            "新挑战脚本未在应用首次输入前编译"
        );
        let frozen = Self {
            seed,
            prepared: self.prepared.clone(),
            agent_ids: self.agent_ids.clone(),
            agent_kinds: self.agent_kinds.clone(),
            scene: self.scene.clone(),
            daily: self.daily.clone(),
            combat: self.combat.clone(),
            npc_detector: self.npc_detector.clone(),
            hashes: self.hashes.clone(),
        };
        frozen.verify()?;
        Ok(frozen)
    }
    pub fn script_names(&self) -> Vec<&str> {
        self.prepared.keys().map(String::as_str).collect()
    }
    /// 只接纳前项核验的编队信息，不扩大整组首次输入前冻结的脚本和代理人集合。
    pub(crate) fn with_group_teams(mut self, teams: TeamConfig) -> Result<Self> {
        teams.validate().map_err(anyhow::Error::msg)?;
        let agents = potential_agents(self.seed.mission, &self.seed.challenge, &teams, false)
            .map_err(anyhow::Error::msg)?;
        let scripts = potential_scripts(self.seed.mission, &self.seed.challenge, &teams, false)
            .map_err(anyhow::Error::msg)?;
        ensure!(
            agents.is_subset(&self.agent_ids)
                && scripts.iter().all(|s| self.prepared.contains_key(s)),
            "前序编队检查需要未在组输入前冻结的迷失之地候选，请重新预览运行"
        );
        self.seed.teams = teams;
        self.seed.validate()?;
        self.verify()?;
        Ok(self)
    }
    pub fn required_agents(&self) -> &BTreeSet<String> {
        &self.agent_ids
    }
    pub fn scene_root(&self) -> &Path {
        &self.scene
    }
    pub fn daily_root(&self) -> Option<&Path> {
        self.daily.as_deref()
    }
    /// 与 scene_root 一样只返回冻结路径，完整内容核对由 verify 完成。
    pub fn combat_root(&self) -> &Path {
        &self.combat
    }
    /// 新层流程在首次输入前扩展依赖；固定模型及标签逐字节校验。
    pub fn with_npc_detector(mut self, root: &Path) -> Result<Self> {
        self.npc_detector = Some(maa_adapter::yolo::validate_challenge_detector(root)?);
        self.verify()?;
        Ok(self)
    }
    pub fn npc_detector_root(&self) -> Option<&Path> {
        self.npc_detector.as_deref()
    }
    /// 人数须来自本次出战页，不能从缺少后台头像推测。
    pub fn roster_request(
        &self,
        plan: &LostVoidTeamPlan,
        level_id: Uuid,
        expected_count: u8,
        options: od_core::lost_void::ArtifactSelection,
        timeout_ms: u32,
    ) -> Result<od_core::lost_void_roster::RosterRequest> {
        self.verify()?;
        let up = (!plan.up_agents.is_empty()).then_some(plan.up_agents.as_slice());
        ensure!(
            self.resolve_plan(up)? == *plan,
            "实际队伍请求与冻结选队不符"
        );
        self.seed.validate_roster_settings()?;
        let request = od_core::lost_void_roster::RosterRequest {
            run_id: self.seed.run_id,
            level_id,
            plan: plan.clone(),
            expected_count,
            challenge: self.seed.challenge.clone(),
            options,
            next_inputs: self
                .seed
                .settings
                .binding("切换角色-下一个")
                .map_err(anyhow::Error::msg)?,
            previous_inputs: self
                .seed
                .settings
                .binding("切换角色-上一个")
                .map_err(anyhow::Error::msg)?,
            switch_for_moving: true,
            timeout_ms,
        };
        request.validate().map_err(anyhow::Error::msg)?;
        Ok(request)
    }
    fn verify_roster_request(
        &self,
        request: &od_core::lost_void_roster::RosterRequest,
    ) -> Result<()> {
        request.validate().map_err(anyhow::Error::msg)?;
        let up = (!request.plan.up_agents.is_empty()).then_some(request.plan.up_agents.as_slice());
        ensure!(
            request.run_id == self.seed.run_id
                && request.challenge == self.seed.challenge
                && self.resolve_plan(up)? == request.plan
                && request.next_inputs
                    == self
                        .seed
                        .settings
                        .binding("切换角色-下一个")
                        .map_err(anyhow::Error::msg)?
                && request.previous_inputs
                    == self
                        .seed
                        .settings
                        .binding("切换角色-上一个")
                        .map_err(anyhow::Error::msg)?
                && self.seed.settings.mode == od_core::InputMode::Foreground,
            "队伍请求已偏离冻结配置或绑定"
        );
        self.verify()
    }
    pub fn create_roster_run(
        &self,
        request: od_core::lost_void_roster::RosterRequest,
        control: PipelineControl,
    ) -> Result<crate::lost_void_roster::LostVoidRosterRun> {
        self.verify_roster_request(&request)?;
        crate::lost_void_roster::LostVoidRosterRun::new(request, control)
    }
    fn verify_roster_run<'a>(
        &self,
        roster: &'a crate::lost_void_roster::LostVoidRosterRun,
        control: &PipelineControl,
    ) -> Result<&'a od_core::lost_void_roster::RosterReceipt> {
        ensure!(roster.uses_control(control), "实际队伍与子战斗须共用父控制");
        let receipt = roster.receipt().context("实际队伍尚未安全交回")?;
        self.verify_roster_request(roster.request())?;
        ensure!(
            receipt.run_id == self.seed.run_id
                && receipt.level_id == roster.request().level_id
                && receipt.plan == roster.request().plan
                && receipt
                    .members
                    .iter()
                    .all(|m| self.agent_kinds.get(&m.id) == Some(&m.kind)),
            "实际队伍身份或类型与冻结资源不符"
        );
        Ok(receipt)
    }
    /// 只接受已保存、释放并在新帧核对的实际执行器回执；截图不能由配置名单代替。
    pub fn create_segment_after_roster(
        &self,
        roster: &crate::lost_void_roster::LostVoidRosterRun,
        control: PipelineControl,
    ) -> Result<CombatRun> {
        let receipt = self.verify_roster_run(roster, &control)?;
        self.create_segment(
            &receipt.plan,
            &receipt
                .members
                .iter()
                .map(|m| m.id.clone())
                .collect::<Vec<_>>(),
            control,
        )
    }

    /// 同控制、同层的实际执行器才能开放本次局内战斗交接。
    pub fn create_battle_after_navigation(
        &self,
        roster: &crate::lost_void_roster::LostVoidRosterRun,
        navigation: &crate::lost_void_navigation::LostVoidNavigationRun,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<crate::lost_void_battle::LostVoidBattleRun> {
        ensure!(
            navigation.uses_control(&control),
            "导航与局内战斗须共用父控制"
        );
        let receipt = navigation.receipt().context("导航遭遇尚未安全交回")?;
        let nav = navigation.snapshot();
        ensure!(
            nav.flow.request().challenge == self.seed.challenge,
            "导航挑战配置与冻结准备不符"
        );
        let segment = self.create_segment_after_roster(roster, control.clone())?;
        let request = od_core::lost_void_battle::BattleRequest {
            roster: roster.receipt().context("实际队伍尚未安全交回")?.clone(),
            encounter: receipt.clone(),
            region: nav.flow.memory().region().into(),
            timeout_ms,
        };
        crate::lost_void_battle::LostVoidBattleRun::new(self, request, segment, control)
    }
    /// 出口意图仍不能当作到达；后续执行器只读等待明确离开及完整队伍。
    pub fn create_transition_after_navigation(
        &self,
        roster: &crate::lost_void_roster::LostVoidRosterRun,
        navigation: &crate::lost_void_navigation::LostVoidNavigationRun,
        next_level_id: Uuid,
        timeout_ms: u32,
        control: PipelineControl,
    ) -> Result<crate::lost_void_transition::LostVoidTransitionRun> {
        ensure!(navigation.uses_control(&control), "跨层与导航须共用父控制");
        let roster = self.verify_roster_run(roster, &control)?.clone();
        let exit = navigation
            .receipt()
            .context("出口导航尚未安全交回")?
            .clone();
        let nav = navigation.snapshot();
        ensure!(nav.flow.request().challenge==self.seed.challenge
            &&nav.flow.memory().pending_event().is_some_and(|(id,target)|id==exit.interaction_id && matches!(&exit.outcome,od_core::lost_void_navigation::NavOutcome::InteractionSubmitted{target:expected} if target==expected)),"出口目标或挑战身份与本次导航不一致");
        let request = od_core::lost_void_transition::TransitionRequest {
            roster,
            exit,
            next_level_id,
            timeout_ms,
        };
        crate::lost_void_transition::LostVoidTransitionRun::new(
            self,
            request,
            nav.flow.memory().clone(),
            nav.flow.departure().cloned(),
            control,
        )
    }
    pub fn verify(&self) -> Result<()> {
        for (path, hash) in &self.hashes {
            od_core::config::reject_links(path)?;
            ensure!(
                Sha256::digest(std::fs::read(path)?).as_slice() == hash,
                "迷失之地准备后场景或选队清单已变化"
            );
        }
        maa_adapter::lost_void_scene::validate_lost_void_scene_bundle(&self.scene)?;
        if let Some(root) = &self.daily {
            maa_adapter::validate_basic_daily_bundle(root)?;
        }
        if let Some(root) = &self.npc_detector {
            maa_adapter::yolo::validate_challenge_detector(root)?;
        }
        // 候选脚本来自同一修订、同一路径；一次完整资源复验覆盖其全部文件。
        self.prepared
            .values()
            .next()
            .context("迷失之地缺少冻结脚本")?
            .resource_root()?;
        Ok(())
    }
    pub fn resolve_plan(&self, up_agents: Option<&[String]>) -> Result<LostVoidTeamPlan> {
        ensure!(
            self.seed.mission == LostVoidMission::TaskForce || up_agents.is_none(),
            "非特遣副本不能采用特遣 UP 名单"
        );
        let plan = plan_team(
            self.seed.mission,
            &self.seed.challenge,
            &self.seed.teams,
            up_agents,
            self.seed.up_completed_this_week,
        )
        .map_err(anyhow::Error::msg)?
        .context("迷失之地 UP 选队缺少本周完整名单")?;
        ensure!(
            self.prepared.contains_key(&plan.script_name),
            "迷失之地选中脚本未在首次输入前编译"
        );
        Ok(plan)
    }
    /// 名单来自父流程的新帧识别；本方法核对身份和冻结计划，本身不产生识别证明。
    pub fn create_segment(
        &self,
        plan: &LostVoidTeamPlan,
        main_agents: &[String],
        control: PipelineControl,
    ) -> Result<CombatRun> {
        let up = (!plan.up_agents.is_empty()).then_some(plan.up_agents.as_slice());
        ensure!(
            self.resolve_plan(up)? == *plan,
            "迷失之地选队计划与冻结配置不符"
        );
        ensure!(
            plan.verify_roster(main_agents)
                .map_err(anyhow::Error::msg)?
                && main_agents.iter().all(|id| self.agent_ids.contains(id)),
            "迷失之地主战身份与冻结计划不符"
        );
        self.verify()?;
        self.prepared
            .get(&plan.script_name)
            .context("迷失之地冻结脚本缺失")?
            .frozen_segment(self.seed.run_id, control)
    }
}
