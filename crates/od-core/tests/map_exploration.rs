use od_core::{
    basic_daily::{DailyConfig, DailyProgress},
    map_exploration::{ExplorationProgress, ExplorationTarget},
};

#[test]
fn observations_have_priority_and_opening_cargo_rechecks_observations() {
    let mut progress = ExplorationProgress::default();
    assert_eq!(progress.target, ExplorationTarget::Observation);
    assert!(progress.confirm().is_err());
    progress.exhausted();
    assert_eq!(progress.target, ExplorationTarget::Cargo);
    progress.interaction_attempted = true;
    let before = progress.clone();
    progress.confirm().unwrap();
    before.check_update(&progress).unwrap();
    assert_eq!(progress.cargo, 1);
    assert_eq!(progress.target, ExplorationTarget::Observation);
    assert!(progress.confirm().is_err());
}

#[test]
fn scan_and_input_release_are_required_for_success() {
    let mut progress = DailyProgress::new("map_exploration", DailyConfig::default()).unwrap();
    progress.stage = "Done".into();
    progress.completion_proven = true;
    progress.returned_world = true;
    progress.inputs_released = true;
    assert!(!progress.completed());
    let exploration = progress.exploration.as_mut().unwrap();
    exploration.exhausted();
    exploration.exhausted();
    assert!(progress.completed());
    progress.inputs_released = false;
    assert!(!progress.completed());
    assert_eq!(progress.timeout_ms(), 1_800_000);
}

#[test]
fn checkpoints_reject_unproven_counts_and_regression() {
    let before = ExplorationProgress::default();
    let mut after = before.clone();
    after.observations = 1;
    assert!(before.check_update(&after).is_err());
    after.observations = 0;
    after.interaction_attempted = true;
    let mut confirmed = after.clone();
    confirmed.confirm().unwrap();
    after.check_update(&confirmed).unwrap();
    assert!(confirmed.check_update(&after).is_err());
}

#[test]
fn application_is_available_in_catalog_and_standalone_registry() {
    assert!(od_core::run_plan::TRIAL_APP_IDS.contains(&"map_exploration"));
    assert!(
        od_core::applications()
            .unwrap()
            .iter()
            .any(|a| a.id == "map_exploration" && a.default_group)
    );
}

#[test]
fn group_slot_freezes_combat_and_rejects_missing_completion_evidence() {
    let mut config = od_core::config::AccountConfig::default();
    config.instances[0].groups[0]
        .apps
        .iter_mut()
        .find(|app| app.app_id == "map_exploration")
        .unwrap()
        .enabled = true;
    let mut plan = od_core::run_plan::plan_standalone_at(
        &config,
        1,
        "one_dragon",
        "map_exploration",
        chrono::Utc::now(),
        None,
    )
    .unwrap();
    assert!(plan.executable);
    let mut slot = od_core::RunSlot::default();
    assert!(slot.begin_group_plan(plan.mail_config, &plan).is_err());
    let mut combat = config.instances[0].combat.clone();
    combat.start_delay_ms = 0;
    combat.dodge_only = false;
    plan.exploration_combat = Some(od_core::app_combat::AppCombatPlan {
        config: combat,
        script_revision: "a".repeat(64),
    });
    let record = slot.begin_group_plan(plan.mail_config, &plan).unwrap();
    slot.confirm_group_saved(&record).unwrap();
    let mut progress = record.daily[0].clone();
    let frozen = progress.app_combat.clone();
    progress.app_combat.as_mut().unwrap().script_revision = "b".repeat(64);
    assert!(
        slot.update_daily(record.id, progress.clone(), "changed".into())
            .is_err()
    );
    progress.app_combat = frozen;
    progress.stage = "Done".into();
    progress.state = od_core::mail::MailState::Succeeded;
    progress.completion_proven = true;
    progress.returned_world = true;
    progress.inputs_released = true;
    assert!(
        slot.update_daily(record.id, progress.clone(), "unproven".into())
            .is_err()
    );
    progress.exploration.as_mut().unwrap().exhausted();
    progress.exploration.as_mut().unwrap().exhausted();
    slot.update_daily(record.id, progress, "当前可见地图扫描完成".into())
        .unwrap();
}

#[test]
fn recovery_covers_interaction_map_and_movement_keys() {
    let inputs = od_core::app_combat::interface_recovery_inputs();
    for key in [70, 78, 87] {
        assert!(
            inputs
                .iter()
                .any(|i| i.mode == od_core::InputMode::Foreground
                    && i.action == od_core::InputProbeAction::Key { key })
        );
    }
}

#[test]
fn map_sweep_moves_before_scanning_and_covers_overlapping_windows() {
    let mut sweep = od_core::map_exploration::MapSweep::default();
    let mut visited = std::collections::HashSet::new();
    let mut drags = 0;
    assert!(sweep.drag().is_some());
    loop {
        while let Some((start, end)) = sweep.drag() {
            assert!((100..1380).contains(&end[0]) && (130..950).contains(&end[1]));
            sweep.applied(start, end);
            drags += 1;
            assert!(drags < 256);
        }
        assert!(visited.insert(sweep.offset));
        if !sweep.advance() {
            break;
        }
    }
    assert_eq!(visited.len(), 49);
    assert_eq!(sweep.total(), 49);
    assert!(visited.contains(&[0, 0]));
    let offset = sweep.offset;
    sweep.restart();
    assert_eq!(sweep.offset, offset);
    while let Some((start, end)) = sweep.drag() {
        sweep.applied(start, end);
    }
    assert_eq!(sweep.offset, [520, 0]);
}

#[test]
fn new_groups_show_exploration_disabled_by_default() {
    let group = od_core::config::default_group();
    let app = group
        .apps
        .iter()
        .find(|app| app.app_id == "map_exploration")
        .unwrap();
    assert!(!app.enabled);
}

#[test]
fn existing_config_exposes_exploration_without_rewriting_saved_file() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/map-exploration-config-tests")
        .join(uuid::Uuid::new_v4().to_string());
    let directory = root.join("config");
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("accounts.yml");
    let mut original = od_core::config::AccountConfig::default();
    original.revision = 27;
    let group = &mut original.instances[0].groups[0];
    group.apps.retain(|app| app.app_id != "map_exploration");
    group.apps[0].enabled = true;
    let mut second_group = group.clone();
    second_group.id = "exploration_source".into();
    original.instances[0].groups.push(second_group);
    let bytes = serde_yaml::to_string(&original).unwrap().into_bytes();
    std::fs::write(&path, &bytes).unwrap();
    let store = od_core::config::ConfigStore::new(&std::path::absolute(&root).unwrap()).unwrap();
    for _ in 0..2 {
        let config = store.read().unwrap();
        assert_eq!(config.revision, 27);
        for (before, group) in original.instances[0]
            .groups
            .iter()
            .zip(&config.instances[0].groups)
        {
            assert_eq!(&group.apps[..before.apps.len()], before.apps);
            let added: Vec<_> = group
                .apps
                .iter()
                .filter(|app| app.app_id == "map_exploration")
                .collect();
            assert_eq!(added.len(), 1);
            assert!(!added[0].enabled);
            let plan = od_core::run_plan::plan_standalone_at(
                &config,
                1,
                &group.id,
                "map_exploration",
                chrono::Utc::now(),
                None,
            )
            .unwrap();
            assert!(plan.executable);
            assert_eq!(plan.context.app_ids, ["map_exploration"]);
        }
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
    std::fs::remove_file(path).unwrap();
    std::fs::remove_dir(directory).unwrap();
    std::fs::remove_dir(root).unwrap();
}
