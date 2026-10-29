//! 复用固定快捷手册包和 Maa OCR，仅读取同一帧资源，不投递游戏输入。
use crate::{NativeSession, RecognitionImage, suibian_adventure::words};
use anyhow::{Result, ensure};
use od_core::{
    charge_resources::{
        BAR_ROI, ChargeResourceObservation, DOUBLE_COUNT_ROI, DOUBLE_LABEL_ROI, ResourceWord,
        parse_bar, resolve_double,
    },
    compendium::{TransportRequest, TransportView},
};
use serde_json::json;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

impl NativeSession {
    /// 供体力父流程读取实时资源；值必须来自同一张仍然新鲜的原生截图。
    pub fn read_charge_resources(
        &self,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<ChargeResourceObservation>> {
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) || timeout.is_zero() {
            return Ok(None);
        }
        let Some(frame) = self.navigation_frame(cancel)? else {
            return Ok(None);
        };
        let result = self.inspect_charge_resources(
            &frame.image,
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        if cancel.load(Ordering::Acquire)
            || started.elapsed() >= timeout
            || frame.age() > Duration::from_millis(1500)
        {
            return Ok(None);
        }
        Ok(result)
    }
    pub fn inspect_charge_resources(
        &self,
        frame: &RecognitionImage,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Option<ChargeResourceObservation>> {
        let started = Instant::now();
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        self.require_compendium()?;
        ensure!(started.elapsed() < timeout, "体力资源识别超时");
        let context = self.inspect_compendium(
            frame,
            &TransportRequest {
                mission_type: "基础材料".into(),
                timeout_ms: 30_000,
            },
            cancel,
            timeout.saturating_sub(started.elapsed()),
        )?;
        let mut observation = ChargeResourceObservation {
            handbook: false,
            training: false,
            resources: None,
            double_left: None,
            issue: None,
        };
        let Some(TransportView::Handbook {
            training,
            simulation_category,
            types,
            target_go,
        }) = context
        else {
            if cancel.load(Ordering::Acquire) {
                return Ok(None);
            }
            observation.issue = Some("当前帧没有无遮挡的快捷手册上下文".into());
            return Ok(Some(observation));
        };
        observation.handbook = true;
        observation.training = training;
        let read_detail = |roi, white, only_rec| -> Result<_> {
            ensure!(started.elapsed() < timeout, "体力资源识别超时");
            ensure!(!cancel.load(Ordering::Acquire), "体力资源识别取消");
            let mut params =
                json!({"model":"ppocrv5","roi":roi,"threshold":0.6,"only_rec":only_rec});
            if white {
                params["color_range"] = json!([[208, 208, 208], [255, 255, 255]]);
            }
            let result = self.recognize_prepared(frame, "OcrText", &params)?.detail;
            ensure!(started.elapsed() < timeout, "体力资源识别超时");
            Ok(result)
        };
        let read = |roi, white| words(&read_detail(roi, white, false)?);
        let result = (|| -> Result<()> {
            let parse = |bar: Vec<crate::suibian_adventure::Word>| {
                parse_bar(
                    &bar.into_iter()
                        .map(|w| ResourceWord {
                            text: w.text,
                            rect: w.rect,
                        })
                        .collect::<Vec<_>>(),
                )
            };
            let white = parse(read(BAR_ROI, true)?);
            // 同一帧完整读两次：白字过滤可能使单个“4”等细字断裂；原色结果须独立提供三项。
            let plain_detail = read_detail(BAR_ROI, false, false)?;
            let plain = parse(words(&plain_detail)?);
            let resolved = match (white, plain) {
                (Ok(a), Ok(b)) if a != b => Err("同帧两种资源读取结果冲突".to_owned()),
                (Ok(resources), _) | (_, Ok(resources)) => Ok(resources),
                (Err(a), Err(b)) => {
                    // 检测器可能只框住细字的一部分，沿同帧检测框扩边单行复读，保持置信度要求。
                    let mut raw = plain_detail;
                    raw["detail"]["filtered"] = raw["detail"]["all"].clone();
                    let mut recovered = Vec::new();
                    for mut word in words(&raw)? {
                        let [x, y, w, h] = word.rect;
                        if [1393, 1568, 1705]
                            .iter()
                            .all(|anchor| (x + w / 2 - anchor).abs() > 90)
                            || w > 210
                        {
                            continue;
                        }
                        let left = (x - 12).max(1220);
                        let top = (y - 12).max(35);
                        let right = (x + w + 12).min(1770);
                        let bottom = (y + h + 12).min(110);
                        let reread = words(&read_detail(
                            [left, top, right - left, bottom - top],
                            false,
                            true,
                        )?)?;
                        if reread.len() == 1 {
                            word.text = reread[0].text.clone();
                            recovered.push(word);
                        }
                    }
                    parse(recovered)
                        .map_err(|_| format!("白字：{a}；原色：{b}；单行复读未获得完整资源"))
                }
            };
            match resolved {
                Ok(resources) => observation.resources = Some(resources),
                Err(issue) => observation.issue = Some(issue),
            }
            // 没有活动的真实训练页还缺素材，因此不通过 OCR 的“未命中”推断零次。
            if observation.resources.is_some()
                && training
                && simulation_category
                && target_go
                && types.iter().any(|t| t == "基础材料")
            {
                let categories = read([164, 287, 265, 583], false)?;
                let selected: Vec<_> = categories
                    .iter()
                    .filter(|w| w.text == "实战模拟室")
                    .collect();
                if selected.len() != 1 {
                    return Ok(());
                }
                let [x, y, w, h] = selected[0].rect;
                if x < 164
                    || y < 287
                    || w <= 0
                    || h <= 0
                    || x.checked_add(w).is_none_or(|right| right > 429)
                    || y.checked_add(h).is_none_or(|bottom| bottom > 870)
                {
                    return Ok(());
                }
                // 与三类训练传送一致：原色文字定位，同位置确认蓝底，避免读到未选中分类。
                let left = (x - 12).max(164);
                let top = (y - 10).max(287);
                let width = (x + w + 12).min(429) - left;
                let height = (y + h + 10).min(870) - top;
                let color = self.recognize_prepared(
                    frame,
                    "ColorMatch",
                    &json!({"roi":[left,top,width,height],"method":4,
                        "lower":[0,70,180],"upper":[100,220,255],"count":(width*height/5).max(1)}),
                )?;
                ensure!(started.elapsed() < timeout, "体力资源识别超时");
                ensure!(!cancel.load(Ordering::Acquire), "体力资源识别取消");
                if color.detail["hit"] != true {
                    return Ok(());
                }
                let convert = |words: Vec<crate::suibian_adventure::Word>| {
                    words
                        .into_iter()
                        .map(|w| ResourceWord {
                            text: w.text,
                            rect: w.rect,
                        })
                        .collect::<Vec<_>>()
                };
                let title = convert(read(DOUBLE_LABEL_ROI, false)?);
                // 参考次数区使用单行识别；检测模式会将细斜杠与数字拆成多个框。
                let count = convert(words(&read_detail(DOUBLE_COUNT_ROI, false, true)?)?);
                observation.double_left = resolve_double(&title, &count);
            }
            Ok(())
        })();
        if cancel.load(Ordering::Acquire) {
            return Ok(None);
        }
        result?;
        Ok(Some(observation))
    }
}
