use crate::{NativeSession, RecognitionImage};
use anyhow::{Context, Result, ensure};
use od_core::{
    combat_vision::{AgentStateDef, VisionSpec},
    conditional::StateUpdate,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};

pub struct CombatBundle {
    pub root: PathBuf,
    pub spec: VisionSpec,
    pub scripts: Vec<String>,
    pub dodge_scripts: Vec<String>,
    masks: BTreeMap<String, Vec<u8>>,
}
impl CombatBundle {
    pub fn load(root: &Path) -> Result<Self> {
        od_core::config::reject_links(root)?;
        let root = root
            .canonicalize()
            .context("战斗资源缺失，请先生成 combat-bundle")?;
        let manifest_file = root.join("combat-manifest.json");
        od_core::config::reject_links(&manifest_file)?;
        ensure!(
            manifest_file.metadata()?.len() <= 2 * 1024 * 1024,
            "战斗清单过大"
        );
        let manifest: Value = serde_json::from_slice(&std::fs::read(manifest_file)?)?;
        ensure!(
            manifest["schema"] == 1
                && manifest["sdk_version"] == crate::SDK_VERSION
                && manifest["accepted"] == false
                && manifest["base_size"] == json!([1920, 1080]),
            "战斗资源版本或验收标记无效"
        );
        let mut seen = BTreeSet::new();
        let mut bytes_total = 0u64;
        let files = manifest["files"].as_array().context("战斗文件列表缺失")?;
        ensure!(files.len() <= 4096, "战斗文件数量过大");
        for file in files {
            let name = file["path"].as_str().context("战斗文件名无效")?;
            let path = Path::new(name);
            ensure!(
                !name.contains([':', '\\'])
                    && path
                        .components()
                        .all(|c| matches!(c, std::path::Component::Normal(_)))
                    && seen.insert(name.to_string()),
                "战斗文件路径无效或重复"
            );
            let target = root.join(path);
            od_core::config::reject_links(&target)?;
            ensure!(
                target.canonicalize()?.starts_with(&root),
                "战斗文件指向包外"
            );
            let size = target.metadata()?.len();
            bytes_total += size;
            ensure!(
                size <= 32 * 1024 * 1024
                    && bytes_total <= 256 * 1024 * 1024
                    && file["bytes"].as_u64() == Some(size),
                "战斗文件大小不匹配"
            );
            ensure!(
                file["sha256"] == hex::encode(Sha256::digest(std::fs::read(target)?)),
                "战斗文件哈希不匹配：{name}"
            );
        }
        for entry in walkdir::WalkDir::new(&root).min_depth(1) {
            let entry = entry?;
            od_core::config::reject_links(entry.path())?;
            if entry.file_type().is_file() {
                let path = entry
                    .path()
                    .strip_prefix(&root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                ensure!(
                    path == "combat-manifest.json" || seen.contains(&path),
                    "战斗包含清单外文件"
                );
            }
        }
        ensure!(
            std::fs::read(root.join("pipeline/combat.json"))?
                == br#"{"Combat.NoInput":{"recognition":"DirectHit","action":"DoNothing"}}"#,
            "战斗包不得包含输入 Pipeline"
        );
        let spec: VisionSpec = serde_json::from_slice(&std::fs::read(root.join("vision.json"))?)?;
        ensure!(
            spec.schema == 1 && spec.agents.len() <= 256 && spec.templates.len() <= 2048,
            "战斗识别定义无效"
        );
        let mut masks = BTreeMap::new();
        for (name, template) in &spec.templates {
            ensure!(seen.contains(&template.image), "战斗模板不在清单中");
            ensure!(
                template.image.starts_with("image/"),
                "战斗模板必须位于 Maa image 目录"
            );
            let dimensions = |path: &str| -> Result<(u32, u32)> {
                let size =
                    image::ImageReader::new(std::io::Cursor::new(std::fs::read(root.join(path))?))
                        .with_guessed_format()?
                        .into_dimensions()?;
                ensure!(
                    (1..=1920).contains(&size.0) && (1..=1080).contains(&size.1),
                    "战斗模板或掩码尺寸超出 1080p"
                );
                Ok(size)
            };
            let original_size = dimensions(&template.image)?;
            if let Some(rect) = template.rect {
                od_core::Roi::from_legacy(rect).map_err(anyhow::Error::msg)?;
            }
            if let Some(mask) = &template.mask {
                ensure!(seen.contains(mask), "战斗掩码不在清单中");
                ensure!(dimensions(mask)? == original_size, "战斗掩码尺寸不匹配");
                let decoded = image::load_from_memory(&std::fs::read(root.join(mask))?)?.to_luma8();
                ensure!(
                    decoded.width() == original_size.0
                        && decoded.height() == original_size.1
                        && decoded.width() <= 1920
                        && decoded.height() <= 1080,
                    "战斗掩码尺寸不匹配"
                );
                masks.insert(name.clone(), decoded.into_raw());
            }
        }
        for rect in spec.areas.values() {
            od_core::Roi::from_legacy(*rect).map_err(anyhow::Error::msg)?;
        }
        // 模型名称与文件字节都固定，不能仅靠自描述清单替换模型。
        let lock: Value = serde_json::from_str(include_str!("../../../models.lock.json"))?;
        for model in lock["models"].as_array().context("固定模型清单无效")? {
            if model["role"] != "ocr" && model["role"] != "classify" {
                continue;
            }
            for file in model["files"].as_array().context("模型文件缺失")? {
                if let Some(name) = file["bundle_path"].as_str() {
                    ensure!(
                        file["sha256"]
                            == hex::encode(Sha256::digest(std::fs::read(root.join(name))?)),
                        "固定战斗模型不匹配"
                    );
                }
            }
        }
        let scripts: Vec<String> = serde_json::from_value(manifest["scripts"].clone())?;
        ensure!(
            !scripts.is_empty() && scripts.len() <= 256,
            "战斗脚本目录无效"
        );
        for name in &scripts {
            spec.validate_script(&od_core::conditional::Script::load(
                &root.join("scripts"),
                name,
            )?)?;
        }
        let dodge_scripts: Vec<String> = serde_json::from_value(manifest["dodge_scripts"].clone())?;
        ensure!(
            !dodge_scripts.is_empty() && dodge_scripts.len() <= 256,
            "闪避脚本目录无效"
        );
        for name in &dodge_scripts {
            spec.validate_script(&od_core::conditional::Script::load_entry(
                &root.join("scripts"),
                "dodge",
                name,
                false,
            )?)?;
        }
        Ok(Self {
            root,
            spec,
            scripts,
            dodge_scripts,
            masks,
        })
    }
}

pub struct CombatFrame {
    pub(crate) image: RecognitionImage,
    pub captured: Instant,
}
impl CombatFrame {
    pub fn age_ms(&self) -> u32 {
        self.captured.elapsed().as_millis().min(u32::MAX as u128) as u32
    }
}
impl NativeSession {
    /// 应用开始前载入战斗资源，保留同一 Tasker、控制器与运行控制权。
    pub fn load_combat_bundle(&mut self, root: &Path) -> Result<CombatRecognizer> {
        ensure!(
            !self.tasker.running()
                && !self.input_cleanup_pending
                && !self.pipeline_controller_pending
                && self.held_keys.is_empty()
                && self.held_contacts.is_empty(),
            "载入战斗资源前须结束任务并释放全部输入"
        );
        let bundle = CombatBundle::load(root)?;
        ensure!(
            self._resource
                .post_bundle(bundle.root.to_str().context("战斗路径不是 UTF-8")?)?
                .wait()
                .is_success(),
            "战斗资源载入失败"
        );
        // 闪光原始推理不经过 Maa 的资源查找，必须使用同一份已校验战斗包。
        *self
            .raw_flash
            .lock()
            .map_err(|_| anyhow::anyhow!("闪光识别锁失效"))? = None;
        self.combat_bundle = Some(bundle.root.clone());
        Ok(CombatRecognizer::new(bundle))
    }

    pub fn check_combat_input_mode(&self, mode: od_core::InputMode) -> Result<()> {
        ensure!(
            self.input_mode == Some(mode),
            "战斗输入方式与当前控制器不匹配"
        );
        Ok(())
    }

    pub fn combat_frame(&self, cancel: &AtomicBool) -> Result<Option<CombatFrame>> {
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let captured = Instant::now();
        ensure!(
            self.wait_input_job(self.controller.post_screencap()?, cancel, &mut None)?
                .is_success(),
            "战斗截图失败"
        );
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let buffer = self.controller.cached_image()?;
        ensure!(
            buffer.width() == 1920
                && buffer.height() == 1080
                && buffer.channels() == 3
                && buffer.image_type() == 16,
            "战斗截图须为连续 1080p BGR8"
        );
        Ok(Some(CombatFrame {
            image: RecognitionImage {
                buffer,
                prepare_us: captured.elapsed().as_micros() as u64,
            },
            captured,
        }))
    }
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct CombatObservation {
    pub in_battle: bool,
    pub states: Vec<StateUpdate>,
    pub team: Vec<String>,
    pub quick_assist: Option<String>,
    pub chain: Vec<Option<String>>,
}
pub struct CombatRecognizer {
    pub bundle: CombatBundle,
    candidates: Vec<usize>,
    stable: u32,
    last_team: Vec<Option<usize>>,
    last: BTreeMap<String, f64>,
    raw: Arc<Mutex<Option<crate::RawRecognitionControl>>>,
    quick: Option<(String, f64)>,
    chain: Option<(Vec<Option<String>>, f64)>,
}
impl CombatRecognizer {
    /// 只读取同一截图中的固定普通结算区域；调用方须确认已进入战斗，
    /// 且不再出现战斗按钮。结果不能单独用于记录应用成功。
    pub fn normal_battle_result(
        &self,
        session: &NativeSession,
        frame: &CombatFrame,
        cancel: &AtomicBool,
    ) -> Result<Option<od_core::battle_result::NormalBattleResult>> {
        use od_core::battle_result::NormalBattleResult;
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        ensure!(
            !session.tasker.running(),
            "结算识别不能与 Pipeline 同时执行"
        );
        let completed = self.area("战斗结果-完成")?.map(i64::from);
        let retreat = self.area("战斗结果-撤退")?.map(i64::from);
        let left = completed[0].min(retreat[0]);
        let top = completed[1].min(retreat[1]);
        let roi = [
            left,
            top,
            completed[2].max(retreat[2]) - left,
            completed[3].max(retreat[3]) - top,
        ];
        let result = session.recognize_prepared(
            &frame.image,
            "OCR",
            &json!({
                "model":"ppocrv5", "roi":roi, "threshold":0.6,
                "expected":["^完成$","^撤退$"]
            }),
        )?;
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let matches = result.detail["detail"]["filtered"]
            .as_array()
            .context("普通战斗结算 OCR 缺少结果")?;
        let at = |text: &str, [left, top, right, bottom]: [i64; 4]| {
            matches.iter().any(|item| {
                let Some(rect) = item["box"].as_array().filter(|rect| rect.len() == 4) else {
                    return false;
                };
                let values: Option<Vec<i64>> = rect.iter().map(Value::as_i64).collect();
                let Some(values) = values else { return false };
                let x = values[0] + values[2] / 2;
                let y = values[1] + values[3] / 2;
                item["text"] == text && (left..right).contains(&x) && (top..bottom).contains(&y)
            })
        };
        Ok(match (at("完成", completed), at("撤退", retreat)) {
            (true, false) => Some(NormalBattleResult::Completed),
            (false, true) => Some(NormalBattleResult::Retreat),
            _ => None,
        })
    }

    pub fn new(bundle: CombatBundle) -> Self {
        Self {
            bundle,
            candidates: Vec::new(),
            stable: 0,
            last_team: Vec::new(),
            last: BTreeMap::new(),
            raw: Arc::new(Mutex::new(None)),
            quick: None,
            chain: None,
        }
    }
    pub fn cancellation_handle(&self) -> Arc<Mutex<Option<crate::RawRecognitionControl>>> {
        self.raw.clone()
    }
    pub fn reset(&mut self) {
        self.candidates.clear();
        self.last_team.clear();
        self.stable = 0;
        self.last.clear();
        self.quick = None;
        self.chain = None;
    }
    pub fn refresh_agents(&mut self) {
        self.last.remove("agents");
    }
    fn due(&mut self, name: &str, now: f64, interval: f64) -> bool {
        if self
            .last
            .get(name)
            .is_some_and(|last| now - last < interval)
        {
            return false;
        }
        self.last.insert(name.into(), now);
        true
    }
    pub(crate) fn matched(
        &self,
        session: &NativeSession,
        frame: &CombatFrame,
        key: &str,
        rect: [i32; 4],
        threshold: f64,
    ) -> Result<Option<f64>> {
        let Some(template) = self.bundle.spec.templates.get(key) else {
            return Ok(None);
        };
        let roi = od_core::Roi::from_legacy(rect).map_err(anyhow::Error::msg)?;
        let template_path = template
            .image
            .strip_prefix("image/")
            .context("战斗模板须位于 Maa image 目录")?;
        let result = session.recognize_prepared(&frame.image, "TemplateMatch", &json!({"roi":roi,"template":template_path,"threshold":threshold,"method":5,"green_mask":template.mask.is_some(),"order_by":"Score"}))?;
        Ok(result.detail["detail"]["best"]["score"]
            .as_f64()
            .filter(|_| result.detail["hit"] == true))
    }
    fn avatar(
        &self,
        session: &NativeSession,
        frame: &CombatFrame,
        prefix: &str,
        rect: [i32; 4],
        cancel: &AtomicBool,
    ) -> Result<Option<usize>> {
        let candidates: Vec<usize> = if self.stable >= 5 && !self.candidates.is_empty() {
            self.candidates.clone()
        } else {
            (0..self.bundle.spec.agents.len()).collect()
        };
        let mut best: Option<(usize, f64)> = None;
        for index in candidates {
            for template in &self.bundle.spec.agents[index].templates {
                if cancel.load(Ordering::Acquire) {
                    return Ok(None);
                }
                if let Some(score) = self.matched(
                    session,
                    frame,
                    &format!("battle/{prefix}{template}"),
                    rect,
                    0.8,
                )? && best.is_none_or(|(_, previous)| score >= previous)
                {
                    best = Some((index, score));
                }
            }
        }
        Ok(best.map(|(index, _)| index))
    }
    pub(crate) fn area(&self, name: &str) -> Result<[i32; 4]> {
        self.bundle
            .spec
            .areas
            .get(name)
            .copied()
            .context(format!("战斗区域缺失 {name}"))
    }
    pub fn observe(
        &mut self,
        session: &NativeSession,
        frame: &CombatFrame,
        now: f64,
        intervals: &BTreeMap<String, f64>,
        dodge_only: bool,
        cancel: &AtomicBool,
    ) -> Result<CombatObservation> {
        let in_battle = self
            .matched(
                session,
                frame,
                "battle/btn_normal_attack",
                self.area("按键-普通攻击")?,
                0.9,
            )?
            .is_some();
        let mut observation = CombatObservation {
            in_battle,
            ..Default::default()
        };
        if in_battle {
            observation.quick_assist = self
                .quick
                .as_ref()
                .filter(|(_, at)| now - at <= 1.0)
                .map(|(name, _)| name.clone());
        } else {
            observation.chain = self
                .chain
                .as_ref()
                .filter(|(_, at)| now - at <= 1.0)
                .map(|(chain, _)| chain.clone())
                .unwrap_or_default();
        }
        let update = |name: String, value: Option<i64>, clear: bool| StateUpdate {
            name,
            at: now,
            value,
            clear,
            add: None,
            time_add: None,
        };
        observation.states.push(update(
            "按键可用-普通攻击".into(),
            None,
            !observation.in_battle,
        ));
        if cancel.load(Ordering::Acquire) {
            return Ok(observation);
        }
        if observation.in_battle {
            if intervals.get("flash_enabled").copied().unwrap_or(1.0) > 0.0
                && self.due(
                    "flash",
                    now,
                    *intervals.get("check_dodge_interval").unwrap_or(&0.02),
                )
            {
                let control = crate::RawRecognitionControl::default();
                *self.raw.lock().expect("战斗推理取消锁失效") = Some(control.clone());
                if cancel.load(Ordering::Acquire) {
                    control.cancel()?;
                }
                let result = session.recognize_prepared_cancellable(
                    &frame.image,
                    "LegacyFlashRaw",
                    &json!({"model":crate::raw_inference::RAW_FLASH_MODEL,"threshold":0.9}),
                    &control,
                );
                *self.raw.lock().expect("战斗推理取消锁失效") = None;
                let result = match result {
                    Ok(result) => result,
                    Err(error)
                        if error
                            .downcast_ref::<crate::RawRecognitionCancelled>()
                            .is_some()
                            && cancel.load(Ordering::Acquire) =>
                    {
                        return Ok(observation);
                    }
                    Err(error) => return Err(error),
                };
                let state = match result.detail["adaptation"]["class_index"].as_i64() {
                    Some(1) => Some("闪避识别-红光"),
                    Some(2) => Some("闪避识别-黄光"),
                    _ => None,
                };
                if let Some(state) = state {
                    observation.states.push(update(state.into(), None, false));
                }
            }
            if dodge_only {
                return Ok(observation);
            }
            if self.due(
                "agents",
                now,
                *intervals.get("check_agent_interval").unwrap_or(&0.5),
            ) {
                let mut found = Vec::new();
                for (name, prefix) in [
                    ("头像-3-1", "avatar_1_"),
                    ("头像-3-2", "avatar_2_"),
                    ("头像-3-3", "avatar_2_"),
                    ("头像-2-2", "avatar_2_"),
                ] {
                    if cancel.load(Ordering::Acquire) {
                        return Ok(observation);
                    }
                    found.push(self.avatar(session, frame, prefix, self.area(name)?, cancel)?);
                }
                let team = if found[1].is_some() && found[2].is_some() {
                    found[..3].to_vec()
                } else if found[3].is_some() {
                    vec![found[0], found[3]]
                } else {
                    vec![found[0]]
                };
                observation.team = team
                    .iter()
                    .map(|index| {
                        index.map_or(String::new(), |i| self.bundle.spec.agents[i].name.clone())
                    })
                    .collect();
                if team.iter().all(Option::is_none) {
                    self.reset();
                } else {
                    self.stable = if self.last_team == team {
                        self.stable + 1
                    } else {
                        0
                    };
                    self.candidates = team.iter().flatten().copied().collect();
                    self.last_team = team.clone();
                    for (position, index) in team.iter().enumerate() {
                        let Some(index) = index else {
                            continue;
                        };
                        let agent = &self.bundle.spec.agents[*index];
                        let prefix = if position == 0 {
                            "前台-".to_string()
                        } else {
                            format!("后台-{position}-")
                        };
                        for suffix in [&agent.name, &agent.kind] {
                            observation.states.push(update(
                                format!("{prefix}{suffix}"),
                                None,
                                false,
                            ));
                        }
                        if position > 0 {
                            observation.states.push(update(
                                format!("后台-{}", agent.name),
                                None,
                                false,
                            ));
                        }
                        for (key, suffix) in [
                            ("ENERGY", "能量"),
                            ("SPECIAL", "特殊技可用"),
                            ("ULTIMATE", "终结技可用"),
                        ] {
                            let total = if team.len() == 2 || (team.len() == 1 && key == "ENERGY") {
                                2
                            } else {
                                3
                            };
                            let definition = self
                                .bundle
                                .spec
                                .common
                                .get(&format!("{key}_{total}{}", position + 1))
                                .context("公共角色状态缺失")?;
                            let value = self.agent_state(
                                session,
                                frame,
                                definition,
                                &definition.template_id,
                            )?;
                            let clear = key != "ENERGY" && value == 0;
                            observation.states.push(update(
                                format!("{prefix}{suffix}"),
                                Some(value),
                                clear,
                            ));
                            observation.states.push(update(
                                format!("{}-{suffix}", agent.name),
                                Some(value),
                                clear,
                            ));
                            if position == 0 && key != "ENERGY" {
                                observation.states.push(update(
                                    format!(
                                        "按键可用-{}",
                                        if key == "SPECIAL" {
                                            "特殊攻击"
                                        } else {
                                            "终结技"
                                        }
                                    ),
                                    Some(value),
                                    clear,
                                ));
                            }
                        }
                        for definition in &agent.states {
                            let suffix = if team.len() == 2 && position == 1 {
                                "2_2".to_string()
                            } else {
                                format!("3_{}", position + 1)
                            };
                            let value = self.agent_state(
                                session,
                                frame,
                                definition,
                                &format!("{}_{}", definition.template_id, suffix),
                            )?;
                            if value >= definition.min_value_trigger_state {
                                observation.states.push(update(
                                    definition.state_name.clone(),
                                    Some(value),
                                    definition.clear_on_zero && value == 0,
                                ));
                            }
                        }
                    }
                }
                for key in ["GUARD_BREAK", "SWITCH_BAN"] {
                    if let Some(definition) = self.bundle.spec.common.get(key) {
                        let value =
                            self.agent_state(session, frame, definition, &definition.template_id)?;
                        observation.states.push(update(
                            definition.state_name.clone(),
                            Some(value),
                            value == 0,
                        ));
                    }
                }
                let key = if self.last_team.len() == 3 {
                    "LIFE_DEDUCTION_31"
                } else {
                    "LIFE_DEDUCTION_21"
                };
                if let Some(definition) = self.bundle.spec.common.get(key) {
                    let value =
                        self.agent_state(session, frame, definition, &definition.template_id)?;
                    if value >= definition.min_value_trigger_state {
                        observation.states.push(update(
                            definition.state_name.clone(),
                            Some(value),
                            false,
                        ));
                    }
                }
            }
            if self.due(
                "quick",
                now,
                *intervals.get("check_quick_interval").unwrap_or(&0.5),
            ) && let Some(index) = self.avatar(
                session,
                frame,
                "avatar_quick_",
                self.area("按键-切换角色")?,
                cancel,
            )? {
                let agent = &self.bundle.spec.agents[index];
                observation.quick_assist = Some(agent.name.clone());
                self.quick = Some((agent.name.clone(), now));
                for name in [
                    format!("快速支援-{}", agent.name),
                    format!("快速支援-{}", agent.kind),
                    "按键可用-快速支援".into(),
                ] {
                    observation.states.push(update(name, None, false));
                }
            }
            if self.due("backup", now, 1.0) {
                let pixels = frame.image.buffer.raw_data().context("战斗帧像素缺失")?;
                let ratio = |rect: [i32; 4], test: fn([u8; 3]) -> bool| {
                    let [left, top, right, bottom] = rect;
                    let mut count = 0;
                    for y in top..bottom {
                        for x in left..right {
                            let i = ((y * 1920 + x) * 3) as usize;
                            if test(hsv([pixels[i + 2], pixels[i + 1], pixels[i]])) {
                                count += 1;
                            }
                        }
                    }
                    count as f64 / ((right - left) * (bottom - top)) as f64
                };
                let mark = self.area("按键-切换后援标记")?;
                let gray = self.area("按键-切换后援灰度区域")?;
                if ratio(mark, |v| v[1] <= 10 && v[2] <= 20) >= 0.9
                    && ratio(gray, |v| v[1] >= 40 && v[2] >= 90) >= 0.5
                    && ratio(gray, |v| v[1] <= 10 && (v[2] as i32 - 150).abs() <= 20) <= 0.2
                {
                    observation
                        .states
                        .push(update("按键可用-切换后援".into(), None, false));
                }
            }
            let target_interval = *intervals.get("target_lock_interval").unwrap_or(&1.0);
            if target_interval > 0.0
                && self.due("target", now, target_interval)
                && let Some(rect) = self
                    .bundle
                    .spec
                    .templates
                    .get("target_state/target_lock")
                    .and_then(|t| t.rect)
            {
                let pixels = frame.image.buffer.raw_data().context("战斗帧像素缺失")?;
                if crate::combat_contours::contours(
                    pixels,
                    rect,
                    [20.0, 200.0, 255.0],
                    [5.0, 55.0, 100.0],
                    true,
                )? >= 2
                {
                    observation
                        .states
                        .push(update("目标-近距离锁定".into(), None, false));
                }
            }
        } else if !dodge_only {
            let pixels = frame.image.buffer.raw_data().context("战斗帧像素缺失")?;
            if crate::combat_contours::contours(
                pixels,
                self.area("连携条")?,
                [15.0, 183.0, 237.0],
                [10.0, 10.0, 10.0],
                false,
            )? > 0
            {
                observation
                    .states
                    .push(update("连携技-准备".into(), None, false));
            }
            if self.due(
                "chain",
                now,
                *intervals.get("check_chain_interval").unwrap_or(&1.0),
            ) {
                observation.chain.clear();
                for (position, name) in ["连携技-1", "连携技-2"].into_iter().enumerate() {
                    let agent =
                        self.avatar(session, frame, "avatar_chain_", self.area(name)?, cancel)?;
                    observation
                        .chain
                        .push(agent.map(|i| self.bundle.spec.agents[i].name.clone()));
                    if let Some(index) = agent {
                        let agent = &self.bundle.spec.agents[index];
                        for suffix in [&agent.name, &agent.kind] {
                            observation.states.push(update(
                                format!("连携技-{}-{suffix}", position + 1),
                                None,
                                false,
                            ));
                        }
                    }
                }
                if observation.chain.iter().any(Option::is_some) {
                    self.chain = Some((observation.chain.clone(), now));
                    observation
                        .states
                        .push(update("按键可用-连携技".into(), None, false));
                    for (position, agent) in observation.chain.iter().enumerate() {
                        if agent.is_none() {
                            observation.states.push(update(
                                format!("连携技-{}-邦布", position + 1),
                                None,
                                false,
                            ));
                        }
                    }
                }
            }
        }
        Ok(observation)
    }

    fn agent_state(
        &self,
        session: &NativeSession,
        frame: &CombatFrame,
        definition: &AgentStateDef,
        template_id: &str,
    ) -> Result<i64> {
        let key = format!("agent_state/{template_id}");
        let Some(template) = self.bundle.spec.templates.get(&key) else {
            return Ok(0);
        };
        let Some(rect) = template.rect else {
            return Ok(0);
        };
        if matches!(
            definition.check_way.as_str(),
            "TEMPLATE_FOUND" | "TEMPLATE_NOT_FOUND"
        ) {
            let found = self
                .matched(
                    session,
                    frame,
                    &key,
                    rect,
                    definition.template_threshold.unwrap_or(0.7),
                )?
                .is_some();
            return Ok(i64::from(if definition.check_way == "TEMPLATE_FOUND" {
                found
            } else {
                !found
            }));
        }
        let pixels = frame.image.buffer.raw_data().context("战斗帧缺失像素")?;
        state_pixels(
            pixels,
            rect,
            self.bundle.masks.get(&key).map(Vec::as_slice),
            definition,
        )
    }
}

pub fn hsv([r, g, b]: [u8; 3]) -> [u8; 3] {
    let (r, g, b) = (r as i32, g as i32, b as i32);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let diff = max - min;
    if max == 0 {
        return [0, 0, 0];
    }
    let sdiv = ((255.0 * 4096.0 / max as f64).round_ties_even()) as i32;
    let saturation = (diff * sdiv + 2048) >> 12;
    if diff == 0 {
        return [0, saturation as u8, max as u8];
    }
    let hue = if max == r {
        g - b
    } else if max == g {
        b - r + 2 * diff
    } else {
        r - g + 4 * diff
    };
    let hdiv = (180.0 * 4096.0 / (6 * diff) as f64).round_ties_even() as i32;
    let h = (hue * hdiv + 2048) >> 12;
    [
        (if h < 0 { h + 180 } else { h }) as u8,
        saturation as u8,
        max as u8,
    ]
}

fn color_matches(rgb: [u8; 3], definition: &AgentStateDef) -> bool {
    if let (Some(center), Some(diff)) = (definition.hsv_color, definition.hsv_color_diff) {
        let hsv = hsv(rgb);
        let lower_h = center[0] - diff[0];
        let upper_h = center[0] + diff[0];
        let h = hsv[0] as f64;
        let hue = if lower_h < 0.0 {
            h >= ((lower_h + 180.0) as i32 as u8) as f64 || h <= (upper_h as i32 as u8) as f64
        } else if upper_h > 179.0 {
            h >= (lower_h as i32 as u8) as f64 || h <= ((upper_h - 180.0) as i32 as u8) as f64
        } else {
            h >= lower_h && h <= upper_h
        };
        return hue
            && (1..=2).all(|i| {
                hsv[i] as f64 >= (center[i] - diff[i]).clamp(0.0, 255.0)
                    && hsv[i] as f64 <= (center[i] + diff[i]).clamp(0.0, 255.0)
            });
    }
    if let (Some(lower), Some(upper)) = (&definition.lower_color, &definition.upper_color)
        && let (Some(lower), Some(upper)) = (lower.as_array(), upper.as_array())
    {
        return lower.len() == 3
            && upper.len() == 3
            && (0..3).all(|i| {
                rgb[i] as f64 >= lower[i].as_f64().unwrap_or(256.0)
                    && rgb[i] as f64 <= upper[i].as_f64().unwrap_or(-1.0)
            });
    }
    true
}

pub fn state_pixels(
    bgr: &[u8],
    rect: [i32; 4],
    mask: Option<&[u8]>,
    definition: &AgentStateDef,
) -> Result<i64> {
    od_core::Roi::from_legacy(rect).map_err(anyhow::Error::msg)?;
    ensure!(bgr.len() == 1920 * 1080 * 3, "战斗像素必须为连续 BGR8");
    let [x, y, right, bottom] = rect;
    let width = (right - x) as usize;
    let height = (bottom - y) as usize;
    ensure!(
        mask.is_none_or(|m| m.len() == width * height),
        "战斗状态掩码与区域尺寸不匹配"
    );
    let mut rgb = Vec::with_capacity(width * height);
    for row in y..bottom {
        for col in x..right {
            let i = (row * 1920 + col) as usize * 3;
            rgb.push([bgr[i + 2], bgr[i + 1], bgr[i]]);
        }
    }
    let way = definition.check_way.as_str();
    if matches!(
        way,
        "BACKGROUND_GRAY_RANGE_LENGTH" | "FOREGROUND_GRAY_RANGE_LENGTH"
    ) {
        let lower = definition
            .lower_color
            .as_ref()
            .and_then(Value::as_f64)
            .context("灰度下限缺失")?;
        let upper = definition
            .upper_color
            .as_ref()
            .and_then(Value::as_f64)
            .context("灰度上限缺失")?;
        let gray: Vec<_> = (0..width)
            .map(|col| {
                (0..height)
                    .map(|row| {
                        let [r, g, b] = rgb[row * width + col];
                        ((r as u32 * 4899 + g as u32 * 9617 + b as u32 * 1868 + 8192) >> 14) as f64
                    })
                    .sum::<f64>()
                    / height as f64
            })
            .filter(|v| {
                way != "FOREGROUND_GRAY_RANGE_LENGTH"
                    || definition
                        .split_color_range
                        .is_none_or(|[lo, hi]| *v < lo || *v > hi)
            })
            .collect();
        if gray.is_empty() {
            return Ok(0);
        }
        let yes: Vec<_> = gray
            .iter()
            .enumerate()
            .filter_map(|(i, v)| (*v >= lower && *v <= upper).then_some(i))
            .collect();
        let span = |indices: &[usize]| {
            indices
                .first()
                .zip(indices.last())
                .map_or(0, |(a, b)| b - a + 1)
        };
        let length = if way == "FOREGROUND_GRAY_RANGE_LENGTH" {
            span(&yes)
        } else {
            let no: Vec<_> = gray
                .iter()
                .enumerate()
                .filter_map(|(i, v)| (*v < lower || *v > upper).then_some(i))
                .collect();
            if yes.first().copied().unwrap_or(gray.len() + 1)
                < no.first().copied().unwrap_or(gray.len() + 1)
            {
                gray.len() - span(&yes)
            } else {
                span(&no)
            }
        };
        return Ok((length as f64
            * if way == "BACKGROUND_GRAY_RANGE_LENGTH" {
                100.0
            } else {
                definition.max_length as f64
            }
            / gray.len() as f64) as i64);
    }
    if way == "FOREGROUND_COLOR_RANGE_LENGTH" {
        let indices: Vec<_> = rgb
            .iter()
            .enumerate()
            .filter_map(|(i, p)| color_matches(*p, definition).then_some(i % width))
            .collect();
        return Ok(indices
            .iter()
            .min()
            .zip(indices.iter().max())
            .map_or(0, |(lo, hi)| {
                ((hi - lo + 1) * definition.max_length as usize / width) as i64
            }));
    }
    let mut binary = Vec::with_capacity(rgb.len());
    for (i, pixel) in rgb.iter().enumerate() {
        let pixel = if mask.is_some_and(|m| m[i] == 0) {
            [0, 0, 0]
        } else {
            *pixel
        };
        let matched = match way {
            "COLOR_CHANNEL_EQUAL_RANGE_CONNECT" => pixel[0] == pixel[1] && pixel[1] == pixel[2],
            "COLOR_CHANNEL_MAX_RANGE_EXIST" => {
                let v = *pixel.iter().max().unwrap() as f64;
                v >= definition
                    .lower_color
                    .as_ref()
                    .and_then(Value::as_f64)
                    .context("通道下限缺失")?
                    && v <= definition
                        .upper_color
                        .as_ref()
                        .and_then(Value::as_f64)
                        .context("通道上限缺失")?
            }
            "COLOR_RANGE_CONNECT" | "COLOR_RANGE_EXIST" => color_matches(pixel, definition),
            _ => anyhow::bail!("未知战斗状态算法 {way}"),
        };
        binary.push(matched);
    }
    if way == "COLOR_CHANNEL_EQUAL_RANGE_CONNECT" {
        return Ok(i64::from(
            binary.iter().filter(|v| **v).count() >= definition.connect_cnt.unwrap_or(1) as usize,
        ));
    }
    let mut dilated = vec![false; width * height];
    for row in 0..height {
        for col in 0..width {
            dilated[row * width + col] = (row.saturating_sub(1)..=row)
                .any(|y| (col.saturating_sub(1)..=col).any(|x| binary[y * width + x]));
        }
    }
    let mut count = 0;
    for index in 0..dilated.len() {
        if !dilated[index] {
            continue;
        }
        let mut stack = vec![index];
        dilated[index] = false;
        let mut area = 0;
        while let Some(i) = stack.pop() {
            area += 1;
            let row = i / width;
            let col = i % width;
            for y in row.saturating_sub(1)..=(row + 1).min(height - 1) {
                for x in col.saturating_sub(1)..=(col + 1).min(width - 1) {
                    let next = y * width + x;
                    if dilated[next] {
                        dilated[next] = false;
                        stack.push(next);
                    }
                }
            }
        }
        if area >= definition.connect_cnt.unwrap_or(1) {
            count += 1;
        }
    }
    Ok(if way.ends_with("EXIST") {
        i64::from(count > 0)
    } else {
        count
    })
}
