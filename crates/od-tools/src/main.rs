use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use ts_rs::TS;

#[derive(Parser)]
#[command(about = "迁移清单、资源转换与协议构建工具")]
struct Args {
    #[command(subcommand)]
    command: Tool,
}

#[derive(Subcommand)]
enum Tool {
    MapBundle {
        #[arg(long)]
        reference: PathBuf,
        #[arg(long, default_value = "sdk/models")]
        models: PathBuf,
        #[arg(long, default_value = "resources/generated/map-preview")]
        output: PathBuf,
    },
    CoffeeBundle {
        #[arg(long)]
        reference: PathBuf,
        #[arg(long, default_value = "sdk/models")]
        models: PathBuf,
        #[arg(long, default_value = "resources/generated/coffee-preview")]
        output: PathBuf,
    },
    /// 只读预览旧配置；--apply 必须同时提供上次预览的 --preview-id。
    ImportConfig {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        data: PathBuf,
        #[arg(long, requires = "preview_id")]
        apply: bool,
        #[arg(long, requires = "apply")]
        preview_id: Option<String>,
    },
    MailBundle {
        #[arg(long)]
        reference: PathBuf,
        #[arg(long, default_value = "sdk/models")]
        models: PathBuf,
        #[arg(long, default_value = "resources/generated/mail")]
        output: PathBuf,
    },
    Models {
        #[arg(long, default_value = "sdk/models")]
        source: PathBuf,
        #[arg(long, default_value = "resources/generated/recognition")]
        output: PathBuf,
    },
    Baseline {
        #[arg(long)]
        reference: PathBuf,
        #[arg(long)]
        test_root: PathBuf,
    },
    Inventory {
        #[arg(long)]
        reference: PathBuf,
        #[arg(long, default_value = ".")]
        output: PathBuf,
    },
    ConvertScreen {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        templates: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    SdkManifest {
        #[arg(long)]
        directory: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    Types {
        #[arg(long, default_value = "apps/desktop/src/generated/protocol.ts")]
        output: PathBuf,
    },
}

fn main() -> Result<()> {
    match Args::parse().command {
        Tool::MapBundle {
            reference,
            models,
            output,
        } => {
            od_tools::map_transport::build_bundle(&reference, &models, &output)?;
            println!("已生成地图离线验证资源：{}", output.display());
            Ok(())
        }
        Tool::CoffeeBundle {
            reference,
            models,
            output,
        } => {
            od_tools::coffee::build_bundle(&reference, &models, &output)?;
            println!("已生成咖啡只读预览资源：{}", output.display());
            Ok(())
        }
        Tool::ImportConfig {
            source,
            data,
            apply,
            preview_id,
        } => {
            if apply {
                let config = od_tools::config_import::apply(
                    &source,
                    &data,
                    preview_id.as_deref().expect("clap 已检查预览指纹"),
                )?;
                println!("{}", serde_json::to_string_pretty(&config)?);
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&od_tools::config_import::preview(
                        &source, &data
                    )?)?
                );
            }
            Ok(())
        }
        Tool::MailBundle {
            reference,
            models,
            output,
        } => {
            let manifest = od_tools::mail::build_bundle(&reference, &models, &output)?;
            println!(
                "邮件资源包已生成：{}（{} 个文件，待实机验收）",
                output.display(),
                manifest["files"].as_array().map_or(0, Vec::len)
            );
            Ok(())
        }
        Tool::Models { source, output } => {
            let manifest = od_tools::models::convert(&source, &output)?;
            println!(
                "已校验并转换 {} 个模型；兼容差异保留在 model-manifest.json",
                manifest["models"].as_array().map_or(0, Vec::len)
            );
            Ok(())
        }
        Tool::Baseline {
            reference,
            test_root,
        } => od_tools::baseline::prepare(&reference, &test_root),
        Tool::Inventory { reference, output } => od_tools::inventory::scan(&reference, &output),
        Tool::ConvertScreen {
            source,
            templates,
            output,
        } => od_tools::resources::convert_screen(&source, &templates, &output),
        Tool::SdkManifest { directory, output } => {
            od_tools::inventory::sdk_manifest(&directory, &output)
        }
        Tool::Types { output } => {
            let declarations = [
                od_core::daily::GameRegion::decl(),
                od_core::daily::RunPolicy::decl(),
                od_core::daily::GameDay::decl(),
                od_core::run_plan::RunContext::decl(),
                od_core::run_plan::PlanDecision::decl(),
                od_core::run_plan::AppAvailability::decl(),
                od_core::run_plan::PlannedApp::decl(),
                od_core::run_plan::GroupPlan::decl(),
                od_core::run_history::HistoryItem::decl(),
                od_core::run_history::RunHistory::decl(),
                od_core::config::GroupApp::decl(),
                od_core::config::GeneralSettings::decl(),
                od_core::config::AppGroup::decl(),
                od_core::config::AccountInstance::decl(),
                od_core::config::AccountConfig::decl(),
                od_core::config::ConfigChange::decl(),
                od_core::AppDefinition::decl(),
                od_core::RunState::decl(),
                od_core::group_run::GroupStepState::decl(),
                od_core::group_run::GroupStepProgress::decl(),
                od_core::group_run::GroupProgress::decl(),
                od_core::mail::MailConfig::decl(),
                od_core::mail::MailState::decl(),
                od_core::mail::MailPhase::decl(),
                od_core::mail::MailProgress::decl(),
                od_core::map_transport::MapConfig::decl(),
                od_core::map_transport::MapState::decl(),
                od_core::map_transport::MapPhase::decl(),
                od_core::map_transport::MapProgress::decl(),
                od_core::InputMode::decl(),
                od_core::InputSide::decl(),
                od_core::MouseButton::decl(),
                od_core::InputProbeAction::decl(),
                od_core::InputProbeConfig::decl(),
                od_core::InputProbeResult::decl(),
                od_core::InputPipelineConfig::decl(),
                od_core::PipelineEnd::decl(),
                od_core::PipelineProbeResult::decl(),
                od_core::InputPipelineResult::decl(),
                od_core::RunRecord::decl(),
                od_core::RunEvent::decl(),
                od_core::WindowInfo::decl(),
                od_core::KeyboardPermission::decl(),
                od_core::SdkStatus::decl(),
                od_core::CaptureMethod::decl(),
                od_core::ScreenshotQuality::decl(),
                od_core::Snapshot::decl(),
                od_core::model_resources::CacheState::decl(),
                od_core::model_resources::ModelCacheStatus::decl(),
                od_core::model_resources::PreparedModel::decl(),
                od_core::model_resources::ModelProgress::decl(),
                od_core::model_resources::ResourceJobKind::decl(),
                od_core::model_resources::ResourceJobState::decl(),
                od_core::model_resources::ResourceJob::decl(),
                od_core::model_resources::ResourceModel::decl(),
                od_core::model_resources::ModelResources::decl(),
                od_core::runtime_resources::ResourceSelection::decl(),
                od_core::runtime_resources::InstalledResources::decl(),
                od_core::runtime_resources::RuntimeResourceCatalog::decl(),
                od_core::runtime_resources::RuntimeResourceRequest::decl(),
                od_core::runtime_resources::RuntimeResourceJobState::decl(),
                od_core::runtime_resources::RuntimeResourceJob::decl(),
                od_core::runtime_resources::DesktopRuntimeResources::decl(),
                od_core::Screenshot::decl(),
                od_protocol::Command::decl(),
                od_protocol::Request::decl(),
                od_protocol::Payload::decl(),
                od_protocol::ApiError::decl(),
                od_protocol::Response::decl(),
                od_protocol::DesktopUpdate::decl(),
                od_protocol::EnginePrivilege::decl(),
            ];
            std::fs::create_dir_all(
                output
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("类型输出路径缺少目录"))?,
            )?;
            std::fs::write(
                output,
                format!(
                    "// 由 od-tools types 生成；请修改 Rust 协议。\n{}\nexport const generalSettingsFields = {} as const;\nexport const gameRegionOptions = {} as const;\nexport const runPolicyOptions = {} as const;\n",
                    declarations
                        .into_iter()
                        .map(|decl| format!("export {decl}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    serde_json::to_string_pretty(&od_core::config::GeneralSettings::fields())?,
                    serde_json::to_string_pretty(&od_core::daily::GameRegion::options())?,
                    serde_json::to_string_pretty(&od_core::daily::RunPolicy::options())?
                ),
            )?;
            Ok(())
        }
    }
}
