import { Channel, invoke, isTauri } from '@tauri-apps/api/core';
import type { ApiError, Command, DesktopUpdate, Payload } from './generated/protocol';

export const desktop = isTauri();

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

export function errorMessage(error: unknown): string {
  if (typeof error === 'object' && error && 'message' in error) return String(error.message);
  return String(error);
}
