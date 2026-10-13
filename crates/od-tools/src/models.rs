use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, path::Path};

#[derive(Debug, Deserialize, Serialize)]
pub struct ModelLock {
    pub schema_version: u32,
    pub reference_commit: String,
    pub sdk_version: String,
    pub hash_source: String,
    pub models: Vec<ModelAsset>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ModelAsset {
    pub id: String,
    pub role: String,
    pub archive: String,
    pub bytes: u64,
    pub url: String,
    pub sha256: String,
    pub files: Vec<ModelFile>,
    pub known_differences: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct ModelFile {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
    pub bundle_path: Option<String>,
}

pub fn locked_models() -> Result<ModelLock> {
    Ok(serde_json::from_str(include_str!(
        "../../../models.lock.json"
    ))?)
}

pub fn cached_files_valid(cache: &Path, model: &ModelAsset) -> bool {
    let Ok(cache) = cache.canonicalize() else {
        return false;
    };
    model.files.iter().all(|file| {
        let Ok(path) = cache.join(&model.id).join(&file.path).canonicalize() else {
            return false;
        };
        path.starts_with(&cache)
            && std::fs::metadata(&path).is_ok_and(|metadata| metadata.len() == file.bytes)
            && crate::sha256(&path).is_ok_and(|hash| hash == file.sha256)
    }) && !model.files.is_empty()
}

pub fn parse_labels(csv: &str) -> Result<Vec<String>> {
    let mut indexed = std::collections::BTreeMap::new();
    for (line_number, line) in csv.trim_start_matches('\u{feff}').lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let fields: Vec<_> = line.trim_end_matches('\r').split(',').collect();
        if line_number == 0 && fields == ["idx", "label"] {
            continue;
        }
        ensure!(
            fields.len() == 2 && !line.contains('"'),
            "标签 CSV 第 {} 行不是固定的 idx,label 格式",
            line_number + 1
        );
        let index: usize = fields[0].parse().context("标签索引不是非负整数")?;
        ensure!(index < 1024 && !fields[1].is_empty(), "标签索引或名称无效");
        ensure!(
            indexed.insert(index, fields[1].to_owned()).is_none(),
            "标签索引重复"
        );
    }
    ensure!(!indexed.is_empty(), "标签表为空");
    ensure!(
        indexed.keys().copied().eq(0..indexed.len()),
        "标签索引必须从 0 连续排列"
    );
    Ok(indexed.into_values().collect())
}

pub fn convert(source: &Path, output: &Path) -> Result<serde_json::Value> {
    let lock = locked_models()?;
    let source = source
        .canonicalize()
        .context("模型缓存不存在，先运行 scripts/setup-models.ps1")?;
    ensure!(
        !output.exists() || !output.canonicalize()?.starts_with(&source),
        "模型输出不能写入来源目录"
    );
    let mut files = Vec::new();
    let mut models = Vec::new();
    let mut destinations = BTreeSet::new();
    // 全部校验通过之后才写文件，不能将未校验模型带入生成目录。
    for model in &lock.models {
        crate::resources::safe_relative(&model.id)?;
        let mut labels = Vec::new();
        for file in &model.files {
            crate::resources::safe_relative(&file.path)?;
            let path = source
                .join(&model.id)
                .join(&file.path)
                .canonicalize()
                .with_context(|| format!("缺少模型 {} 的 {}", model.id, file.path))?;
            ensure!(path.starts_with(&source), "模型文件指向缓存之外");
            ensure!(
                std::fs::metadata(&path)?.len() == file.bytes
                    && crate::sha256(&path)? == file.sha256,
                "模型文件校验失败：{} / {}",
                model.id,
                file.path
            );
            if file.path == "labels.csv" {
                labels = parse_labels(&std::fs::read_to_string(&path)?)?;
            }
            if let Some(destination) = &file.bundle_path {
                crate::resources::safe_relative(destination)?;
                ensure!(destinations.insert(destination.clone()), "生成模型路径重复");
                files.push(serde_json::json!({"path":destination, "sha256":file.sha256, "bytes":file.bytes}));
            }
        }
        models.push(serde_json::json!({"id":model.id,"role":model.role,"labels":labels,"known_differences":model.known_differences,"legacy_compatible":false}));
    }
    std::fs::create_dir_all(output)?;
    let output = output.canonicalize()?;
    ensure!(
        !output.starts_with(&source) && !source.starts_with(&output),
        "模型来源与输出目录不能重叠"
    );
    for model in &lock.models {
        for file in &model.files {
            if let Some(destination) = &file.bundle_path {
                let destination = output.join(destination);
                std::fs::create_dir_all(destination.parent().context("生成路径缺少目录")?)?;
                ensure!(
                    destination
                        .parent()
                        .unwrap()
                        .canonicalize()?
                        .starts_with(&output),
                    "生成路径指向输出目录之外"
                );
                ensure!(
                    !destination.exists() || destination.canonicalize()?.starts_with(&output),
                    "生成文件指向输出目录之外"
                );
                std::fs::copy(source.join(&model.id).join(&file.path), &destination)?;
                ensure!(
                    crate::sha256(&destination)? == file.sha256,
                    "生成模型校验失败"
                );
            }
        }
    }
    // 资源包含一个无输入节点，保证可单独载入；诊断只调用识别接口。
    crate::write_json(
        &output.join("pipeline/model-diagnostics.json"),
        &serde_json::json!({"Diagnostics.Ready":{"recognition":"DirectHit","action":"DoNothing"}}),
    )?;
    let manifest = serde_json::json!({"schema_version":1,"sdk_version":lock.sdk_version,"reference_commit":lock.reference_commit,"files":files,"models":models,"legacy_compatible":false});
    crate::write_json(&output.join("model-manifest.json"), &manifest)?;
    Ok(manifest)
}
