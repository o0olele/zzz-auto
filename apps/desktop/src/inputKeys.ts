import type { InputMode } from './generated/protocol';

export type KeyOption = { value: number; label: string };
const commonKeys: KeyOption[] = [
  { value: 87, label: 'W' }, { value: 65, label: 'A' }, { value: 83, label: 'S' }, { value: 68, label: 'D' },
  { value: 81, label: 'Q' }, { value: 69, label: 'E' }, { value: 82, label: 'R' },
  { value: 32, label: '空格 Space' }, { value: 27, label: 'Esc' }, { value: 13, label: '回车 Enter' },
  { value: 9, label: 'Tab' }, { value: 160, label: '左 Shift' }, { value: 161, label: '右 Shift' },
  { value: 162, label: '左 Ctrl' }, { value: 163, label: '右 Ctrl' }, { value: 164, label: '左 Alt' }, { value: 165, label: '右 Alt' },
  { value: 37, label: '方向键 ←' }, { value: 38, label: '方向键 ↑' }, { value: 39, label: '方向键 →' }, { value: 40, label: '方向键 ↓' },
];
export const keyboardKeys: KeyOption[] = [
  ...commonKeys,
  ...Array.from({ length: 26 }, (_, index) => ({ value: 65 + index, label: String.fromCharCode(65 + index) })).filter(key => !commonKeys.some(common => common.value === key.value)),
  ...Array.from({ length: 10 }, (_, index) => ({ value: 48 + index, label: `${index}（数字行）` })),
  ...Array.from({ length: 12 }, (_, index) => ({ value: 112 + index, label: `F${index + 1}` })),
  { value: 8, label: '退格 Backspace' }, { value: 46, label: 'Delete' }, { value: 45, label: 'Insert' },
  { value: 36, label: 'Home' }, { value: 35, label: 'End' }, { value: 33, label: 'Page Up' }, { value: 34, label: 'Page Down' },
];

const gamepadKeys: KeyOption[] = [
  { value: 1, label: '十字键 ↑' }, { value: 2, label: '十字键 ↓' }, { value: 4, label: '十字键 ←' }, { value: 8, label: '十字键 →' },
  { value: 4096, label: 'A / Cross（×）' }, { value: 8192, label: 'B / Circle（○）' },
  { value: 16384, label: 'X / Square（□）' }, { value: 32768, label: 'Y / Triangle（△）' },
  { value: 256, label: 'LB / L1' }, { value: 512, label: 'RB / R1' },
  { value: 64, label: '左摇杆按下 / L3' }, { value: 128, label: '右摇杆按下 / R3' },
  { value: 16, label: 'Start / Options' }, { value: 32, label: 'Back / Share' },
];

export function keyOptions(mode: InputMode): KeyOption[] {
  if (mode === 'xbox') return [...gamepadKeys, { value: 1024, label: 'Guide' }];
  if (mode === 'ds4') return [...gamepadKeys, { value: 65536, label: 'PS' }, { value: 131072, label: '触摸板按下' }];
  return keyboardKeys;
}

export function keyName(mode: InputMode, key: number): string {
  return keyOptions(mode).find(option => option.value === key)?.label ?? String(key);
}
