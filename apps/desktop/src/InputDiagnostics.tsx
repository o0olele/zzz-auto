import { useEffect, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, Select, Spinner } from '@fluentui/react-components';
import { Play20Regular, Stop20Regular } from '@fluentui/react-icons';
import type { InputMode, InputProbeConfig, Snapshot } from './generated/protocol';

const modes: Record<InputMode, string> = { foreground: '前台键盘', background: '后台键盘', xbox: 'Xbox 360', ds4: 'DualShock 4' };

export function InputDiagnostics({ snapshot, connected, busy, onStart, onStop }: {
  snapshot: Snapshot | null; connected: boolean; busy: boolean;
  onStart: (config: InputProbeConfig) => Promise<void>; onStop: () => Promise<void>;
}) {
  const [mode, setMode] = useState<InputMode>('foreground');
  const [key, setKey] = useState('87');
  const [hold, setHold] = useState('500');
  const [delay, setDelay] = useState('3000');
  const [confirmed, setConfirmed] = useState(false);
  const active = !!snapshot?.run && ['running', 'stopping'].includes(snapshot.run.state);
  const inputRun = snapshot?.run?.app_id === 'prototype.input_probe' ? snapshot.run : null;
  const result = inputRun?.input_probe;
  const config: InputProbeConfig = { mode, key: Number(key), hold_ms: Number(hold), start_delay_ms: Number(delay) };
  const mask = mode === 'xbox' ? 0xf7ff : 0x3f7ff;
  const valid = [key, hold, delay].every(value => value.trim() !== '')
    && Object.values(config).slice(1).every(value => Number.isSafeInteger(value))
    && config.key > 0 && (mode === 'foreground' || mode === 'background' ? config.key <= 254 : config.key <= mask && (config.key & ~mask) === 0)
    && config.hold_ms >= 1 && config.hold_ms <= 2000 && config.start_delay_ms >= 0 && config.start_delay_ms <= 10000;
  const disabled = !connected || busy || active;
  useEffect(() => { setConfirmed(false); }, [connected, snapshot?.window?.hwnd, snapshot?.window?.pid]);

  function edit(set: (value: string) => void, value: string) { set(value); setConfirmed(false); }

  return <Card className="input-diagnostics">
    <div className="card-title"><h2>真实输入诊断</h2><Badge appearance="outline">单次按下与释放</Badge></div>
    <p>选择输入方式，按住一个按键后释放。启动延时用于切回游戏；前台方式可能将游戏置顶并临时接管输入。Xbox / DS4 需要 ViGEmBus 驱动。</p>
    <div className="input-fields">
      <Field label="输入方式"><Select value={mode} disabled={disabled} onChange={(_, data) => { const next = data.value as InputMode; setMode(next); setKey(next === 'xbox' || next === 'ds4' ? '1' : '87'); setConfirmed(false); }}>{Object.entries(modes).map(([value, label]) => <option key={value} value={value}>{label}</option>)}</Select></Field>
      <Field label={mode === 'xbox' || mode === 'ds4' ? '手柄按键位掩码' : '键盘 VK'}><Input type="number" value={key} disabled={disabled} onChange={(_, data) => edit(setKey, data.value)}/></Field>
      <Field label="按住时间（ms）"><Input type="number" min={1} max={2000} value={hold} disabled={disabled} onChange={(_, data) => edit(setHold, data.value)}/></Field>
      <Field label="启动延时（ms）"><Input type="number" min={0} max={10000} value={delay} disabled={disabled} onChange={(_, data) => edit(setDelay, data.value)}/></Field>
    </div>
    <p>{mode === 'xbox' || mode === 'ds4' ? '位掩码示例：方向上 1、A / Cross 4096、LB / L1 256。只支持数字按键；摇杆与扳机另行验收。' : 'VK 示例：W 87、A 65、S 83、D 68、空格 32。请选择可观察按下与释放效果的场景。'}</p>
    {!valid && <p role="alert">请输入有效按键，按住时间为 1..2000ms，启动延时为 0..10000ms。</p>}
    <Checkbox label="我确认向当前游戏窗口发送这次真实输入" checked={confirmed} disabled={disabled || !valid} onChange={(_, data) => setConfirmed(data.checked === true)}/>
    <div className="button-row"><Button appearance="primary" icon={<Play20Regular/>} disabled={disabled || !snapshot?.window || !confirmed || !valid} onClick={async () => { setConfirmed(false); await onStart(config); }}>开始输入诊断</Button><Button icon={<Stop20Regular/>} disabled={!connected || !active || !inputRun || inputRun.state === 'stopping'} onClick={onStop}>停止输入诊断</Button></div>
    {active && inputRun && <Spinner size="tiny" label={inputRun.state === 'stopping' ? '等待队列完成与输入释放' : '正在准备控制器、等待延时或执行按键'}/>}
    {inputRun && <p className="run-message">{inputRun.message}</p>}
    {result && <>
      <div className="input-metrics" aria-label="输入诊断结果">
        <span>实际方式<strong>{modes[result.config.mode]}</strong></span>
        <span>按下投递<strong>{result.enqueue_us} μs</strong></span>
        <span>按下完成<strong>{result.key_down_ms} ms</strong></span>
        <span>实际持有<strong>{result.held_ms} ms</strong></span>
        <span>输入释放<strong>{result.release_ms} ms</strong></span>
        <span>取消收尾<strong>{result.stop_ms === null ? '未取消' : `${result.stop_ms} ms`}</strong></span>
      </div>
      <p>{result.pressed ? '已投递按下并完成释放。' : '取消时未投递按下，控制器已收尾。'}计时来自 Maa 队列，取消收尾从引擎检测到取消时开始；游戏是否响应及是否存在残留动作，需要观察游戏确认。</p>
    </>}
  </Card>;
}
