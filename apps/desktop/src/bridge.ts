import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import type { ApiError, Command, DesktopUpdate, Payload, DesktopRuntimeResources, RuntimeResourceRequest, RuntimeDownloadSource } from './generated/protocol';

export const desktop = isTauri();

export async function cancelAfterDone(id: string): Promise<import('./generated/protocol').AfterDoneStatus | null> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  return invoke('cancel_after_done', { id });
}

export async function configRecoveryState(): Promise<import('./generated/protocol').DesktopConfigRecovery> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  return invoke('config_recovery_state');
}

export async function startConfigRecovery(request: import('./generated/protocol').ConfigRecoveryRequest): Promise<import('./generated/protocol').DesktopConfigRecovery> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  return invoke('start_config_recovery', { request });
}

export async function desktopLogs(sessionId: string | null): Promise<import('./generated/protocol').DesktopLogs> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  return invoke('desktop_logs', { sessionId });
}

export async function exportLogSession(sessionId: string): Promise<import('./generated/protocol').LogExport> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  return invoke('export_log_session', { sessionId });
}

export async function retryShortcuts(): Promise<import('./generated/protocol').ShortcutStatus> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  return invoke('retry_shortcuts');
}

export async function request(command: Command): Promise<Payload> {
  if (!desktop) throw { code: 'preview_only', message: '浏览器仅供界面预览，请通过 Tauri 启动原生引擎。', details: null } satisfies ApiError;
  return invoke<Payload>('engine_request', { command });
}

export async function subscribe(onUpdate: (update: DesktopUpdate) => void): Promise<void> {
  if (!desktop) return;
  const channel = new Channel<DesktopUpdate>();
  channel.onmessage = onUpdate;
  await invoke('subscribe_engine', { channel });
}

export async function reconnect(): Promise<void> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  await invoke('reconnect_engine');
}

export async function elevateEngine(): Promise<void> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  await invoke('elevate_engine');
}

export async function recoverInputs(): Promise<string> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  return invoke<string>('recover_inputs');
}

export async function runtimeResourceState(): Promise<DesktopRuntimeResources> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  return invoke<DesktopRuntimeResources>('runtime_resource_state');
}

export async function pickRuntimeResourceArchive(): Promise<string | null> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  return invoke<string | null>('pick_runtime_resource_archive');
}

export async function pickConfigImportSource(): Promise<string | null> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  return invoke<string | null>('pick_config_import_source');
}

export async function startRuntimeResourceJob(request: RuntimeResourceRequest): Promise<DesktopRuntimeResources> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  return invoke<DesktopRuntimeResources>('start_runtime_resource_job', { request });
}

export async function startRuntimeResourceDownload(source: RuntimeDownloadSource): Promise<DesktopRuntimeResources> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  return invoke<DesktopRuntimeResources>('start_runtime_resource_download', { source });
}

export async function cancelRuntimeResourceDownload(id: string): Promise<DesktopRuntimeResources> {
  if (!desktop) throw new Error('请通过 Tauri 启动桌面应用');
  return invoke<DesktopRuntimeResources>('cancel_runtime_resource_download', { id });
}

export function errorMessage(error: unknown): string {
  if (typeof error === 'object' && error && 'message' in error) return String(error.message);
  return String(error);
}
