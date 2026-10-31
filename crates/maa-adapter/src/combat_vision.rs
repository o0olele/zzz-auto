pub use crate::combat_plan::CombatRecognitionPlan;
use crate::{CombatVision, NativeSession, RecognitionImage};
use anyhow::{Context, Result, ensure};
use od_core::{
    combat_vision::{AgentStateDef, VisionSpec},
    conditional::StateUpdate,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    cell::{Ref, RefCell},
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub struct CombatBundle {
    pub root: PathBuf,
    pub spec: VisionSpec,
    pub scripts: Vec<String>,
    pub dodge_scripts: Vec<String>,
    masks: BTreeMap<String, Vec<u8>>,
    templates: BTreeMap<String, crate::CombatTemplate>,
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
        let templates = spec
            .templates
            .iter()
            .map(|(key, template)| {
                Ok((
                    key.clone(),
                    crate::CombatTemplate::load(
                        &root.join(&template.image),
                        template.mask.is_some(),
                    )?,
                ))
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            root,
            spec,
            scripts,
            dodge_scripts,
            masks,
            templates,
        })
    }

    pub fn template(&self, key: &str) -> Option<&crate::CombatTemplate> {
        self.templates.get(key)
    }
}

pub struct CombatFrame {
    pub(crate) image: RecognitionImage,
    pub captured: Instant,
    crops: RefCell<BTreeMap<[i32; 4], RecognitionImage>>,
    pub capture_timing: CombatCaptureTiming,
    performance: Option<Box<CombatFramePerformance>>,
}

pub(crate) struct CombatCapture {
    id: i64,
    captured: Instant,
    post_us: u64,
}

#[derive(Default)]
struct CombatFramePerformance {
    stages: RefCell<BTreeMap<&'static str, CombatStageTiming>>,
}

#[derive(Default, Debug, serde::Serialize)]
pub struct CombatCaptureTiming {
    pub post_us: u64,
    pub wait_us: u64,
    pub read_us: u64,
}

#[derive(Default, Clone, Debug, serde::Serialize)]
pub struct CombatStageTiming {
    pub calls: u64,
    pub total_us: u64,
}

// 只在独立战斗诊断开启时计时；作用域退出也覆盖取消和错误返回。
struct CombatStage<'a> {
    frame: &'a CombatFrame,
    name: &'static str,
    started: Option<Instant>,
}
impl Drop for CombatStage<'_> {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            self.frame
                .record_stage(self.name, started.elapsed().as_micros() as u64);
        }
    }
}
impl CombatFrame {
    pub fn enable_performance(&mut self) {
        self.performance = Some(Box::default());
    }

    pub fn performance(&self) -> Option<BTreeMap<&'static str, CombatStageTiming>> {
        self.performance.as_ref().map(|p| p.stages.borrow().clone())
    }

    fn stage(&self, name: &'static str) -> CombatStage<'_> {
        CombatStage {
            frame: self,
            name,
            started: self.performance.as_ref().map(|_| Instant::now()),
        }
    }

    fn record_stage(&self, name: &'static str, us: u64) {
        if let Some(performance) = &self.performance {
            let mut stages = performance.stages.borrow_mut();
            let stage = stages.entry(name).or_default();
            stage.calls += 1;
            stage.total_us += us;
        }
    }

    pub fn recognition_image(&self) -> &RecognitionImage {
        &self.image
    }

    pub fn age_ms(&self) -> u32 {
        self.captured.elapsed().as_millis().min(u32::MAX as u128) as u32
    }
    /// 同帧同区域只复制一次，避免每个头像模板都向 SDK 传入整张 1080p 图。
    pub fn template_crop(&self, rect: [i32; 4]) -> Result<Ref<'_, RecognitionImage>> {
        od_core::Roi::from_legacy(rect).map_err(anyhow::Error::msg)?;
        let mut cache = self.crops.borrow_mut();
        if let std::collections::btree_map::Entry::Vacant(entry) = cache.entry(rect) {
            let [x, y, right, bottom] = rect;
            let width = (right - x) as usize;
            let height = (bottom - y) as usize;
            let source = self.image.buffer.raw_data().context("战斗帧像素缺失")?;
            let mut pixels = Vec::with_capacity(width * height * 3);
            for row in y..bottom {
                let start = (row as usize * 1920 + x as usize) * 3;
                pixels.extend_from_slice(&source[start..start + width * 3]);
            }
            let mut buffer = maa_framework::buffer::MaaImageBuffer::new()?;
            buffer.set_raw_data(&pixels, width as i32, height as i32, 16)?;
            entry.insert(RecognitionImage {
                buffer,
                prepare_us: 0,
            });
        }
        drop(cache);
        Ok(Ref::map(self.crops.borrow(), |c| &c[&rect]))
    }
}
impl NativeSession {
    pub fn combat_capture_pending(&self) -> bool {
        self.combat_capture.is_some()
    }

    pub fn start_combat_frame(&mut self, cancel: &AtomicBool) -> Result<bool> {
        ensure!(self.combat_capture.is_none(), "战斗截图仍在途");
        ensure!(!self.tasker.running(), "Pipeline 运行中不能提交战斗截图");
        if cancel.load(Ordering::Acquire) {
            return Ok(false);
        }
        let captured = Instant::now();
        let id = self.controller.post_screencap()?;
        ensure!(id > 0, "Maa 拒绝战斗截图");
        self.combat_capture = Some(CombatCapture {
            id,
            captured,
            post_us: captured.elapsed().as_micros() as u64,
        });
        Ok(true)
    }

    /// 只查询状态；完成后读取唯一截图缓存，调用者不能同时追加第二次截图。
    pub fn poll_combat_frame(&mut self, cancel: &AtomicBool) -> Result<Option<CombatFrame>> {
        let Some(pending) = &self.combat_capture else {
            return Ok(None);
        };
        let status = self.controller.status(pending.id);
        ensure!(
            status != maa_framework::MaaStatus::INVALID,
            "战斗截图 ID 无效"
        );
        if !status.done() {
            return Ok(None);
        }
        let pending = self.combat_capture.take().expect("在途截图已检查");
        ensure!(
            self.controller.wait(pending.id).is_success(),
            "战斗截图失败"
        );
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let wait_us =
            (pending.captured.elapsed().as_micros() as u64).saturating_sub(pending.post_us);
        self.read_combat_frame(pending.captured, pending.post_us, wait_us)
            .map(Some)
    }

    /// 必须先释放输入；排空旧截图后才能继续或交接本控制器。
    pub fn finish_combat_capture(&mut self) -> Result<()> {
        if let Some(pending) = self.combat_capture.take() {
            ensure!(
                self.controller.wait(pending.id).is_success(),
                "战斗在途截图收尾失败"
            );
        }
        Ok(())
    }

    /// 应用开始前载入战斗资源，保留同一 Tasker、控制器与运行控制权。
    pub fn load_combat_bundle(&mut self, root: &Path) -> Result<CombatRecognizer> {
        ensure!(
            !self.tasker.running()
                && self.combat_capture.is_none()
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
        ensure!(self.combat_capture.is_none(), "异步战斗截图尚未收尾");
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let captured = Instant::now();
        let job = self.controller.post_screencap()?;
        let post_us = captured.elapsed().as_micros() as u64;
        let wait_at = Instant::now();
        ensure!(
            self.wait_input_job(job, cancel, &mut None)?.is_success(),
            "战斗截图失败"
        );
        let wait_us = wait_at.elapsed().as_micros() as u64;
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        self.read_combat_frame(captured, post_us, wait_us).map(Some)
    }

    fn read_combat_frame(
        &self,
        captured: Instant,
        post_us: u64,
        wait_us: u64,
    ) -> Result<CombatFrame> {
        let read_at = Instant::now();
        let buffer = self.controller.cached_image()?;
        ensure!(
            buffer.width() == 1920
                && buffer.height() == 1080
                && buffer.channels() == 3
                && buffer.image_type() == 16,
            "战斗截图须为连续 1080p BGR8"
        );
        Ok(CombatFrame {
            image: RecognitionImage {
                buffer,
                prepare_us: captured.elapsed().as_micros() as u64,
            },
            captured,
            crops: RefCell::new(BTreeMap::new()),
            capture_timing: CombatCaptureTiming {
                post_us,
                wait_us,
                read_us: read_at.elapsed().as_micros() as u64,
            },
            performance: None,
        })
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
#[derive(Clone)]
pub struct CombatRecognizer {
    pub bundle: Arc<CombatBundle>,
    candidates: Vec<usize>,
    stable: u32,
    team_size: usize,
    last_team: Vec<Option<usize>>,
    last: BTreeMap<String, f64>,
    raw: Arc<Mutex<Option<crate::RawRecognitionControl>>>,
    quick: Option<(String, f64)>,
    chain: Option<(Vec<Option<String>>, f64)>,
    patrol_hud_offset: Option<[i32; 2]>,
    patrol_avatar_cache: BTreeMap<[i32; 4], usize>,
    patrol_avatar_scan: BTreeMap<[i32; 4], usize>,
    patrol_portrait_template: BTreeMap<[i32; 4], (usize, String)>,
    avatar_scan: BTreeMap<(String, [i32; 4]), usize>,
    avatar_template: BTreeMap<(String, [i32; 4]), (usize, String)>,
    agent_scope: Option<Vec<usize>>,
    flash_observer: Option<FlashObserver>,
    plan: Option<Arc<CombatRecognitionPlan>>,
}
pub type FlashObserver = Arc<dyn Fn(&[StateUpdate]) + Send + Sync>;
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
            bundle: Arc::new(bundle),
            candidates: Vec::new(),
            stable: 0,
            team_size: 0,
            last_team: Vec::new(),
            last: BTreeMap::new(),
            raw: Arc::new(Mutex::new(None)),
            quick: None,
            chain: None,
            patrol_hud_offset: None,
            patrol_avatar_cache: BTreeMap::new(),
            patrol_avatar_scan: BTreeMap::new(),
            patrol_portrait_template: BTreeMap::new(),
            avatar_scan: BTreeMap::new(),
            avatar_template: BTreeMap::new(),
            agent_scope: None,
            flash_observer: None,
            plan: None,
        }
    }
    pub fn cancellation_handle(&self) -> Arc<Mutex<Option<crate::RawRecognitionControl>>> {
        self.raw.clone()
    }
    /// 独立后台识别可先发布闪光，避免等待较慢的头像和角色状态。
    pub fn set_flash_observer(&mut self, observer: Option<FlashObserver>) {
        self.flash_observer = observer;
    }
    /// 未设置时保留离线检查入口的全量识别。
    pub fn set_recognition_plan(&mut self, plan: Arc<CombatRecognitionPlan>) {
        self.plan = Some(plan);
    }
    /// 本次完整实际队伍已确认后限制正常前后台头像候选；None 恢复通用识别。
    /// 重置状态或暂停不改变冻结名单，不能用这个设置补齐未识别的成员。
    pub fn set_agent_scope(&mut self, ids: Option<&[String]>) -> Result<()> {
        let scope = ids
            .map(|ids| {
                ensure!(
                    (1..=3).contains(&ids.len()),
                    "战斗头像候选须为完整 1..3 名成员"
                );
                let mut found = Vec::new();
                for id in ids {
                    let index = self
                        .bundle
                        .spec
                        .agents
                        .iter()
                        .position(|a| &a.id == id)
                        .context("战斗头像候选不在冻结目录")?;
                    ensure!(!found.contains(&index), "战斗头像候选不能重复");
                    found.push(index);
                }
                Ok(found)
            })
            .transpose()?;
        self.agent_scope = scope;
        self.reset();
        Ok(())
    }
    pub fn reset(&mut self) {
        self.candidates.clear();
        self.last_team.clear();
        self.stable = 0;
        self.team_size = self.agent_scope.as_ref().map_or(0, Vec::len);
        self.last.clear();
        self.quick = None;
        self.chain = None;
        self.avatar_scan.clear();
        self.avatar_template.clear();
    }
    /// 仅用于锄大地：用攻击按钮的高分匹配校准 HUD，再统一平移识别帧。
    /// 不改变控制器的截图或输入坐标；战斗按键仍沿用原有绑定。
    pub fn align_patrol_frame(
        &mut self,
        session: &NativeSession,
        frame: &mut CombatFrame,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        if cancel.load(Ordering::Acquire) {
            return Ok(false);
        }
        let attack = self.area("按键-普通攻击")?;
        let template = &self.bundle.spec.templates["battle/btn_normal_attack"];
        let result = session.recognize_prepared(&frame.image, "TemplateMatch", &json!({
            "template":template.image.strip_prefix("image/").context("攻击模板路径无效")?,
            "roi":[1380,880,160,200],"threshold":0.9,"method":5,"green_mask":template.mask.is_some(),"order_by":"Score"
        }))?;
        let matched = result.detail["hit"] == true;
        if matched {
            let [x, y, w, h]: [i32; 4] =
                serde_json::from_value(result.detail["detail"]["best"]["box"].clone())?;
            let offset = [
                x + w / 2 - (attack[0] + attack[2]) / 2,
                y + h / 2 - (attack[1] + attack[3]) / 2,
            ];
            ensure!(
                offset[0].abs() <= 40 && offset[1].abs() <= 60,
                "锄大地战斗 HUD 偏移超出支持范围"
            );
            self.patrol_hud_offset = Some(offset.map(|v| if v.abs() <= 3 { 0 } else { v }));
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(false);
        }
        if let Some([dx, dy]) = self.patrol_hud_offset.filter(|offset| *offset != [0, 0]) {
            let source = frame.image.buffer.raw_data().context("战斗帧像素缺失")?;
            ensure!(source.len() == 1920 * 1080 * 3, "战斗帧须为 1080p BGR8");
            let mut aligned = vec![0; source.len()];
            let left = (-dx).max(0) as usize;
            let source_left = dx.max(0) as usize;
            let bytes = (1920 - dx.abs()) as usize * 3;
            for y in 0..1080i32 {
                let sy = y + dy;
                if !(0..1080).contains(&sy) {
                    continue;
                }
                let from = (sy as usize * 1920 + source_left) * 3;
                let to = (y as usize * 1920 + left) * 3;
                aligned[to..to + bytes].copy_from_slice(&source[from..from + bytes]);
            }
            frame.image.buffer.set_raw_data(&aligned, 1920, 1080, 16)?;
            frame.crops.get_mut().clear();
        }
        Ok(matched)
    }
    /// 模型冷启动发生在跑图前；全部步骤只读画面，取消时不投递输入。
    pub fn warm_patrol(
        &mut self,
        session: &NativeSession,
        flash: bool,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let Some(mut frame) = session.combat_frame(cancel)? else {
            return Ok(());
        };
        if flash {
            let control = crate::RawRecognitionControl::default();
            let finished = AtomicBool::new(false);
            let result = std::thread::scope(|scope| {
                let watch = control.clone();
                let ended = &finished;
                scope.spawn(move || {
                    while !ended.load(Ordering::Acquire) {
                        if cancel.load(Ordering::Acquire) {
                            let _ = watch.cancel();
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                });
                let result = session.recognize_prepared_cancellable(
                    &frame.image,
                    "LegacyFlashRaw",
                    &json!({"model":crate::raw_inference::RAW_FLASH_MODEL,"threshold":0.9}),
                    &control,
                );
                finished.store(true, Ordering::Release);
                result
            });
            match result {
                Err(error)
                    if cancel.load(Ordering::Acquire)
                        && error
                            .downcast_ref::<crate::RawRecognitionCancelled>()
                            .is_some() =>
                {
                    return Ok(());
                }
                Err(error) => return Err(error),
                _ => {}
            }
        }
        if cancel.load(Ordering::Acquire) {
            return Ok(());
        }
        self.align_patrol_frame(session, &mut frame, cancel)?;
        self.observe(
            session,
            &frame,
            0.0,
            &BTreeMap::from([("flash_enabled".into(), 0.0)]),
            false,
            cancel,
        )?;
        self.last.clear();
        Ok(())
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
        session: &impl CombatVision,
        frame: &CombatFrame,
        key: &str,
        rect: [i32; 4],
        threshold: f64,
    ) -> Result<Option<f64>> {
        let _stage = frame.stage("template");
        let Some(template) = self.bundle.template(key) else {
            return Ok(None);
        };
        Ok(session
            .match_template(frame, rect, template)?
            .filter(|score| *score > threshold))
    }
    fn avatar(
        &mut self,
        session: &impl CombatVision,
        frame: &CombatFrame,
        prefix: &str,
        rect: [i32; 4],
        cancel: &AtomicBool,
    ) -> Result<Option<usize>> {
        let _stage = frame.stage("avatar");
        let _kind = frame.stage(match prefix {
            "avatar_quick_" => "avatar_quick",
            "avatar_chain_" => "avatar_chain",
            _ => "avatar_team",
        });
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        let scoped = self.agent_scope.is_some() && matches!(prefix, "avatar_1_" | "avatar_2_");
        let patrol = self.patrol_hud_offset.is_some() && !scoped;
        if !patrol && !scoped {
            let slot = (prefix.to_string(), rect);
            if let Some((index, key)) = self.avatar_template.get(&slot)
                && self.matched(session, frame, key, rect, 0.8)?.is_some()
            {
                return Ok(Some(*index));
            }
            self.avatar_template.remove(&slot);
            // 已发现的队友先在新画面上验证，切人不需要重新遍历全角色。
            if let Some((index, key)) =
                self.avatar_candidates(session, frame, prefix, rect, cancel, &self.candidates)?
            {
                self.avatar_template.insert(slot, (index, key));
                return Ok(Some(index));
            }
            // 每槽位每轮只发现两个角色。缺失的双人槽、支援按钮或动画帧
            // 不能触发完整串行扫描，阻塞同一循环中的松键与闪避。
            let count = self.bundle.spec.agents.len();
            if count == 0 {
                return Ok(None);
            }
            let cursor = self.avatar_scan.entry(slot.clone()).or_default();
            let candidates: Vec<_> = (0..count.min(2))
                .map(|offset| (*cursor + offset) % count)
                .filter(|index| !self.candidates.contains(index))
                .collect();
            *cursor = (*cursor + 2) % count;
            let best = self.avatar_candidates(session, frame, prefix, rect, cancel, &candidates)?;
            if let Some((index, key)) = best {
                self.avatar_template.insert(slot, (index, key));
                return Ok(Some(index));
            }
            return Ok(None);
        }
        if patrol {
            if let Some((index, key)) = self.patrol_portrait_template.get(&rect)
                && self.matched(session, frame, key, rect, 0.9)?.is_some()
            {
                return Ok(Some(*index));
            }
            self.patrol_portrait_template.remove(&rect);
            let preferred: Vec<_> = self
                .patrol_avatar_cache
                .get(&rect)
                .copied()
                .into_iter()
                .chain(self.candidates.iter().copied())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            if let Some((index, key)) =
                self.avatar_candidates(session, frame, prefix, rect, cancel, &preferred)?
            {
                self.patrol_avatar_cache.insert(rect, index);
                self.patrol_portrait_template.insert(rect, (index, key));
                return Ok(Some(index));
            }
            self.patrol_avatar_cache.remove(&rect);
        }
        let candidates: Vec<usize> = if let Some(scope) = &self.agent_scope
            && matches!(prefix, "avatar_1_" | "avatar_2_")
        {
            scope.clone()
        } else if patrol {
            let count = self.bundle.spec.agents.len();
            if count == 0 {
                return Ok(None);
            }
            let cursor = self.patrol_avatar_scan.entry(rect).or_default();
            let candidates = (0..count.min(2)).map(|i| (*cursor + i) % count).collect();
            *cursor = (*cursor + 2) % count;
            candidates
        } else if self.stable >= 5 && !self.candidates.is_empty() {
            self.candidates.clone()
        } else {
            (0..self.bundle.spec.agents.len()).collect()
        };
        let best = self.avatar_candidates(session, frame, prefix, rect, cancel, &candidates)?;
        if patrol && let Some((index, key)) = &best {
            self.patrol_avatar_cache.insert(rect, *index);
            self.patrol_portrait_template
                .insert(rect, (*index, key.clone()));
        }
        Ok(best.map(|(index, _)| index))
    }
    fn avatar_candidates(
        &self,
        session: &impl CombatVision,
        frame: &CombatFrame,
        prefix: &str,
        rect: [i32; 4],
        cancel: &AtomicBool,
        candidates: &[usize],
    ) -> Result<Option<(usize, String)>> {
        let mut best: Option<(usize, f64, String)> = None;
        for &index in candidates {
            for template in &self.bundle.spec.agents[index].templates {
                if cancel.load(Ordering::Acquire) {
                    return Ok(None);
                }
                if let Some(score) = self.matched(
                    session,
                    frame,
                    &format!("battle/{prefix}{template}"),
                    rect,
                    if self.patrol_hud_offset.is_some() {
                        0.9
                    } else {
                        0.8
                    },
                )? && best
                    .as_ref()
                    .is_none_or(|(_, previous, _)| score >= *previous)
                {
                    best = Some((index, score, format!("battle/{prefix}{template}")));
                }
            }
        }
        Ok(best.map(|(index, _, key)| (index, key)))
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
        session: &impl CombatVision,
        frame: &CombatFrame,
        now: f64,
        intervals: &BTreeMap<String, f64>,
        dodge_only: bool,
        cancel: &AtomicBool,
    ) -> Result<CombatObservation> {
        let _total = frame.stage("observe");
        let plan = self.plan.clone();
        let battle_stage = frame.stage("battle_button");
        let in_battle = self
            .matched(
                session,
                frame,
                "battle/btn_normal_attack",
                self.area("按键-普通攻击")?,
                0.9,
            )?
            .is_some();
        drop(battle_stage);
        let mut observation = CombatObservation {
            in_battle,
            ..Default::default()
        };
        if in_battle && plan.as_ref().is_none_or(|p| p.quick) {
            observation.quick_assist = self
                .quick
                .as_ref()
                .filter(|(_, at)| now - at <= 1.0)
                .map(|(name, _)| name.clone());
        } else if !in_battle && plan.as_ref().is_none_or(|p| p.chain) {
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
            if plan.as_ref().is_none_or(|p| p.flash)
                && intervals.get("flash_enabled").copied().unwrap_or(1.0) > 0.0
                && self.due(
                    "flash",
                    now,
                    *intervals.get("check_dodge_interval").unwrap_or(&0.02),
                )
            {
                let _stage = frame.stage("flash");
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
                for (name, us) in [
                    ("flash_setup", result.timing.session_setup_us),
                    ("flash_prepare", result.timing.prepare_us),
                    ("flash_inference", result.timing.completion_us),
                    ("flash_details", result.timing.details_us),
                    ("flash_raw_total", result.timing.total_us),
                ] {
                    frame.record_stage(name, us);
                }
                let state = match result.detail["adaptation"]["class_index"].as_i64() {
                    Some(1) => Some("闪避识别-红光"),
                    Some(2) => Some("闪避识别-黄光"),
                    _ => None,
                };
                if let Some(state) = state {
                    observation.states.push(update(state.into(), None, false));
                    if let Some(observer) = &self.flash_observer {
                        observer(&observation.states);
                    }
                }
            }
            if dodge_only {
                return Ok(observation);
            }
            if plan.as_ref().is_none_or(|p| p.agents)
                && self.due(
                    "agents",
                    now,
                    *intervals.get("check_agent_interval").unwrap_or(&0.5),
                )
            {
                let _stage = frame.stage("agents");
                let mut found: Vec<Option<usize>> = Vec::new();
                for (name, prefix) in [
                    ("头像-3-1", "avatar_1_"),
                    ("头像-3-2", "avatar_2_"),
                    ("头像-3-3", "avatar_2_"),
                    ("头像-2-2", "avatar_2_"),
                ] {
                    if name == "头像-2-2"
                        && (self.team_size == 3 || (found[1].is_some() && found[2].is_some()))
                    {
                        found.push(None);
                        break;
                    }
                    if cancel.load(Ordering::Acquire) {
                        return Ok(observation);
                    }
                    found.push(self.avatar(session, frame, prefix, self.area(name)?, cancel)?);
                }
                let detected_size = if found[1].is_some() && found[2].is_some() {
                    3
                } else if found[3].is_some() {
                    2
                } else {
                    1
                };
                // 一帧没读到后台不能证明减员。保留已确认的槽位，缺失身份留空。
                self.team_size = self.team_size.max(detected_size);
                let team = if self.team_size == 3 {
                    found[..3].to_vec()
                } else if self.team_size == 2 {
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
                    // 保留分帧发现游标，否则始终只能扫描目录前两个角色。
                    self.last_team.clear();
                    self.stable = 0;
                    self.last.remove("agents");
                } else {
                    self.stable = if self.last_team == team {
                        self.stable + 1
                    } else {
                        0
                    };
                    if team.iter().all(Option::is_some) {
                        self.candidates = team.iter().flatten().copied().collect();
                    }
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
                        for (common_index, (key, suffix)) in [
                            ("ENERGY", "能量"),
                            ("SPECIAL", "特殊技可用"),
                            ("ULTIMATE", "终结技可用"),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            if plan
                                .as_ref()
                                .is_some_and(|p| !p.common[*index][position][common_index])
                            {
                                continue;
                            }
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
                        for (state_index, definition) in agent.states.iter().enumerate() {
                            if plan
                                .as_ref()
                                .is_some_and(|p| !p.agent_states[*index][state_index])
                            {
                                continue;
                            }
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
                // 公共前台条与技能按钮不依赖角色身份。头像仍在分批识别时，
                // 也须提供真实能量状态，让通用脚本的“未知角色”分支能够出招。
                if team.first().is_none_or(Option::is_none) {
                    let total = if team.len() == 2 { 2 } else { 3 };
                    for (common_index, (key, suffix)) in [
                        ("ENERGY", "能量"),
                        ("SPECIAL", "特殊技可用"),
                        ("ULTIMATE", "终结技可用"),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        if plan.as_ref().is_some_and(|p| !p.front_common[common_index]) {
                            continue;
                        }
                        let definition = self
                            .bundle
                            .spec
                            .common
                            .get(&format!("{key}_{total}1"))
                            .context("公共前台状态缺失")?;
                        let value =
                            self.agent_state(session, frame, definition, &definition.template_id)?;
                        let clear = key != "ENERGY" && value == 0;
                        observation.states.push(update(
                            format!("前台-{suffix}"),
                            Some(value),
                            clear,
                        ));
                        if key != "ENERGY" {
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
                }
                for key in ["GUARD_BREAK", "SWITCH_BAN"] {
                    if plan.as_ref().is_some_and(|p| !p.shared.contains(key)) {
                        continue;
                    }
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
                let key = if self.team_size == 3 {
                    "LIFE_DEDUCTION_31"
                } else {
                    "LIFE_DEDUCTION_21"
                };
                if plan.as_ref().is_none_or(|p| p.shared.contains(key))
                    && let Some(definition) = self.bundle.spec.common.get(key)
                {
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
            let quick_stage = plan
                .as_ref()
                .is_none_or(|p| p.quick)
                .then(|| frame.stage("quick_check"));
            if plan.as_ref().is_none_or(|p| p.quick)
                && self.due(
                    "quick",
                    now,
                    *intervals.get("check_quick_interval").unwrap_or(&0.5),
                )
                && let Some(index) = self.avatar(
                    session,
                    frame,
                    "avatar_quick_",
                    self.area("按键-切换角色")?,
                    cancel,
                )?
            {
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
            drop(quick_stage);
            if plan.as_ref().is_none_or(|p| p.backup) && self.due("backup", now, 1.0) {
                let _stage = frame.stage("backup");
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
            if plan.as_ref().is_none_or(|p| p.target)
                && target_interval > 0.0
                && self.due("target", now, target_interval)
                && let Some(rect) = self
                    .bundle
                    .spec
                    .templates
                    .get("target_state/target_lock")
                    .and_then(|t| t.rect)
            {
                let _stage = frame.stage("target");
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
        } else if !dodge_only && plan.as_ref().is_none_or(|p| p.chain) {
            let _stage = frame.stage("chain");
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
        session: &impl CombatVision,
        frame: &CombatFrame,
        definition: &AgentStateDef,
        template_id: &str,
    ) -> Result<i64> {
        let _stage = frame.stage("agent_state");
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
