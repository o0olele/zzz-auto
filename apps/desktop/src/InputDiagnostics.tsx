import { useEffect, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import { Play20Regular, Stop20Regular } from '@fluentui/react-icons';
import type { InputMode, InputProbeAction, InputProbeConfig, InputSide, MouseButton, Snapshot } from './generated/protocol';
import { keyName, keyOptions } from './inputKeys';

const modes: Record<InputMode, string> = { foreground: '前台键鼠', foreground_legacy: '前台键鼠（兼容方式）', background: '后台键鼠', background_post: '后台键鼠（异步方式）', xbox: 'Xbox 360', ds4: 'DualShock 4' };
const mouseButtons: Record<MouseButton, string> = { left: '左键', right: '右键', middle: '中键', x1: '侧键 X1', x2: '侧键 X2' };
type ActionType = InputProbeAction['type'];

function integerIn(value: string, min: number, max: number): boolean {
  return value.trim() !== '' && Number.isSafeInteger(Number(value)) && Number(value) >= min && Number(value) <= max;
}

function actionName(mode: InputMode, action: InputProbeAction): string {
  switch (action.type) {
    case 'key': return keyName(mode, action.key);
    case 'mouse_button': return '鼠标' + mouseButtons[action.button] + ' (' + action.x + ', ' + action.y + ')';
    case 'stick': return (action.stick === 'left' ? '左' : '右') + '摇杆 (' + action.x + ', ' + action.y + ')';
    case 'trigger': return (action.trigger === 'left' ? 'LT / L2' : 'RT / R2') + ' · ' + action.value;
  }
}

export function InputDiagnostics({ snapshot, connected, busy, onStart, onStop }: {
  snapshot: Snapshot | null; connected: boolean; busy: boolean;
  onStart: (config: InputProbeConfig, pipelineTimeout: number | null) => Promise<void>; onStop: () => Promise<void>;
}) {
  const [mode, setMode] = useState<InputMode>('foreground');
  const [kind, setKind] = useState<ActionType>('key');
  const [key, setKey] = useState('87');
  const [button, setButton] = useState<MouseButton>('left');
  const [side, setSide] = useState<InputSide>('left');
  const [x, setX] = useState('960');
  const [y, setY] = useState('540');
  const [value, setValue] = useState('128');
  const [hold, setHold] = useState('500');
  const [delay, setDelay] = useState('3000');
  const [confirmed, setConfirmed] = useState(false);
  const [execution, setExecution] = useState('direct');
  const [timeout, setTimeout] = useState('5000');
  const pipeline = execution === 'pipeline';
  const active = !!snapshot?.run && ['running', 'pausing', 'paused', 'stopping'].includes(snapshot.run.state);
  const inputRun = snapshot?.run && ['prototype.input_probe', 'prototype.input_pipeline'].includes(snapshot.run.app_id) ? snapshot.run : null;
  const result = inputRun?.input_probe;
  const pipelineResult = inputRun?.input_pipeline;
  const gamepad = mode === 'xbox' || mode === 'ds4';
  const keys = keyOptions(mode);
  const action: InputProbeAction = kind === 'key' ? { type: 'key', key: Number(key) }
    : kind === 'mouse_button' ? { type: 'mouse_button', button, x: Number(x), y: Number(y) }
    : kind === 'stick' ? { type: 'stick', stick: side, x: Number(x), y: Number(y) }
    : { type: 'trigger', trigger: side, value: Number(value) };
  const config: InputProbeConfig = { mode, action, hold_ms: Number(hold), start_delay_ms: Number(delay) };
  const validAction = kind === 'key' ? keys.some(option => option.value === Number(key))
    : kind === 'mouse_button' ? !gamepad && integerIn(x, 0, 1919) && integerIn(y, 0, 1079)
    : kind === 'stick' ? gamepad && integerIn(x, -32768, 32767) && integerIn(y, -32768, 32767) && (Number(x) !== 0 || Number(y) !== 0)
    : gamepad && integerIn(value, 1, 255);
  const valid = validAction && integerIn(hold, 1, 2000) && integerIn(delay, 0, 10000) && (!pipeline || (integerIn(timeout, 1, 10000) && (kind === 'key' || kind === 'mouse_button')));
  const disabled = !connected || busy || active;
  useEffect(() => { setConfirmed(false); }, [connected, snapshot?.window?.hwnd, snapshot?.window?.pid, snapshot?.capture_method]);

  function edit(set: (value: string) => void, next: string) { set(next); setConfirmed(false); }
  function selectAction(next: ActionType) {
    setKind(next); setConfirmed(false);
    if (next === 'mouse_button') { setX('960'); setY('540'); }
    if (next === 'stick') { setX('16000'); setY('0'); }
  }

  return <Card className="input-diagnostics">
    <div className="card-title"><h2>真实输入诊断</h2><Badge appearance="outline">{pipeline ? 'Maa Pipeline' : '单次输入与释放'}</Badge></div>
    <p>选择输入方式和动作，持有后自动释放。鼠标被游戏锁定时，可先按 Win 或 Alt+Tab 回到工具；启动诊断后，在延时期间切回游戏。Xbox / DS4 需要 ViGEmBus 驱动。</p>
    {!gamepad && snapshot?.window?.keyboard_permission?.compatible === false && <MessageBar intent="warning"><MessageBarBody>检测到游戏权限高于引擎，Windows 会限制键鼠输入。请先停止任务，再到总览点击“提升引擎权限”，授权后重新连接游戏。</MessageBarBody></MessageBar>}
    <div className="input-fields">
      <Field label="执行方式"><Select value={execution} disabled={disabled} onChange={(_, data) => { setExecution(data.value); if (kind === 'stick' || kind === 'trigger') setKind('key'); setConfirmed(false); }}><option value="direct">直接输入</option><option value="pipeline">Maa Pipeline</option></Select></Field>
      <Field label="输入方式"><Select value={mode} disabled={disabled} onChange={(_, data) => { const next = data.value as InputMode; setMode(next); setKind('key'); setKey(next === 'xbox' || next === 'ds4' ? '1' : '87'); setConfirmed(false); }}>{Object.entries(modes).map(([option, label]) => <option key={option} value={option}>{label}</option>)}</Select></Field>
      <Field label="输入动作"><Select value={kind} disabled={disabled} onChange={(_, data) => selectAction(data.value as ActionType)}><option value="key">{gamepad ? '数字按键' : '键盘按键'}</option>{gamepad ? (!pipeline && <><option value="stick">摇杆</option><option value="trigger">扳机</option></>) : <option value="mouse_button">鼠标按键</option>}</Select></Field>
      {kind === 'key' && <Field label={gamepad ? '手柄按键' : '键盘按键'}><Select value={key} disabled={disabled} onChange={(_, data) => edit(setKey, data.value)}>{keys.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</Select></Field>}
      {kind === 'mouse_button' && <Field label="鼠标按键"><Select value={button} disabled={disabled} onChange={(_, data) => { setButton(data.value as MouseButton); setConfirmed(false); }}>{Object.entries(mouseButtons).map(([option, label]) => <option key={option} value={option}>{label}</option>)}</Select></Field>}
      {(kind === 'stick' || kind === 'trigger') && <Field label={kind === 'stick' ? '摇杆选择' : '扳机选择'}><Select value={side} disabled={disabled} onChange={(_, data) => { setSide(data.value as InputSide); setConfirmed(false); }}><option value="left">{kind === 'stick' ? '左摇杆' : '左扳机 LT / L2'}</option><option value="right">{kind === 'stick' ? '右摇杆' : '右扳机 RT / R2'}</option></Select></Field>}
      {(kind === 'mouse_button' || kind === 'stick') && <>
        <Field label={kind === 'stick' ? '摇杆 X' : '鼠标 X'}><Input type="number" min={kind === 'stick' ? -32768 : 0} max={kind === 'stick' ? 32767 : 1919} value={x} disabled={disabled} onChange={(_, data) => edit(setX, data.value)}/></Field>
        <Field label={kind === 'stick' ? '摇杆 Y' : '鼠标 Y'}><Input type="number" min={kind === 'stick' ? -32768 : 0} max={kind === 'stick' ? 32767 : 1079} value={y} disabled={disabled} onChange={(_, data) => edit(setY, data.value)}/></Field>
      </>}
      {kind === 'trigger' && <Field label="扳机力度"><Input type="number" min={1} max={255} value={value} disabled={disabled} onChange={(_, data) => edit(setValue, data.value)}/></Field>}
      <Field label="按住时间（ms）"><Input type="number" min={1} max={2000} value={hold} disabled={disabled} onChange={(_, data) => edit(setHold, data.value)}/></Field>
      <Field label="启动延时（ms）"><Input type="number" min={0} max={10000} value={delay} disabled={disabled} onChange={(_, data) => edit(setDelay, data.value)}/></Field>
      {pipeline && <Field label="Pipeline 超时（ms）"><Input type="number" min={1} max={10000} value={timeout} disabled={disabled} onChange={(_, data) => edit(setTimeout, data.value)}/></Field>}
    </div>
    {pipeline && <p>Pipeline 执行固定的按下、短间隔等待、释放。超时不包含启动延时、截图准备与控制器重建；停止或超时仍等待原生队列收尾，不能立即交出控制权。摇杆和扳机请使用直接输入。</p>}
    {kind === 'mouse_button' && <p>坐标以游戏客户区左上角为原点，范围 X 0..1919、Y 0..1079。前台方式会移动鼠标到目标位置并持有所选按键，请选择可观察的画面位置。</p>}
    {kind === 'stick' && <p>摇杆 X / Y 为 -32768..32767，X 正值向右、Y 正值向上。持有结束或停止后回中；可在大世界观察移动或视角变化。</p>}
    {kind === 'trigger' && <p>力度为 1..255，持有结束或停止后归零。游戏须为所选扳机设置对应动作，才能观察响应。</p>}
    <p>{gamepad ? 'Xbox / DS4 的数字按键、摇杆和扳机分别验收；完成表示控制器已执行和释放，游戏响应需实际观察。' : mode === 'background' || mode === 'background_post' ? '后台键鼠属于兼容性测试，当前环境的后台键盘未生效；鼠标响应需单独观察。' : '延时结束时游戏须处于前台。键盘可在自由移动的大世界测试 W / A / S / D；若没有响应，可对照兼容方式。'}</p>
    {!valid && <p role="alert">请输入有效动作参数，按住时间为 1..2000ms，启动延时为 0..10000ms{pipeline ? '，Pipeline 超时为 1..10000ms' : ''}。</p>}
    <Checkbox label="我确认向当前游戏窗口发送这次真实输入" checked={confirmed} disabled={disabled || !valid} onChange={(_, data) => setConfirmed(data.checked === true)}/>
    <div className="button-row"><Button appearance="primary" icon={<Play20Regular/>} disabled={disabled || !snapshot?.window || !confirmed || !valid} onClick={async () => { setConfirmed(false); await onStart(config, pipeline ? Number(timeout) : null); }}>开始输入诊断</Button><Button icon={<Stop20Regular/>} disabled={!connected || !active || !inputRun || inputRun.state === 'stopping'} onClick={onStop}>停止输入诊断</Button></div>
    {active && inputRun && <Spinner size="tiny" label={inputRun.state === 'stopping' ? '等待队列完成与输入释放' : '正在准备控制器、等待延时或执行输入'}/>}
    {inputRun && <p className="run-message">{inputRun.message}</p>}
    {result && <>
      <div className="input-metrics" aria-label="输入诊断结果">
        <span>实际方式<strong>{modes[result.config.mode]}</strong></span>
        <span>实际动作<strong>{actionName(result.config.mode, result.config.action)}</strong></span>
        <span>按下投递<strong>{result.enqueue_us} μs</strong></span>
        <span>按下完成<strong>{result.key_down_ms} ms</strong></span>
        <span>实际持有<strong>{result.held_ms} ms</strong></span>
        <span>输入释放<strong>{result.release_ms} ms</strong></span>
        <span>取消收尾<strong>{result.stop_ms === null ? '未取消' : result.stop_ms + ' ms'}</strong></span>
      </div>
      <p>{result.pressed ? '已投递输入并完成释放。' : '取消时未投递输入，控制器已收尾。'}计时来自 Maa 队列，取消收尾从引擎检测到取消时开始；游戏是否响应及是否存在残留动作，需要观察游戏确认。</p>
    </>}
    {pipelineResult && <>
      <div className="input-metrics" aria-label="Pipeline 诊断结果">
        <span>实际执行<strong>Maa Pipeline</strong></span>
        <span>实际方式<strong>{modes[pipelineResult.config.input.mode]}</strong></span>
        <span>实际动作<strong>{actionName(pipelineResult.config.input.mode, pipelineResult.config.input.action)}</strong></span>
        <span>流程结果<strong>{{ succeeded: '完成', failed: '失败', cancelled: '已停止', timed_out: '超时' }[pipelineResult.pipeline.end]}</strong></span>
        <span>Pipeline 投递<strong>{pipelineResult.pipeline.enqueue_us} μs</strong></span>
        <span>Pipeline 总耗时<strong>{pipelineResult.pipeline.elapsed_ms} ms</strong></span>
        <span>输入释放<strong>{pipelineResult.pipeline.release_ms} ms</strong></span>
        <span>停止／超时收尾<strong>{pipelineResult.pipeline.stop_ms === null ? '未请求' : pipelineResult.pipeline.stop_ms + ' ms'}</strong></span>
      </div>
      <p>{pipelineResult.pipeline.released ? '原生队列与显式输入释放已完成。' : '输入释放尚未确认。'}流程总耗时不包含启动延时和准备；不能作为实际持有时长或完整 IPC 停止耗时。游戏响应及停止后的残留动作仍需实际观察。</p>
    </>}
  </Card>;
}
