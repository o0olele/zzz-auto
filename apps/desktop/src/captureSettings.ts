import type { CaptureMethod } from './generated/protocol';
import { generalSettingsFields } from './generated/protocol';

const captureField = generalSettingsFields.find(field => field.kind === 'select')!;
export const captureNames = Object.fromEntries(captureField.options.map(option => [option.value, option.label])) as Record<CaptureMethod, string>;
export const captureHints: Record<CaptureMethod, string> = {
  frame_pool: '可捕获被其他窗口遮挡的游戏。首次排查请保持游戏窗口已恢复。',
  print_window: '兼容方式，与参考项目的默认截图 API 相同。建议使用窗口模式。',
  dxgi: '从屏幕画面裁剪游戏区域。游戏必须可见、未最小化，且没有被其他窗口遮挡。',
  screen_dc: '从屏幕画面裁剪游戏区域。游戏必须完整显示；可作为 DXGI 的备选。',
  gdi: '传统兼容方式。部分游戏会返回黑图，遇到黑图请尝试 FramePool 或 PrintWindow。',
};
