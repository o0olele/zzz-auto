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
    /// 转换战斗角色资料、模板、脚本与声音模板，不执行 Python。
    CombatBundle {
        #[arg(long)]
        reference: PathBuf,
        #[arg(long, default_value = "sdk/models")]
        models: PathBuf,
        #[arg(long, default_value = "resources/generated/combat")]
        output: PathBuf,
    },
    /// 生成当前日常及情报板共用资源，实机验收单独记录。
    BasicDailyBundle {
        #[arg(long)]
        reference: PathBuf,
        #[arg(long, default_value = "sdk/models")]
        models: PathBuf,
        #[arg(long, default_value = "resources/generated/basic-daily")]
        output: PathBuf,
    },
    /// 生成活跃度奖励离线只读识别包，不包含游戏动作。
    EngagementRewardBundle {
        #[arg(long)]
        reference: PathBuf,
        #[arg(long, default_value = "sdk/models")]
        models: PathBuf,
        #[arg(long, default_value = "resources/generated/engagement-reward-preview")]
        output: PathBuf,
    },
    CoffeeRunBundle {
        #[arg(long)]
        reference: PathBuf,
        #[arg(long, default_value = "sdk/models")]
        models: PathBuf,
        #[arg(long, default_value = "resources/generated/coffee-run")]
        output: PathBuf,
    },
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
        Tool::CombatBundle {
            reference,
            models,
            output,
        } => {
            let manifest = od_tools::combat::build_bundle(&reference, &models, &output)?;
            println!("{}", serde_json::to_string_pretty(&manifest)?);
            Ok(())
        }
        Tool::EngagementRewardBundle {
            reference,
            models,
            output,
        } => {
            od_tools::engagement_reward::build_bundle(&reference, &models, &output)?;
            println!("已生成活跃度奖励只读资源：{}", output.display());
            Ok(())
        }
        Tool::BasicDailyBundle {
            reference,
            models,
            output,
        } => {
            od_tools::basic_daily::build_bundle(&reference, &models, &output)?;
            println!("已生成日常与情报板共用资源：{}", output.display());
            Ok(())
        }
        Tool::CoffeeRunBundle {
            reference,
            models,
            output,
        } => {
            od_tools::coffee_run::build_bundle(&reference, &models, &output)?;
            println!("已生成咖啡试运行资源：{}", output.display());
            Ok(())
        }
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
                od_core::combat::CombatConfig::decl(),
                od_core::combat::CombatScriptCatalog::decl(),
                od_core::combat_scripts::CombatScriptImportPreview::decl(),
                od_core::combat::CombatProgress::decl(),
                format!(
                    "const combatDefaultConfig = {} as const;",
                    serde_json::to_string(&od_core::combat::CombatConfig::default())?
                ),
                format!(
                    "const combatButtonNames = {} as const;",
                    serde_json::to_string(&od_core::combat::BUTTONS)?
                ),
                od_core::screen_samples::ScreenSampleKind::decl(),
                od_core::screen_samples::ScreenSampleSource::decl(),
                od_core::screen_samples::ScreenSamplePreview::decl(),
                od_core::screen_samples::ScreenSampleExport::decl(),
                od_core::screen_samples::ScreenSampleState::decl(),
                od_core::screen_sample_library::ScreenSampleQuery::decl(),
                od_core::screen_sample_library::ScreenSampleItem::decl(),
                od_core::screen_sample_library::ScreenSamplePage::decl(),
                od_core::screen_sample_library::ScreenSampleView::decl(),
                od_core::after_done::AfterDoneAction::decl(),
                od_core::after_done::AfterDoneState::decl(),
                od_core::after_done::AfterDoneStatus::decl(),
                od_core::daily::GameRegion::decl(),
                od_core::daily::RunPolicy::decl(),
                od_core::daily::GameDay::decl(),
                od_core::run_plan::RunContext::decl(),
                od_core::run_plan::PlanDecision::decl(),
                od_core::run_plan::AppAvailability::decl(),
                od_core::run_plan::PlannedApp::decl(),
                od_core::intel_board::IntelBoardConfig::decl(),
                od_core::intel_board::IntelHistory::decl(),
                od_core::intel_board::IntelWeek::decl(),
                od_core::intel_board::WeeklyTotals::decl(),
                od_core::intel_board::CommissionKind::decl(),
                od_core::intel_board::SettledCommission::decl(),
                od_core::intel_board::PendingCommission::decl(),
                od_core::intel_board::IntelLedger::decl(),
                od_core::battle_result::NormalBattleResult::decl(),
                od_core::run_plan::GroupPlan::decl(),
                od_core::run_history::HistoryItem::decl(),
                od_core::run_history::RunHistory::decl(),
                od_core::run_history::HistoryOutcome::decl(),
                od_core::run_history::HistoryCursor::decl(),
                od_core::run_history::HistoryQuery::decl(),
                od_core::run_history::HistoryPage::decl(),
                od_core::run_history::HistoryExport::decl(),
                od_core::config::GroupApp::decl(),
                od_core::config::GeneralSettings::decl(),
                od_core::download_proxy::DownloadProxy::decl(),
                od_core::shortcuts::ShortcutKey::decl(),
                od_core::shortcuts::ShortcutSettings::decl(),
                od_core::shortcuts::ShortcutStatus::decl(),
                od_core::config::AppGroup::decl(),
                od_core::config::AccountInstance::decl(),
                od_core::run_notifications::RunNotificationMode::decl(),
                od_core::run_notifications::RunNotificationSettings::decl(),
                od_core::run_notifications::RunNoticeLevel::decl(),
                od_core::run_notifications::RunNotice::decl(),
                od_core::run_notifications::RunNotifications::decl(),
                od_core::game_launch::GameLaunchSettings::decl(),
                od_core::game_launch::GameLaunchResult::decl(),
                od_core::config::AccountConfig::decl(),
                od_core::config::ConfigBackup::decl(),
                od_core::config::ConfigBackups::decl(),
                od_core::config::ConfigBackupPreview::decl(),
                od_core::config::ConfigRecoveryState::decl(),
                od_core::config::ConfigRecoveryCurrent::decl(),
                od_core::config::ConfigRecoveryCatalog::decl(),
                od_core::config::ConfigRecoveryPreview::decl(),
                od_core::config::ConfigRecoveryResult::decl(),
                od_core::config::ConfigRecoveryRequest::decl(),
                od_core::config::ConfigRecoveryJobState::decl(),
                od_core::config::ConfigRecoveryJob::decl(),
                od_core::config::DesktopConfigRecovery::decl(),
                od_core::config::ConfigChange::decl(),
                od_core::config_import::ImportFile::decl(),
                od_core::config_import::ImportPreview::decl(),
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
                od_core::coffee_run::CoffeeKind::decl(),
                od_core::coffee_run::CoffeeConfig::decl(),
                od_core::coffee_run::CoffeeState::decl(),
                od_core::coffee_run::CoffeePhase::decl(),
                od_core::coffee_run::CoffeeProgress::decl(),
                od_core::redemption::RedemptionConfig::decl(),
                od_core::redemption::RedemptionCode::decl(),
                od_core::redemption::RedemptionOutcome::decl(),
                od_core::redemption::RedemptionResult::decl(),
                od_core::redemption::RedemptionHistory::decl(),
                od_core::redemption::RedemptionProgress::decl(),
                od_core::dismantle::DismantleGrade::decl(),
                od_core::dismantle::DismantleConfig::decl(),
                od_core::dismantle::DismantleProgress::decl(),
                od_core::random_play::ShopLocation::decl(),
                od_core::random_play::RandomPlayConfig::decl(),
                od_core::random_play::ShopOutcome::decl(),
                od_core::random_play::RandomPlayProgress::decl(),
                od_core::predefined_teams::PredefinedTeam::decl(),
                od_core::predefined_teams::TeamConfig::decl(),
                od_core::predefined_teams::TeamMembers::decl(),
                od_core::team_check::TeamCheckProgress::decl(),
                format!(
                    "const teamDefaultConfig = {} as TeamConfig;",
                    serde_json::to_string(&od_core::predefined_teams::TeamConfig::default())?
                ),
                od_core::basic_daily::DailyConfig::decl(),
                od_core::basic_daily::DailyProgress::decl(),
                format!(
                    "const trialAppOptions = {} as const;",
                    serde_json::to_string(
                        &od_core::applications()?
                            .into_iter()
                            .filter(
                                |app| od_core::run_plan::TRIAL_APP_IDS.contains(&app.id.as_str())
                            )
                            .map(|app| serde_json::json!({"id":app.id,"label":app.name}))
                            .collect::<Vec<_>>()
                    )?
                ),
                od_core::coffee_approach::CoffeeApproachPhase::decl(),
                od_core::coffee_approach::CoffeeApproachProgress::decl(),
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
                od_core::run_logs::LogRun::decl(),
                od_core::run_logs::LogSession::decl(),
                od_core::run_logs::LogSessionInfo::decl(),
                od_core::run_logs::DesktopLogs::decl(),
                od_core::run_logs::LogExport::decl(),
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
                od_core::runtime_resources::RuntimeDownloadSource::decl(),
                od_core::runtime_resources::RuntimeDownloadProgress::decl(),
                od_core::runtime_resources::RuntimeDownloadedArchive::decl(),
                od_core::runtime_resources::DesktopRuntimeResources::decl(),
                od_core::runtime_feed::RuntimeFeedSource::decl(),
                od_core::runtime_feed::RuntimeRelease::decl(),
                od_core::runtime_feed::RuntimeReleaseOption::decl(),
                od_core::runtime_feed::RuntimeFeedCatalog::decl(),
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
                    "// 由 od-tools types 生成；请修改 Rust 协议。\n{}\nexport const generalSettingsFields = {} as const;\nexport const gameRegionOptions = {} as const;\nexport const runPolicyOptions = {} as const;\nexport const shortcutKeys = {} as const;\nexport const coffeeSettingsFields = {} as const;\nexport const coffeeDefaultConfig = {} as const;\nexport const dailySettingsFields = {} as const;\nexport const dailyDefaultConfig = {} as const;\nexport const afterDoneOptions = {} as const;\nexport const gameLaunchDefaultConfig = {} as const;\nexport const runNotificationOptions = {} as const;\nexport const screenSampleKindOptions = {} as const;\nexport const redemptionSampleCodes = {} as const;\n",
                    declarations
                        .into_iter()
                        .map(|decl| format!("export {decl}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                        .lines()
                        .map(str::trim_end)
                        .collect::<Vec<_>>()
                        .join("\n"),
                    serde_json::to_string_pretty(&od_core::config::GeneralSettings::fields())?,
                    serde_json::to_string_pretty(&od_core::daily::GameRegion::options())?,
                    serde_json::to_string_pretty(&od_core::daily::RunPolicy::options())?,
                    serde_json::to_string_pretty(&od_core::shortcuts::ShortcutKey::ALL)?,
                    serde_json::to_string_pretty(&od_core::coffee_run::CoffeeConfig::fields())?,
                    serde_json::to_string_pretty(&od_core::coffee_run::CoffeeConfig::default())?,
                    serde_json::to_string_pretty(&od_core::basic_daily::DailyConfig::fields())?,
                    serde_json::to_string_pretty(&od_core::basic_daily::DailyConfig::default())?,
                    serde_json::to_string_pretty(&od_core::after_done::AfterDoneAction::options())?,
                    serde_json::to_string_pretty(
                        &od_core::game_launch::GameLaunchSettings::default()
                    )?,
                    serde_json::to_string_pretty(
                        &od_core::run_notifications::RunNotificationMode::options()
                    )?,
                    serde_json::to_string_pretty(
                        &od_core::screen_samples::ScreenSampleKind::options()
                    )?,
                    serde_json::to_string_pretty(&od_core::redemption::samples())?
                ),
            )?;
            Ok(())
        }
    }
}
