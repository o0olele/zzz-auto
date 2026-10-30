//! 迷失之地只读计划与藏品决策入口，不创建原生会话。
use anyhow::{Result, ensure};
use od_core::{daily::GameDay, lost_void::*};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LostVoidPreviewInput {
    pub config: LostVoidConfig,
    pub candidates: Vec<ArtifactCandidate>,
    pub count: u8,
    pub options: ArtifactSelection,
    pub day: Option<GameDay>,
    pub history: LostVoidHistory,
}
#[derive(Debug, Serialize)]
pub struct LostVoidPreview {
    pub config: LostVoidConfig,
    pub choices: Vec<ArtifactChoice>,
    pub decision: Option<LostVoidDecision>,
    pub artifact_count: usize,
    pub executable: bool,
    pub message: &'static str,
}
pub fn preview(input: LostVoidPreviewInput) -> Result<LostVoidPreview> {
    input.config.validate().map_err(anyhow::Error::msg)?;
    let choices = choose_artifacts(
        &input.config.challenge,
        &input.candidates,
        input.count,
        &input.options,
    )
    .map_err(anyhow::Error::msg)?;
    let decision = input
        .day
        .as_ref()
        .map(|day| plan(&input.config, day, &input.history))
        .transpose()
        .map_err(anyhow::Error::msg)?;
    Ok(LostVoidPreview {
        config: input.config,
        choices,
        decision,
        artifact_count: artifacts().map_err(anyhow::Error::msg)?.len(),
        executable: false,
        message: "迷失之地配置与决策预览；完整游戏流程仍在开发",
    })
}
pub fn preview_file(input: Option<&Path>) -> Result<LostVoidPreview> {
    let input = if let Some(path) = input {
        let file = std::fs::File::open(path)?;
        ensure!(
            file.metadata()?.is_file(),
            "迷失之地预览输入必须是普通 JSON 文件"
        );
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 1024 * 1024, "迷失之地预览输入超过 1 MiB");
        serde_json::from_slice(&bytes)?
    } else {
        LostVoidPreviewInput::default()
    };
    preview(input)
}
