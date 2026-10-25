//! 旧路线与地图转换；运行时不读取旧程序目录。
use anyhow::{Context, Result, ensure};
use od_core::world_patrol::*;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

fn read(root: &Path, name: &str, sources: &mut BTreeMap<String, String>) -> Result<Vec<u8>> {
    od_core::config::reject_links(&root.join(name))?;
    let bytes = crate::mail::read_within(root, name)?;
    sources.insert(name.into(), hex::encode(Sha256::digest(&bytes)));
    Ok(bytes)
}
#[derive(Deserialize)]
struct OldIcon {
    #[serde(default)]
    icon_name: String,
    template_id: String,
    lm_pos: [i32; 2],
    tp_pos: [i32; 2],
}
#[derive(Deserialize)]
struct OldRoute {
    idx: u32,
    tp_area_id: String,
    tp_name: String,
    op_list: Vec<OldOp>,
}
#[derive(Deserialize)]
struct OldOp {
    op_type: String,
    data: Vec<String>,
}
pub fn extract(reference: &Path, output: &Path) -> Result<Value> {
    let reference = reference.canonicalize()?;
    let mut sources = BTreeMap::new();
    let map: Value = serde_yaml::from_slice(&read(
        &reference,
        "assets/game_data/map_area_all.yml",
        &mut sources,
    )?)?;
    let mut catalog = PatrolCatalog {
        areas: vec![],
        routes: vec![],
        lists: vec![],
    };
    for entry in map["full_list"].as_array().context("缺少地图入口")? {
        let entry_id = entry["entry_id"].as_str().context("入口编号无效")?;
        for parent in entry["area_list"].as_array().context("区域列表无效")? {
            let parents = vec![parent.clone()];
            let areas = parent["sub_area_list"].as_array().unwrap_or(&parents);
            for area in areas {
                let mut id = area["area_id"]
                    .as_str()
                    .context("区域编号无效")?
                    .to_string();
                let parent_name = if parent["sub_area_list"].is_array() {
                    id = format!("{}_{}", parent["area_id"].as_str().unwrap(), id);
                    Some(parent["area_name"].as_str().unwrap().into())
                } else {
                    None
                };
                let relative = format!("config/world_patrol_route/system/{entry_id}/{id}");
                if !reference.join(&relative).is_dir() {
                    continue;
                }
                let map_dir = format!("assets/game_data/world_patrol/{entry_id}/{id}");
                let road = read(
                    &reference,
                    &format!("{map_dir}/road_mask.png"),
                    &mut sources,
                )?;
                let image = image::load_from_memory(&road)?;
                let icons: Vec<OldIcon> = serde_yaml::from_slice(&read(
                    &reference,
                    &format!("{map_dir}/icon.yml"),
                    &mut sources,
                )?)?;
                catalog.areas.push(PatrolArea {
                    id: id.clone(),
                    entry_id: entry_id.into(),
                    name: area["area_name"].as_str().unwrap().into(),
                    parent_name,
                    hollow: parent["is_hollow"].as_bool().unwrap_or(false),
                    width: image.width(),
                    height: image.height(),
                    icons: icons
                        .into_iter()
                        .map(|i| PatrolIcon {
                            template_id: i.template_id,
                            icon_name: i.icon_name,
                            lm_pos: PatrolPoint {
                                x: i.lm_pos[0],
                                y: i.lm_pos[1],
                            },
                            tp_pos: PatrolPoint {
                                x: i.tp_pos[0],
                                y: i.tp_pos[1],
                            },
                        })
                        .collect(),
                });
                let mut routes: Vec<_> =
                    std::fs::read_dir(reference.join(&relative))?.collect::<Result<Vec<_>, _>>()?;
                routes.sort_by_key(|e| e.file_name());
                for route in routes {
                    if route.path().extension().is_none_or(|e| e != "yml") {
                        continue;
                    }
                    let name = format!("{relative}/{}", route.file_name().to_string_lossy());
                    let route: OldRoute =
                        serde_yaml::from_slice(&read(&reference, &name, &mut sources)?)?;
                    ensure!(route.tp_area_id == id, "路线目录与区域不符");
                    let mut points = Vec::new();
                    for op in route.op_list {
                        ensure!(
                            op.op_type == "move" && op.data.len() == 2,
                            "不支持的旧路线指令"
                        );
                        points.push(PatrolPoint {
                            x: op.data[0].parse()?,
                            y: op.data[1].parse()?,
                        });
                    }
                    catalog.routes.push(PatrolRoute {
                        id: format!("{id}_{}", route.idx),
                        area_id: id.clone(),
                        tp_name: route.tp_name,
                        points,
                    });
                }
            }
        }
    }
    let mut lists: Vec<_> = std::fs::read_dir(reference.join("config/world_patrol_route_list"))?
        .collect::<Result<Vec<_>, _>>()?;
    lists.sort_by_key(|e| e.file_name());
    for entry in lists {
        if entry.path().extension().is_some_and(|e| e == "yml") {
            catalog.lists.push(serde_yaml::from_slice(&read(
                &reference,
                &format!(
                    "config/world_patrol_route_list/{}",
                    entry.file_name().to_string_lossy()
                ),
                &mut sources,
            )?)?);
        }
    }
    catalog.validate().map_err(anyhow::Error::msg)?;
    for name in ["3d_map", "normal_world", "inter_knot"] {
        read(
            &reference,
            &format!("assets/game_data/screen_info/{name}.yml"),
            &mut sources,
        )?;
    }
    let mut templates: std::collections::BTreeSet<_> = catalog
        .areas
        .iter()
        .flat_map(|a| a.icons.iter().map(|i| format!("map/{}", i.template_id)))
        .collect();
    templates.extend([
        "normal_world/map".into(),
        "normal_world/function_menu".into(),
        "normal_world/preset_config".into(),
        "normal_world_investigation/btn_area_info_close".into(),
        "map/3d_map_tp_icon_1".into(),
        "menu/back".into(),
        "menu/btn_close".into(),
    ]);
    let mut images = BTreeMap::new();
    for template in templates {
        for file in ["raw.png", "mask.png"] {
            let name = format!("assets/template/{template}/{file}");
            if reference.join(&name).is_file() {
                read(&reference, &name, &mut sources)?;
            }
        }
        let png = template_png(&reference, &template)?;
        images.insert(
            format!("image/world-patrol/{template}.png"),
            hex::encode(Sha256::digest(&png)),
        );
    }
    crate::write_json(&output.join("catalog.json"), &catalog)?;
    crate::write_json(&output.join("source-lock.json"), &sources)?;
    crate::write_json(&output.join("image-lock.json"), &images)?;
    Ok(
        json!({"routes":catalog.routes.len(),"areas":catalog.areas.len(),"lists":catalog.lists.len()}),
    )
}
fn template_png(reference: &Path, folder: &str) -> Result<Vec<u8>> {
    let mut rgb = image::load_from_memory(&crate::mail::read_within(
        reference,
        &format!("assets/template/{folder}/raw.png"),
    )?)?
    .to_rgb8();
    let mask_name = format!("assets/template/{folder}/mask.png");
    if reference.join(&mask_name).is_file() {
        let mask =
            image::load_from_memory(&crate::mail::read_within(reference, &mask_name)?)?.to_luma8();
        ensure!(mask.dimensions() == rgb.dimensions(), "模板掩码尺寸不一致");
        for (x, y, pixel) in rgb.enumerate_pixels_mut() {
            if mask.get_pixel(x, y)[0] == 0 {
                *pixel = image::Rgb([0, 255, 0]);
            }
        }
    }
    let mut png = std::io::Cursor::new(Vec::new());
    rgb.write_to(&mut png, image::ImageFormat::Png)?;
    Ok(png.into_inner())
}
pub fn build_bundle(reference: &Path, models: &Path, output: &Path) -> Result<Value> {
    let reference = reference.canonicalize()?;
    let models = models.canonicalize()?;
    let output = crate::mail::resolved_output(output)?;
    ensure!(
        !output.starts_with(&reference)
            && !output.starts_with(&models)
            && !reference.starts_with(&output)
            && !models.starts_with(&output),
        "锄大地来源与输出不能重叠"
    );
    let sources: BTreeMap<String, String> = serde_json::from_str(include_str!(
        "../../../resources/world-patrol/source-lock.json"
    ))?;
    let mut files = BTreeMap::<String, Vec<u8>>::new();
    for (name, hash) in &sources {
        od_core::config::reject_links(&reference.join(name))?;
        let bytes = crate::mail::read_within(&reference, name)?;
        ensure!(
            hex::encode(Sha256::digest(&bytes)) == *hash,
            "锄大地来源已变化：{name}"
        );
        if name.ends_with("road_mask.png") {
            let dest = name.strip_prefix("assets/game_data/world_patrol/").unwrap();
            files.insert(format!("maps/{dest}"), bytes);
        } else if name.ends_with("/raw.png") {
            let folder = name
                .strip_prefix("assets/template/")
                .unwrap()
                .trim_end_matches("/raw.png");
            let mask_name = format!("assets/template/{folder}/mask.png");
            let mut rgb = image::load_from_memory(&bytes)?.to_rgb8();
            if sources.contains_key(&mask_name) {
                let mask =
                    image::load_from_memory(&crate::mail::read_within(&reference, &mask_name)?)?
                        .to_luma8();
                ensure!(mask.dimensions() == rgb.dimensions(), "模板掩码尺寸不一致");
                for (x, y, pixel) in rgb.enumerate_pixels_mut() {
                    if mask.get_pixel(x, y)[0] == 0 {
                        *pixel = image::Rgb([0, 255, 0]);
                    }
                }
            }
            let mut png = std::io::Cursor::new(Vec::new());
            rgb.write_to(&mut png, image::ImageFormat::Png)?;
            files.insert(format!("image/world-patrol/{folder}.png"), png.into_inner());
        }
    }
    files.insert(
        "catalog.json".into(),
        include_bytes!("../../../resources/world-patrol/catalog.json").to_vec(),
    );
    files.insert(
        "pipeline/world-patrol.json".into(),
        serde_json::to_vec_pretty(&od_core::world_patrol_pipeline::pipeline())?,
    );
    let lock = crate::models::locked_models()?;
    for model in &lock
        .models
        .iter()
        .find(|m| m.id == "ppocrv5")
        .context("缺少 OCR")?
        .files
    {
        if let Some(dest) = &model.bundle_path {
            let name = format!("ppocrv5/{}", model.path);
            od_core::config::reject_links(&models.join(&name))?;
            let bytes = crate::mail::read_within(&models, &name)?;
            ensure!(
                bytes.len() as u64 == model.bytes
                    && hex::encode(Sha256::digest(&bytes)) == model.sha256,
                "OCR 校验失败"
            );
            files.insert(dest.clone(), bytes);
        }
    }
    let manifest = json!({"schema_version":1,"sdk_version":lock.sdk_version,"accepted":false,"base_size":[1920,1080],"sources":sources,"files":files.iter().map(|(path,bytes)|json!({"path":path,"bytes":bytes.len(),"sha256":hex::encode(Sha256::digest(bytes))})).collect::<Vec<_>>()});
    files.insert(
        "world-patrol-manifest.json".into(),
        serde_json::to_vec_pretty(&manifest)?,
    );
    if output.exists() {
        for entry in walkdir::WalkDir::new(&output).min_depth(1) {
            let entry = entry?;
            od_core::config::reject_links(entry.path())?;
            let name = entry
                .path()
                .strip_prefix(&output)?
                .to_string_lossy()
                .replace('\\', "/");
            ensure!(
                if entry.file_type().is_file() {
                    files.contains_key(&name)
                } else {
                    files.keys().any(|p| p.starts_with(&format!("{name}/")))
                },
                "锄大地输出含非受管文件"
            );
        }
    }
    for (name, bytes) in files {
        crate::resources::safe_relative(&name)?;
        let path = output.join(name);
        std::fs::create_dir_all(path.parent().unwrap())?;
        std::fs::write(path, bytes)?;
    }
    Ok(manifest)
}
