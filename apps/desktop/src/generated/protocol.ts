// 由 od-tools types 生成；请修改 Rust 协议。
export type AfterDoneAction = "none" | "close_game" | "shutdown" | "hibernate" | "sleep";
export type AfterDoneState = "watching" | "counting_down" | "executing" | "succeeded" | "cancelled" | "failed";
export type AfterDoneStatus = { id: string, run_id: string, action: AfterDoneAction, state: AfterDoneState, remaining_seconds: number, message: string, };
export type GameRegion = "cn" | "cn_b" | "us" | "eu" | "asia" | "twhkmo";
export type RunPolicy = "always" | "skip_daily_success";
export type GameDay = { region: GameRegion, date: string, };
export type RunContext = { instance_id: number, instance_name: string, group_id: string, group_name?: string, config_revision: number, app_ids: Array<string>, game_day?: GameDay, };
export type PlanDecision = "disabled" | "run" | "already_done" | "blocked";
export type AppAvailability = "mail_trial" | "coffee_trial" | "unmigrated" | "unknown";
export type PlannedApp = { app_id: string, name: string, enabled: boolean, availability: AppAvailability, decision: PlanDecision, completed_run_id: string | null, };
export type GroupPlan = { after_done: AfterDoneAction, context: RunContext, mail_config: MailConfig, coffee_config: CoffeeConfig, items: Array<PlannedApp>, blockers: Array<string>, executable: boolean, all_done: boolean, game_day: GameDay, run_policy: RunPolicy, warnings: Array<string>, valid_until: string, };
export type HistoryItem = { record: RunRecord, interrupted: boolean, };
export type RunHistory = { instance_id: number, items: Array<HistoryItem>, unreadable: number, has_more: boolean, };
export type HistoryOutcome = "active" | "succeeded" | "failed" | "timed_out" | "stopped" | "interrupted";
export type HistoryCursor = { started_at: string, id: string, };
export type HistoryQuery = { group_id: string | null, app_id: string | null, game_day: string | null, outcome: HistoryOutcome | null, before: HistoryCursor | null, };
export type HistoryPage = { instance_id: number, query: HistoryQuery, items: Array<HistoryItem>, matching: number, unreadable: number, next: HistoryCursor | null, content_id: string, };
export type HistoryExport = { path: string, instance_id: number, record_count: number, content_id: string, };
export type GroupApp = { app_id: string, enabled: boolean, };
export type GeneralSettings = { after_done: AfterDoneAction, capture_method: CaptureMethod, mail: MailConfig, shortcuts: ShortcutSettings, };
export type ShortcutKey = "f1" | "f2" | "f3" | "f4" | "f5" | "f6" | "f7" | "f8" | "f9" | "f10" | "f11";
export type ShortcutSettings = { pause_resume: ShortcutKey | null, stop: ShortcutKey | null, };
export type ShortcutStatus = { active: ShortcutSettings | null, registration_error: string | null, action_error: string | null, };
export type AppGroup = { id: string, name?: string, run_policy: RunPolicy, coffee: CoffeeConfig, apps: Array<GroupApp>, };
export type AccountInstance = { id: number, name: string, active_in_od: boolean, game_region: GameRegion, game_launch?: GameLaunchSettings, run_notifications?: RunNotificationSettings, groups: Array<AppGroup>, };
export type RunNotificationMode = "off" | "errors" | "all";
export type RunNotificationSettings = { mode: RunNotificationMode, };
export type RunNoticeLevel = "success" | "info" | "warning" | "error";
export type RunNotice = { id: string, session_id: string, run_id: string, instance_id: number, instance_name: string, group_name: string, at: string, level: RunNoticeLevel, title: string, message: string, read: boolean, };
export type RunNotifications = { revision: number, items: Array<RunNotice>, dropped: number, };
export type GameLaunchSettings = { executable: string, use_arguments: boolean, fullscreen: boolean, borderless: boolean, monitor: number, };
export type GameLaunchResult = { instance_id: number, pid: number, };
export type AccountConfig = { schema: number, revision: number, general: GeneralSettings, active_instance: number, instances: Array<AccountInstance>, import_id: string | null, reserved_instance_ids?: Array<number>, };
export type ConfigBackup = { id: string, revision: number, bytes: number, modified_at: string | null, };
export type ConfigBackups = { revision: number, items: Array<ConfigBackup>, omitted: number, unavailable: number, };
export type ConfigBackupPreview = { id: string, preview_id: string, current: AccountConfig, backup: AccountConfig, };
export type ConfigRecoveryState = "missing" | "damaged" | "healthy" | "unsupported";
export type ConfigRecoveryCurrent = { state: ConfigRecoveryState, bytes: number | null, sha256: string | null, };
export type ConfigRecoveryCatalog = { data_directory: string, current: ConfigRecoveryCurrent, backups: Array<ConfigBackup>, omitted: number, unavailable: number, };
export type ConfigRecoveryPreview = { data_directory: string, id: string, preview_id: string, current: ConfigRecoveryCurrent, recovered: AccountConfig, unreadable_backups: number, warnings: Array<string>, };
export type ConfigRecoveryResult = { config: AccountConfig, preserved_file: string | null, recovered_backup: string, };
export type ConfigRecoveryRequest = { "type": "list" } | { "type": "preview", id: string, } | { "type": "restore", id: string, preview_id: string, confirm: boolean, };
export type ConfigRecoveryJobState = "running" | "succeeded" | "failed";
export type ConfigRecoveryJob = { id: string, request: ConfigRecoveryRequest, state: ConfigRecoveryJobState, phase: string, reconnected: boolean, reconnect_error: string | null, error: string | null, error_code: string | null, };
export type DesktopConfigRecovery = { job: ConfigRecoveryJob | null, catalog: ConfigRecoveryCatalog | null, preview: ConfigRecoveryPreview | null, restored: ConfigRecoveryResult | null, };
export type ConfigChange = { "type": "save_run_notifications", instance_id: number, settings: RunNotificationSettings, } | { "type": "save_game_launch", instance_id: number, settings: GameLaunchSettings, } | { "type": "set_after_done", action: AfterDoneAction, } | { "type": "save_coffee", instance_id: number, group_id: string, settings: CoffeeConfig, } | { "type": "set_game_region", instance_id: number, game_region: GameRegion, } | { "type": "set_run_policy", instance_id: number, group_id: string, run_policy: RunPolicy, } | { "type": "save_general", settings: GeneralSettings, } | { "type": "create_instance", name: string, } | { "type": "copy_instance", instance_id: number, name: string, } | { "type": "delete_instance", instance_id: number, confirm: boolean, } | { "type": "reorder_instances", instance_ids: Array<number>, } | { "type": "reorder_groups", instance_id: number, group_ids: Array<string>, } | { "type": "update_instance", instance_id: number, name: string, active_in_od: boolean, } | { "type": "switch_instance", instance_id: number, } | { "type": "save_group", instance_id: number, group_id: string, apps: Array<GroupApp>, } | { "type": "create_group", instance_id: number, name: string, source_group_id: string | null, } | { "type": "rename_group", instance_id: number, group_id: string, name: string, } | { "type": "delete_group", instance_id: number, group_id: string, };
export type ImportFile = { path: string, sha256: string, disposition: string, };
export type ImportPreview = { preview_id: string, config: AccountConfig, files: Array<ImportFile>, warnings: Array<string>, };
export type AppDefinition = { id: string, name: string, execution: string, reason: string, factory: string, configs: Array<string>, settings: Array<string>, run_records: Array<string>, source_files: Array<string>, priority: number, default_group: boolean, implemented: boolean, };
export type RunState = "running" | "pausing" | "paused" | "stopping" | "succeeded" | "failed" | "stopped";
export type GroupStepState = "pending" | "running" | "skipped" | "succeeded" | "failed" | "stopped";
export type GroupStepProgress = { app_id: string, state: GroupStepState, inputs_released: boolean, started_at: string | null, finished_at: string | null, };
export type GroupProgress = { checkpoint: number, state: RunState, current_index: number | null, steps: Array<GroupStepProgress>, };
export type MailConfig = { timeout_ms: number, start_delay_ms: number, };
export type MailState = "running" | "paused" | "succeeded" | "failed" | "stopped" | "timed_out";
export type MailPhase = "navigate" | "inspect" | "claim" | "after_claim" | "confirm" | "after_confirm" | "return_list" | "return_world" | "done";
export type MailProgress = { config: MailConfig, state: MailState, phase: MailPhase, claim_attempted: boolean, reward_observed: boolean, inputs_released: boolean, active_ms: number, };
export type MapConfig = { timeout_ms: number, start_delay_ms: number, };
export type MapState = "running" | "paused" | "succeeded" | "failed" | "stopped" | "timed_out";
export type MapPhase = "inspect" | "select_area" | "select_point" | "inspect_dialog" | "confirm" | "wait_arrival" | "done";
export type MapProgress = { config: MapConfig, state: MapState, phase: MapPhase, point_attempted: boolean, confirm_attempted: boolean, inputs_released: boolean, active_ms: number, };
export type CoffeeKind = "tinman" | "espresso";
export type CoffeeConfig = { kind: CoffeeKind, timeout_ms: number, start_delay_ms: number, };
export type CoffeeState = "running" | "paused" | "succeeded" | "failed" | "stopped" | "timed_out";
export type CoffeePhase = "inspect" | "order" | "wait_energy" | "confirm_energy" | "after_energy" | "confirm_buff" | "done";
export type CoffeeProgress = { config: CoffeeConfig, state: CoffeeState, phase: CoffeePhase, approach?: CoffeeApproachProgress, order_attempted: boolean, energy_seen: boolean, inputs_released: boolean, active_ms: number, };
export type CoffeeApproachPhase = "inspect_world" | "open_map" | "wait_map" | "select_area" | "select_point" | "wait_dialog" | "confirm" | "wait_arrival" | "interact" | "wait_dialogue" | "advance_dialogue" | "done";
export type CoffeeApproachProgress = { phase: CoffeeApproachPhase, dialogue_advances: number, };
export type InputMode = "foreground" | "foreground_legacy" | "background" | "background_post" | "xbox" | "ds4";
export type InputSide = "left" | "right";
export type MouseButton = "left" | "right" | "middle" | "x1" | "x2";
export type InputProbeAction = { "type": "key", key: number, } | { "type": "mouse_button", button: MouseButton, x: number, y: number, } | { "type": "stick", stick: InputSide, x: number, y: number, } | { "type": "trigger", trigger: InputSide, value: number, };
export type InputProbeConfig = { mode: InputMode, action: InputProbeAction, hold_ms: number, start_delay_ms: number, };
export type InputProbeResult = { config: InputProbeConfig, pressed: boolean, released: boolean, stopped: boolean, enqueue_us: number, key_down_ms: number, held_ms: number, release_ms: number, stop_ms: number | null, elapsed_ms: number, };
export type InputPipelineConfig = { input: InputProbeConfig, timeout_ms: number, };
export type PipelineEnd = "succeeded" | "failed" | "cancelled" | "timed_out";
export type PipelineProbeResult = { sdk_version: string, entry: string, end: PipelineEnd, native_status: string, stop_status: string | null, enqueue_us: number, elapsed_ms: number, release_ms: number, stop_ms: number | null, released: boolean, };
export type InputPipelineResult = { config: InputPipelineConfig, pipeline: PipelineProbeResult, };
export type RunRecord = { id: string, app_id: string, state: RunState, started_at: string, finished_at: string | null, message: string, context?: RunContext, input_probe: InputProbeResult | null, input_pipeline: InputPipelineResult | null, mail: MailProgress | null, map?: MapProgress, coffee?: CoffeeProgress, group?: GroupProgress, };
export type RunEvent = { sequence: number, run_id: string | null, at: string, level: string, message: string, };
export type LogRun = { id: string, app_id: string, state: RunState, };
export type LogSession = { id: string, engine_version: string, captured_at: string, event_sequence: number, dropped_events: number, omitted_events: number, truncated_events: number, run: LogRun | null, events: Array<RunEvent>, };
export type LogSessionInfo = { id: string, captured_at: string, event_count: number, };
export type DesktopLogs = { sessions: Array<LogSessionInfo>, selected: LogSession | null, discarded_sessions: number, };
export type LogExport = { path: string, session_id: string, event_count: number, };
export type WindowInfo = { hwnd: string, pid: number, title: string, executable: string, width: number, height: number, keyboard_permission: KeyboardPermission | null, };
export type KeyboardPermission = { engine_integrity: number, game_integrity: number, compatible: boolean, };
export type SdkStatus = { ready: boolean, version: string | null, expected_version: string, binding_version: string, directory: string, message: string, };
export type CaptureMethod = "frame_pool" | "print_window" | "dxgi" | "screen_dc" | "gdi";
export type ScreenshotQuality = { mean_luma: number, dark_pixel_ratio: number, near_black: boolean, };
export type Snapshot = { record_error?: string, engine_version: string, sdk: SdkStatus, window: WindowInfo | null, capture_method: CaptureMethod | null, run: RunRecord | null, events: Array<RunEvent>, event_sequence: number, dropped_events: number, };
export type CacheState = "missing" | "invalid" | "valid";
export type ModelCacheStatus = { id: string, archive: CacheState, files: CacheState, };
export type PreparedModel = { id: string, reused: boolean, downloaded: boolean, };
export type ModelProgress = { id: string, phase: string, completed_bytes: number, total_bytes: number, };
export type ResourceJobKind = "check" | "prepare";
export type ResourceJobState = "running" | "cancelling" | "succeeded" | "failed" | "cancelled";
export type ResourceJob = { id: string, kind: ResourceJobKind, state: ResourceJobState, models: Array<string>, offline: boolean, progress: ModelProgress | null, prepared: Array<PreparedModel>, error: string | null, };
export type ResourceModel = { id: string, name: string, archive_bytes: number, archive: CacheState | null, files: CacheState | null, };
export type ModelResources = { cache_directory: string, models: Array<ResourceModel>, job: ResourceJob | null, };
export type ResourceSelection = { schema_version: number, current: string | null, previous: string | null, can_rollback: boolean, };
export type InstalledResources = { id: string, version: string | null, compatible: boolean, error: string | null, };
export type RuntimeResourceCatalog = { revision: string, selection: ResourceSelection, directory: string, versions: Array<InstalledResources>, };
export type RuntimeResourceRequest = { "type": "refresh" } | { "type": "download", bytes: number, sha256: string, } | { "type": "install", archive: string, sha256: string, } | { "type": "activate", id: string, revision: string, } | { "type": "rollback", revision: string, };
export type RuntimeResourceJobState = "running" | "cancelling" | "succeeded" | "failed" | "cancelled";
export type RuntimeResourceJob = { id: string, request: RuntimeResourceRequest, state: RuntimeResourceJobState, phase: string, reconnected: boolean, error: string | null, error_code: string | null, progress: RuntimeDownloadProgress | null, downloaded: RuntimeDownloadedArchive | null, };
export type RuntimeDownloadSource = { url: string, bytes: number, sha256: string, };
export type RuntimeDownloadProgress = { phase: string, completed_bytes: number, total_bytes: number, };
export type RuntimeDownloadedArchive = { archive: string, sha256: string, bytes: number, reused: boolean, };
export type DesktopRuntimeResources = { catalog: RuntimeResourceCatalog | null, job: RuntimeResourceJob | null, };
export type Screenshot = { png_base64: string, width: number, height: number, color_order: string, capture_method: CaptureMethod | null, quality: ScreenshotQuality, captured_at: string, elapsed_ms: number, };
export type Command = { "type": "launch_game", "params": { instance_id: number, revision: number, confirm: boolean, } } | { "type": "model_resources" } | { "type": "start_model_resources", "params": { kind: ResourceJobKind, models: Array<string>, offline: boolean, } } | { "type": "cancel_model_resources", "params": { job_id: string, } } | { "type": "preview_group", "params": { instance_id: number, group_id: string, } } | { "type": "start_group_trial", "params": { instance_id: number, group_id: string, revision: number, game_day: string, allow_input: boolean, confirm_account: boolean, } } | { "type": "run_history", "params": { instance_id: number, } } | { "type": "query_run_history", "params": { instance_id: number, query: HistoryQuery, } } | { "type": "export_run_history", "params": { instance_id: number, query: HistoryQuery, content_id: string, } } | { "type": "retry_run_save" } | { "type": "account_config" } | { "type": "config_backups" } | { "type": "preview_config_backup", "params": { revision: number, id: string, } } | { "type": "restore_config_backup", "params": { revision: number, id: string, preview_id: string, confirm: boolean, } } | { "type": "preview_config_import", "params": { source: string, } } | { "type": "apply_config_import", "params": { source: string, preview_id: string, } } | { "type": "update_account_config", "params": { revision: number, change: ConfigChange, } } | { "type": "start_mail", "params": { config: MailConfig, allow_input: boolean, } } | { "type": "start_map", "params": { config: MapConfig, allow_input: boolean, } } | { "type": "start_coffee", "params": { config: CoffeeConfig, allow_input: boolean, } } | { "type": "start_coffee_from_world", "params": { config: CoffeeConfig, allow_input: boolean, } } | { "type": "pause", "params": { run_id: string, } } | { "type": "resume", "params": { run_id: string, } } | { "type": "snapshot" } | { "type": "applications" } | { "type": "windows" } | { "type": "attach", "params": { hwnd: string, capture_method: CaptureMethod, } } | { "type": "screenshot" } | { "type": "start_probe" } | { "type": "start_input_probe", "params": { config: InputProbeConfig, allow_input: boolean, } } | { "type": "start_input_pipeline", "params": { config: InputPipelineConfig, allow_input: boolean, } } | { "type": "start_application", "params": { app_id: string, } } | { "type": "stop", "params": { run_id: string, } } | { "type": "events", "params": { after: number, } } | { "type": "shutdown" };
export type Request = { version: number, id: string, command: Command, };
export type Payload = { "type": "game_launched", "data": GameLaunchResult } | { "type": "model_resources", "data": ModelResources } | { "type": "group_plan", "data": GroupPlan } | { "type": "run_history", "data": RunHistory } | { "type": "history_page", "data": HistoryPage } | { "type": "history_export", "data": HistoryExport } | { "type": "account_config", "data": AccountConfig } | { "type": "config_backups", "data": ConfigBackups } | { "type": "config_backup_preview", "data": ConfigBackupPreview } | { "type": "config_import_preview", "data": ImportPreview } | { "type": "snapshot", "data": Snapshot } | { "type": "applications", "data": Array<AppDefinition> } | { "type": "windows", "data": Array<WindowInfo> } | { "type": "screenshot", "data": Screenshot } | { "type": "run", "data": RunRecord } | { "type": "events", "data": Array<RunEvent> } | { "type": "ack" };
export type ApiError = { code: string, message: string, details: string | null, };
export type Response = { version: number, id: string, result: Payload | null, error: ApiError | null, };
export type DesktopUpdate = { "type": "run_notifications", "data": RunNotifications } | { "type": "open_run_notifications" } | { "type": "after_done", "data": AfterDoneStatus | null } | { "type": "snapshot", "data": Snapshot } | { "type": "offline", "data": ApiError } | { "type": "privilege", "data": EnginePrivilege } | { "type": "shortcuts", "data": ShortcutStatus };
export type EnginePrivilege = { engine_integrity: number, desktop_integrity: number, };
export const generalSettingsFields = [
  {
    "key": "capture_method",
    "kind": "select",
    "label": "默认截图方式",
    "options": [
      {
        "label": "FramePool",
        "value": "frame_pool"
      },
      {
        "label": "PrintWindow",
        "value": "print_window"
      },
      {
        "label": "DXGI",
        "value": "dxgi"
      },
      {
        "label": "ScreenDC",
        "value": "screen_dc"
      },
      {
        "label": "GDI",
        "value": "gdi"
      }
    ]
  },
  {
    "key": "timeout_ms",
    "kind": "integer",
    "label": "默认邮件超时（ms）",
    "max": 60000,
    "min": 1
  },
  {
    "key": "start_delay_ms",
    "kind": "integer",
    "label": "默认启动 / 继续延时（ms）",
    "max": 10000,
    "min": 0
  }
] as const;
export const gameRegionOptions = [
  {
    "label": "国服",
    "value": "cn"
  },
  {
    "label": "B服",
    "value": "cn_b"
  },
  {
    "label": "美服",
    "value": "us"
  },
  {
    "label": "欧服",
    "value": "eu"
  },
  {
    "label": "亚服",
    "value": "asia"
  },
  {
    "label": "港澳台服",
    "value": "twhkmo"
  }
] as const;
export const runPolicyOptions = [
  {
    "label": "每次运行",
    "value": "always"
  },
  {
    "label": "跳过本游戏日已完成",
    "value": "skip_daily_success"
  }
] as const;
export const shortcutKeys = [
  "f1",
  "f2",
  "f3",
  "f4",
  "f5",
  "f6",
  "f7",
  "f8",
  "f9",
  "f10",
  "f11"
] as const;
export const coffeeSettingsFields = [
  {
    "key": "kind",
    "kind": "select",
    "label": "默认咖啡",
    "options": [
      {
        "label": "汀曼特调",
        "value": "tinman"
      },
      {
        "label": "浓缩咖啡",
        "value": "espresso"
      }
    ]
  },
  {
    "key": "timeout_ms",
    "kind": "integer",
    "label": "默认咖啡超时（ms）",
    "max": 60000,
    "min": 1
  },
  {
    "key": "start_delay_ms",
    "kind": "integer",
    "label": "默认咖啡启动 / 继续延时（ms）",
    "max": 10000,
    "min": 0
  }
] as const;
export const coffeeDefaultConfig = {
  "kind": "tinman",
  "timeout_ms": 60000,
  "start_delay_ms": 3000
} as const;
export const afterDoneOptions = [
  {
    "label": "无",
    "value": "none"
  },
  {
    "label": "关闭游戏",
    "value": "close_game"
  },
  {
    "label": "关机",
    "value": "shutdown"
  },
  {
    "label": "休眠",
    "value": "hibernate"
  },
  {
    "label": "睡眠",
    "value": "sleep"
  }
] as const;
export const gameLaunchDefaultConfig = {
  "executable": "",
  "use_arguments": false,
  "fullscreen": false,
  "borderless": false,
  "monitor": 1
} as const;
export const runNotificationOptions = [
  {
    "label": "关闭",
    "value": "off"
  },
  {
    "label": "仅异常",
    "value": "errors"
  },
  {
    "label": "全部结果",
    "value": "all"
  }
] as const;
