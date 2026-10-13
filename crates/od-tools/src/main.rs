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
                od_core::AppDefinition::decl(),
                od_core::RunState::decl(),
                od_core::InputMode::decl(),
                od_core::InputProbeConfig::decl(),
                od_core::InputProbeResult::decl(),
                od_core::RunRecord::decl(),
                od_core::RunEvent::decl(),
                od_core::WindowInfo::decl(),
                od_core::SdkStatus::decl(),
                od_core::CaptureMethod::decl(),
                od_core::ScreenshotQuality::decl(),
                od_core::Snapshot::decl(),
                od_core::Screenshot::decl(),
                od_protocol::Command::decl(),
                od_protocol::Request::decl(),
                od_protocol::Payload::decl(),
                od_protocol::ApiError::decl(),
                od_protocol::Response::decl(),
                od_protocol::DesktopUpdate::decl(),
            ];
            std::fs::create_dir_all(
                output
                    .parent()
                    .ok_or_else(|| anyhow::anyhow!("类型输出路径缺少目录"))?,
            )?;
            std::fs::write(
                output,
                format!(
                    "// 由 od-tools types 生成；请修改 Rust 协议。\n{}\n",
                    declarations
                        .into_iter()
                        .map(|decl| format!("export {decl}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            )?;
            Ok(())
        }
    }
}
