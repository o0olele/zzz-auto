import { useEffect, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, MessageBar, MessageBarBody, Select } from '@fluentui/react-components';
import type { CoffeeApproachPhase, CoffeeKind, CoffeePhase, Command, Snapshot } from './generated/protocol';
import { coffeeDefaultConfig, coffeeSettingsFields } from './generated/protocol';
import { CoffeePresets, coffeeName } from './CoffeeSettings';

const phases: Record<CoffeePhase, string> = { inspect: '检查咖啡选项', order: '点单', wait_energy: '等待喝完与电量', confirm_energy: '确认电量', after_energy: '检查喝后画面', confirm_buff: '确认咖啡增益', done: '完成' };
const approachPhases: Record<CoffeeApproachPhase, string> = { inspect_world: '检查大世界起点', open_map: '打开普通地图', wait_map: '等待地图列表', select_area: '选择澄辉坪', select_point: '选择汀曼咖啡', wait_dialog: '等待传送确认', confirm: '确认传送', wait_arrival: '等待落地', interact: '与咖啡店交互', wait_dialogue: '等待汀曼大师', advance_dialogue: '推进点单前对话', done: '已进入点单' };
const states: Record<string, string> = { running: '运行中', pausing: '暂停中', paused: '已暂停', stopping: '停止中', succeeded: '完成', failed: '失败', stopped: '已停止' };
export function CoffeeDiagnostics({ snapshot, connected, busy, onCommand }: {
  snapshot: Snapshot | null; connected: boolean; busy: boolean; onCommand: (command: Command) => Promise<void>;
}) {
  const [kind, setKind] = useState<CoffeeKind>(coffeeDefaultConfig.kind);
  const [start, setStart] = useState<'options' | 'world'>('options');
  const [timeout, setTimeout] = useState(String(coffeeDefaultConfig.timeout_ms));
  const [delay, setDelay] = useState(String(coffeeDefaultConfig.start_delay_ms));
  const [confirmed, setConfirmed] = useState(false);
  const active = !!snapshot?.run && ['running', 'pausing', 'paused', 'stopping'].includes(snapshot.run.state);
  const run = snapshot?.run?.app_id === 'prototype.coffee' ? snapshot.run : null;
  const progress = run?.coffee;
  const integer = (value: string, min: number, max: number) => value.trim() !== '' && Number.isSafeInteger(Number(value)) && Number(value) >= min && Number(value) <= max;
  const timeoutField = coffeeSettingsFields[1];
  const delayField = coffeeSettingsFields[2];
  const valid = integer(timeout, timeoutField.min, timeoutField.max) && integer(delay, delayField.min, delayField.max);
  const disabled = !connected || busy || active || !!snapshot?.record_error;
  useEffect(() => { setConfirmed(false); }, [connected, snapshot?.window?.hwnd, snapshot?.window?.pid, snapshot?.capture_method, snapshot?.run?.id, snapshot?.record_error]);
  const name = coffeeName(kind);
  useEffect(() => {
    if (progress) { setStart(progress.approach ? 'world' : 'options'); setKind(progress.config.kind); setTimeout(String(progress.config.timeout_ms)); setDelay(String(progress.config.start_delay_ms)); }
  }, [run?.id]);
  return <Card className="coffee-diagnostics">
    <div className="card-title"><h2>咖啡点单试运行</h2><Badge appearance="outline">实机待验收</Badge></div>
    <p>{start === 'world' ? '从大世界自动打开普通地图，沿澄辉坪 → 汀曼咖啡传送，交互并推进到点单选项。' : '先传送到澄辉坪的汀曼咖啡，手动交互到右侧出现咖啡选项，再启动。'}流程会选择本次咖啡、等待动画、确认电量和增益，最后返回大世界。</p>
    <MessageBar intent="info"><MessageBarBody>请在延时内切回游戏并保持前台。当前提供汀曼特调与浓缩咖啡的独立试运行，结果保存为诊断记录。</MessageBarBody></MessageBar>
    <CoffeePresets connected={connected} disabled={disabled} onInvalidate={() => setConfirmed(false)} onApply={value => { setKind(value.kind); setTimeout(String(value.timeout_ms)); setDelay(String(value.start_delay_ms)); }}/>
    <div className="input-fields">
      <Field label="咖啡起点"><Select value={start} disabled={disabled} onChange={(_, data) => { setStart(data.value as 'options' | 'world'); setConfirmed(false); }}><option value="options">已打开点单选项</option><option value="world">从大世界传送并点单</option></Select></Field>
      <Field label="本次咖啡"><Select value={kind} disabled={disabled} onChange={(_, data) => { setKind(data.value as CoffeeKind); setConfirmed(false); }}>{coffeeSettingsFields[0].options.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</Select></Field>
      <Field label="咖啡超时（ms）"><Input type="number" min={timeoutField.min} max={timeoutField.max} value={timeout} disabled={disabled} onChange={(_, data) => { setTimeout(data.value); setConfirmed(false); }}/></Field>
      <Field label="咖啡启动 / 继续延时（ms）"><Input type="number" min={delayField.min} max={delayField.max} value={delay} disabled={disabled} onChange={(_, data) => { setDelay(data.value); setConfirmed(false); }}/></Field>
    </div>
    <Checkbox label={`我确认本次${start === 'world' ? '自动开图并传送到' : '在'}澄辉坪点单${name}`} checked={confirmed} disabled={disabled || !valid} onChange={(_, data) => setConfirmed(data.checked === true)}/>
    <div className="button-row">
      <Button appearance="primary" disabled={disabled || !snapshot?.window || !valid || !confirmed} onClick={async () => { setConfirmed(false); await onCommand({ type: start === 'world' ? 'start_coffee_from_world' : 'start_coffee', params: { config: { kind, timeout_ms: Number(timeout), start_delay_ms: Number(delay) }, allow_input: true } }); }}>开始咖啡点单</Button>
      <Button disabled={!connected || busy || run?.state !== 'running'} onClick={() => run && onCommand({ type: 'pause', params: { run_id: run.id } })}>暂停咖啡</Button>
      <Button disabled={!connected || busy || run?.state !== 'paused'} onClick={() => run && onCommand({ type: 'resume', params: { run_id: run.id } })}>继续咖啡</Button>
      <Button disabled={!connected || busy || !run || !active || run.state === 'stopping'} onClick={() => run && onCommand({ type: 'stop', params: { run_id: run.id } })}>停止咖啡</Button>
    </div>
    {snapshot?.record_error && <><MessageBar intent="error"><MessageBarBody>{snapshot.record_error}</MessageBarBody></MessageBar><Button disabled={!connected || busy || active} onClick={() => onCommand({ type: 'retry_run_save' })}>重试保存咖啡记录</Button></>}
    {run && <div aria-label="咖啡运行结果"><Badge appearance="tint">{progress?.state === 'timed_out' ? '咖啡超时' : states[run.state]}</Badge><p>{run.message}</p>{progress && <p>步骤：{progress.approach && progress.approach.phase !== 'done' ? approachPhases[progress.approach.phase] : phases[progress.phase]} · 电量已确认出现：{progress.energy_seen ? '是' : '否'} · {progress.inputs_released ? '输入已释放' : '等待输入收尾'}</p>}<p>运行 ID：{run.id}</p></div>}
  </Card>;
}
