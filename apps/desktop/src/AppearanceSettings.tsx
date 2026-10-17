import { Badge, Button, Card, Field, Select } from '@fluentui/react-components';
import { interfaceScales, useAppearance, type InterfaceScale, type ThemePreference } from './appearance';

export function AppearanceSettings() {
  const { preferences, dark, update, reset } = useAppearance();
  return <Card className="appearance-card">
    <div className="card-title"><h2>桌面外观</h2><Badge appearance="outline">本机界面</Badge></div>
    <div className="input-fields">
      <Field label="界面主题"><Select value={preferences.theme} onChange={(_, data) => update({ theme: data.value as ThemePreference })}>
        <option value="system">跟随系统</option><option value="light">浅色</option><option value="dark">深色</option>
      </Select></Field>
      <Field label="界面缩放"><Select value={String(preferences.scale)} onChange={(_, data) => update({ scale: Number(data.value) as InterfaceScale })}>
        {interfaceScales.map(scale => <option key={scale} value={scale}>{scale}%{scale === 100 ? '（默认）' : ''}</option>)}
      </Select></Field>
    </div>
    <p>当前为{dark ? '深色' : '浅色'}主题。更改后立即应用并保存在本机，独立于账号配置；运行任务或引擎未连接时也可调整。</p>
    <p>界面缩放叠加在 Windows 显示缩放之上，游戏截图和输入坐标保持 1920 × 1080 基准。</p>
    <div className="button-row"><Button onClick={reset}>恢复外观默认值</Button></div>
  </Card>;
}
