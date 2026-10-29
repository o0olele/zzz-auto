pub mod agent_goal;
pub mod agent_mission;
pub mod agent_transport;
pub mod area_patrol;
pub mod baseline;
pub mod charge_navigation;
pub mod charge_transport;
pub mod coffee;
pub mod coffee_run;
pub mod combat;
pub mod combat_source;
pub mod compendium;
pub mod config_import;
pub mod engagement_reward;
pub mod ether_synthesis;
pub mod ether_synthesis_entry;
pub mod expert_challenge;
pub mod flash_model;
pub mod inventory;
pub mod mail;
pub mod map_transport;
pub mod models;
pub mod resources;
pub mod restore_charge;
pub mod simulation;
pub mod simulation_deploy;
pub mod simulation_mission;
pub mod simulation_settlement;

use anyhow::{Context, Result};
use std::path::Path;

pub fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut data = serde_json::to_vec_pretty(value)?;
    data.push(b'\n');
    std::fs::write(path, data).with_context(|| format!("写入 {} 失败", path.display()))
}

pub fn sha256(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    Ok(hex::encode(Sha256::digest(std::fs::read(path)?)))
}
pub mod basic_daily;

pub mod charge_level;
pub mod deep_hunt;
pub mod suibian_temple;
pub mod world_patrol;
