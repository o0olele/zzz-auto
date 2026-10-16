// 由 od-tools types 生成；请修改 Rust 协议。
export type GameRegion = "cn" | "cn_b" | "us" | "eu" | "asia" | "twhkmo";
export type RunPolicy = "always" | "skip_daily_success";
export type GameDay = { region: GameRegion, date: string, };
export type RunContext = { instance_id: number, instance_name: string, group_id: string, config_revision: number, app_ids: Array<string>, game_day?: GameDay, };
export type PlanDecision = "disabled" | "run" | "already_done" | "blocked";
export type AppAvailability = "mail_trial" | "unmigrated" | "unknown";
export type PlannedApp = { app_id: string, name: string, enabled: boolean, availability: AppAvailability, decision: PlanDecision, completed_run_id: string | null, };
export type GroupPlan = { context: RunContext, mail_config: MailConfig, items: Array<PlannedApp>, blockers: Array<string>, executable: boolean, all_done: boolean, game_day: GameDay, run_policy: RunPolicy, warnings: Array<string>, valid_until: string, };
export type HistoryItem = { record: RunRecord, interrupted: boolean, };
export type RunHistory = { instance_id: number, items: Array<HistoryItem>, unreadable: number, has_more: boolean, };
export type GroupApp = { app_id: string, enabled: boolean, };
export type GeneralSettings = { capture_method: CaptureMethod, mail: MailConfig, };
export type AppGroup = { id: string, run_policy: RunPolicy, apps: Array<GroupApp>, };
export type AccountInstance = { id: number, name: string, active_in_od: boolean, game_region: GameRegion, groups: Array<AppGroup>, };
export type AccountConfig = { schema: number, revision: number, general: GeneralSettings, active_instance: number, instances: Array<AccountInstance>, import_id: string | null, };
export type ConfigChange = { "type": "set_game_region", instance_id: number, game_region: GameRegion, } | { "type": "set_run_policy", instance_id: number, group_id: string, run_policy: RunPolicy, } | { "type": "save_general", settings: GeneralSettings, } | { "type": "create_instance", name: string, } | { "type": "update_instance", instance_id: number, name: string, active_in_od: boolean, } | { "type": "switch_instance", instance_id: number, } | { "type": "save_group", instance_id: number, group_id: string, apps: Array<GroupApp>, };
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
export type RunRecord = { id: string, app_id: string, state: RunState, started_at: string, finished_at: string | null, message: string, context?: RunContext, input_probe: InputProbeResult | null, input_pipeline: InputPipelineResult | null, mail: MailProgress | null, map?: MapProgress, group?: GroupProgress, };
export type RunEvent = { sequence: number, run_id: string | null, at: string, level: string, message: string, };
export type WindowInfo = { hwnd: string, pid: number, title: string, executable: string, width: number, height: number, keyboard_permission: KeyboardPermission | null, };
export type KeyboardPermission = { engine_integrity: number, game_integrity: number, compatible: boolean, };
export type SdkStatus = { ready: boolean, version: string | null, expected_version: string, binding_version: string, directory: string, message: string, };
export type CaptureMethod = "frame_pool" | "print_window" | "dxgi" | "screen_dc" | "gdi";
export type ScreenshotQuality = { mean_luma: number, dark_pixel_ratio: number, near_black: boolean, };
export type Snapshot = { record_error?: string, engine_version: string, sdk: SdkStatus, window: WindowInfo | null, capture_method: CaptureMethod | null, run: RunRecord | null, events: Array<RunEvent>, event_sequence: number, dropped_events: number, };
export type Screenshot = { png_base64: string, width: number, height: number, color_order: string, capture_method: CaptureMethod | null, quality: ScreenshotQuality, captured_at: string, elapsed_ms: number, };
export type Command = { "type": "preview_group", "params": { instance_id: number, group_id: string, } } | { "type": "start_group_trial", "params": { instance_id: number, group_id: string, revision: number, game_day: string, allow_input: boolean, confirm_account: boolean, } } | { "type": "run_history", "params": { instance_id: number, } } | { "type": "retry_run_save" } | { "type": "account_config" } | { "type": "update_account_config", "params": { revision: number, change: ConfigChange, } } | { "type": "start_mail", "params": { config: MailConfig, allow_input: boolean, } } | { "type": "start_map", "params": { config: MapConfig, allow_input: boolean, } } | { "type": "pause", "params": { run_id: string, } } | { "type": "resume", "params": { run_id: string, } } | { "type": "snapshot" } | { "type": "applications" } | { "type": "windows" } | { "type": "attach", "params": { hwnd: string, capture_method: CaptureMethod, } } | { "type": "screenshot" } | { "type": "start_probe" } | { "type": "start_input_probe", "params": { config: InputProbeConfig, allow_input: boolean, } } | { "type": "start_input_pipeline", "params": { config: InputPipelineConfig, allow_input: boolean, } } | { "type": "start_application", "params": { app_id: string, } } | { "type": "stop", "params": { run_id: string, } } | { "type": "events", "params": { after: number, } } | { "type": "shutdown" };
export type Request = { version: number, id: string, command: Command, };
export type Payload = { "type": "group_plan", "data": GroupPlan } | { "type": "run_history", "data": RunHistory } | { "type": "account_config", "data": AccountConfig } | { "type": "snapshot", "data": Snapshot } | { "type": "applications", "data": Array<AppDefinition> } | { "type": "windows", "data": Array<WindowInfo> } | { "type": "screenshot", "data": Screenshot } | { "type": "run", "data": RunRecord } | { "type": "events", "data": Array<RunEvent> } | { "type": "ack" };
export type ApiError = { code: string, message: string, details: string | null, };
export type Response = { version: number, id: string, result: Payload | null, error: ApiError | null, };
export type DesktopUpdate = { "type": "snapshot", "data": Snapshot } | { "type": "offline", "data": ApiError } | { "type": "privilege", "data": EnginePrivilege };
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
