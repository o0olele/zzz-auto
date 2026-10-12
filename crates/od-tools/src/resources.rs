use anyhow::{Context, Result, ensure};
use od_core::Roi;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::{Component, Path};

#[derive(Deserialize)]
pub struct LegacyScreen {
    pub screen_id: String,
    pub screen_name: String,
    #[serde(default)]
    pub pc_alt: bool,
    pub area_list: Vec<LegacyArea>,
}

#[derive(Deserialize)]
pub struct LegacyArea {
    pub area_name: String,
    pub pc_rect: [i32; 4],
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub template_sub_dir: String,
    #[serde(default)]
    pub template_id: String,
    pub template_match_threshold: Option<f32>,
    pub lcs_percent: Option<f32>,
    pub color_range: Option<Value>,
    #[serde(default)]
    pub id_mark: bool,
    #[serde(default)]
    pub goto_list: Vec<String>,
}

pub fn safe_relative(path: &str) -> Result<()> {
    ensure!(
        !path.is_empty()
            && Path::new(path)
                .components()
                .all(|part| matches!(part, Component::Normal(_)))
            && !path.contains(':')
            && !path.contains('\\'),
        "资源路径必须是包内相对路径"
    );
    Ok(())
}

pub fn convert_screen(source: &Path, templates: &Path, destination: &Path) -> Result<()> {
    let screen: LegacyScreen = serde_yaml::from_str(&std::fs::read_to_string(source)?)
        .with_context(|| format!("解析 {} 失败", source.display()))?;
    safe_relative(&screen.screen_id)?;
    let mut nodes = serde_json::Map::new();
    let mut report = Vec::new();
    for area in screen.area_list {
        let roi = Roi::from_legacy(area.pc_rect)
            .map_err(anyhow::Error::msg)
            .with_context(|| area.area_name.clone())?;
        let name = format!("{}.{}", screen.screen_id, area.area_name);
        ensure!(!nodes.contains_key(&name), "重复区域 {name}");
        let mut node = json!({"roi": roi, "action": "DoNothing"});
        let mut warnings = Vec::new();
        if !area.text.is_empty() {
            node["recognition"] = json!("OCR");
            node["expected"] = json!([regex::escape(&area.text)]);
            warnings.push("旧 LCS 匹配与 Maa 正则匹配不等价，需要同模型对照后验收");
        } else if !area.template_id.is_empty() {
            safe_relative(&area.template_sub_dir)?;
            safe_relative(&area.template_id)?;
            let folder = format!("{}/{}", area.template_sub_dir, area.template_id);
            let template = templates.join(&folder).join("raw.png");
            ensure!(template.is_file(), "模板不存在：{}", template.display());
            let image_name = format!("legacy/{folder}.png");
            let output = destination.join("image").join(&image_name);
            std::fs::create_dir_all(output.parent().context("模板路径无效")?)?;
            std::fs::copy(&template, &output)?;
            node["recognition"] = json!("TemplateMatch");
            node["template"] = json!([image_name]);
            node["threshold"] = json!(area.template_match_threshold.unwrap_or(0.7));
            if templates.join(folder).join("mask.png").exists() {
                warnings.push(
                    "旧模板有独立 mask；首轮保留原图，掩码转换与算法方法需验证，不能用于正式流程",
                );
            }
        } else {
            // 空区域只提供坐标，不能转换为永远命中的识别节点。
            report.push(json!({"area": area.area_name, "roi": roi, "status": "coordinate_only", "id_mark": area.id_mark, "goto": area.goto_list}));
            continue;
        }
        if area.color_range.is_some() {
            warnings.push("旧颜色过滤参数需要核对色序和阈值，尚未转换");
        }
        report.push(json!({"area": area.area_name, "roi": roi, "id_mark": area.id_mark, "goto": area.goto_list, "legacy_lcs": area.lcs_percent, "warnings": warnings, "accepted": false}));
        nodes.insert(name, node);
    }
    crate::write_json(
        &destination
            .join("pipeline")
            .join(format!("{}.json", screen.screen_id)),
        &nodes,
    )?;
    crate::write_json(
        &destination.join(format!("{}.conversion.json", screen.screen_id)),
        &json!({"source_sha256": crate::sha256(source)?, "screen": screen.screen_name, "pc_alt": screen.pc_alt, "base_size": [1920,1080], "areas": report, "accepted": false}),
    )
}
