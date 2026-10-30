//! 迷失之地发布能力检查；仅核验固定文件，不加载 SDK 或创建游戏控制器。
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::path::Path;

pub const SCENE_DIRECTORY: &str = "generated/lost-void-scene";
pub const SCENE_MANIFEST: &str = "generated/lost-void-scene/lost_void_scene-manifest.json";

pub fn validate(resources: &Path) -> Result<Value> {
    od_core::config::reject_links(resources)?;
    let resources = resources
        .canonicalize()
        .context("迷失之地运行资源根目录缺失")?;
    let scene = maa_adapter::lost_void_scene::validate_lost_void_scene_bundle(
        &resources.join(SCENE_DIRECTORY),
    )?;
    let daily = maa_adapter::validate_basic_daily_bundle(&resources.join("generated/basic-daily"))?;
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(daily.join("basic-daily-manifest.json"))?)?;
    ensure!(
        manifest["schema_version"] == 10,
        "发布迷失之地普通选队需要 schema 10 日常资源"
    );
    // 日常包已携带与迷失之地相同的固定检测模型及标签，无需另带诊断模型目录。
    let navigation = maa_adapter::yolo::validate_challenge_detector(&daily)?;
    let combat =
        maa_adapter::combat_vision::CombatBundle::load(&resources.join("generated/combat"))?;
    Ok(
        json!({"valid":true,"accepted":false,"input_sent":false,"scene":scene,"daily":daily,"daily_schema":10,"navigation":navigation,"combat":combat.root,"protocol_version":od_protocol::PROTOCOL_VERSION}),
    )
}
