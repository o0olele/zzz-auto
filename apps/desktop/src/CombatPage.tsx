import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, MessageBar, MessageBarBody, Select, Spinner, Textarea } from '@fluentui/react-components';
import type { AccountConfig, CombatConfig, CombatScriptImportPreview, Command, InputMode, Snapshot } from './generated/protocol';
import { combatDefaultConfig } from './generated/protocol';
import { errorMessage, request } from './bridge';
import { isRunActive, runStateNames, RunSaveRecovery } from './GroupRunStatus';

const modes: Record<InputMode, string> = { foreground: '前台键鼠', foreground_legacy: '前台键鼠（兼容）', background: '后台键鼠', background_post: '后台键鼠（异步）', xbox: 'Xbox 360', ds4: 'DualShock 4' };
export const defaultCombatConfig = (): CombatConfig => structuredClone(combatDefaultConfig) as CombatConfig;

export function CombatPage({ connected, blocked, snapshot, onCommand }: {
  connected: boolean; blocked: boolean; snapshot: Snapshot | null; onCommand: (command: Command) => Promise<void>;
}) {
  const [account, setAccount] = useState<AccountConfig | null>(null);
  const [draft, setDraft] = useState<CombatConfig>(defaultCombatConfig);
  const [bindings, setBindings] = useState('{}');
  const [scripts, setScripts] = useState<string[]>([]);
  const [dodgeScripts,setDodgeScripts]=useState<string[]>([]);
  const [scriptRevision,setScriptRevision]=useState('');
  const [imported,setImported]=useState<string|null>(null);
  const [source,setSource]=useState('');
  const [preview,setPreview]=useState<CombatScriptImportPreview|null>(null);
  const [importConfirmed,setImportConfirmed]=useState(false);
  const [importing,setImporting]=useState(false);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState('');
  const [resourceError, setResourceError] = useState('');
  const [confirmed, setConfirmed] = useState(false);
  const generation = useRef(0);
  const run = snapshot?.run;
  const combat = run?.combat;
  const active = isRunActive(run);
  const instance = account?.instances.find(item => item.id === account.active_instance);
  const saved = instance?.combat ?? defaultCombatConfig();
  const dirty = JSON.stringify(draft) !== JSON.stringify(saved) || bindings !== JSON.stringify(saved.bindings, null, 2);
  const disabled = !connected || blocked || loading || saving || starting || importing;
  const editing = disabled || active;
  const choices=draft.dodge_only?dodgeScripts:scripts;
  const selectedScript=draft.dodge_only?draft.dodge_script:draft.script;
  async function load() {
    const ticket = ++generation.current;
    setAccount(null); setError(''); setResourceError('');setPreview(null);setImportConfirmed(false); setConfirmed(false); setLoading(true);
    const results = await Promise.allSettled([request({ type: 'account_config' }), request({ type: 'combat_scripts' })]);
    if (generation.current !== ticket) return;
    const config = results[0];
    if (config.status === 'fulfilled' && config.value.type === 'account_config') {
      const savedAccount=config.value.data;
      const current = savedAccount.instances.find(item => item.id === savedAccount.active_instance);
      if (!current) setError('当前账号不存在，请先检查账号设置');
      else { const settings = current.combat ?? defaultCombatConfig(); setAccount(config.value.data); setDraft(structuredClone(settings)); setBindings(JSON.stringify(settings.bindings, null, 2)); }
    } else setError(config.status === 'rejected' ? errorMessage(config.reason) : '引擎未返回账号配置');
    const resources = results[1];
    if (resources.status === 'fulfilled' && resources.value.type === 'combat_scripts') {setScripts(resources.value.data.battle);setDodgeScripts(resources.value.data.dodge);setScriptRevision(resources.value.data.revision);setImported(resources.value.data.imported);}
    else { setScripts([]); setResourceError(resources.status === 'rejected' ? errorMessage(resources.reason) : '引擎未返回战斗脚本'); }
    setLoading(false);
  }
  useEffect(() => {
    setAccount(null); setScripts([]);setDodgeScripts([]);setScriptRevision('');setPreview(null);setImportConfirmed(false);setImporting(false); setConfirmed(false); setLoading(false); setSaving(false); setStarting(false);
    if (connected) void load();
    return () => { generation.current++; };
  }, [connected]);
  useEffect(() => { setConfirmed(false); }, [snapshot?.window?.hwnd, snapshot?.window?.pid, snapshot?.capture_method, active]);
  function edit(change: Partial<CombatConfig>) { setDraft(current => ({ ...current, ...change })); setConfirmed(false); }
  async function save() {
    if (!account || !instance) return;
    const ticket = ++generation.current;
    setSaving(true); setConfirmed(false); setError('');
    try {
      const parsed: unknown = JSON.parse(bindings);
      if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error('按键映射必须是对象');
      const settings = { ...draft, bindings: parsed as CombatConfig['bindings'] };
      const result = await request({ type: 'update_account_config', params: { revision: account.revision, change: { type: 'save_combat', instance_id: instance.id, settings } } });
      if (generation.current !== ticket) return;
      if (result.type !== 'account_config') throw new Error('引擎未返回保存结果');
      const saved = result.data.instances.find(item => item.id === instance.id)?.combat;
      if (!saved) throw new Error('保存结果缺少战斗设置，请重新读取');
      setAccount(result.data); setDraft(structuredClone(saved)); setBindings(JSON.stringify(saved.bindings, null, 2));
    } catch (reason) { if (generation.current === ticket) setError(errorMessage(reason)); }
    finally { if (generation.current === ticket) setSaving(false); }
  }
  async function start() {
    if (!account || !instance || !confirmed || dirty || disabled || active) return;
    setStarting(true); setConfirmed(false);
    try { await onCommand({ type: 'start_combat', params: { instance_id: instance.id, revision: account.revision,script_revision:scriptRevision, allow_input: true, confirm_account: true } }); }
    finally { setStarting(false); }
  }
  async function importScripts(apply:boolean) {
    const ticket=++generation.current;setImporting(true);setConfirmed(false);setError('');
    try {
      if(apply&&(!preview||!importConfirmed))return;
      const result=await request(apply?{type:'apply_combat_script_import',params:{source:preview!.source,preview_id:preview!.preview_id}}:{type:'preview_combat_script_import',params:{source}});
      if(generation.current!==ticket)return;
      if(result.type!=='combat_script_import')throw new Error('引擎未返回脚本导入结果');
      setPreview(result.data);setImportConfirmed(false);
      if(apply){setPreview(null);setImporting(false);await load();}
    } catch(reason){if(generation.current===ticket){setError(errorMessage(reason));setPreview(null);setImportConfirmed(false);}}
    finally{if(generation.current===ticket)setImporting(false);}
  }
  const numbers: { key: keyof CombatConfig; label: string; min: number; max: number }[] = [
    { key: 'frame_interval_ms',label:'截图间隔（ms）',min:1,max:1000 },
    { key: 'start_delay_ms', label: '启动 / 继续延时（ms）', min: 0, max: 10000 },
    { key: 'timeout_ms', label: '运行超时（ms）', min: 1, max: 21600000 },
    { key: 'max_frame_age_ms', label: '允许的最大画面延迟（ms）', min: 20, max: 1000 },
    { key: 'tap_ms', label: '默认点按时长（ms）', min: 1, max: 1000 },
    { key: 'turn_sensitivity_milli', label: '转向灵敏度（1000 为原始位移）', min: 1, max: 100000 },
    { key: 'gamepad_turn_speed', label: '手柄转向摇杆偏移', min: 1, max: 32767 },
    { key: 'audio_threshold_milli', label: '声音阈值（100 对应 0.1）', min: 0, max: 10000 },
  ];
  const valid = numbers.every(({ key, min, max }) => Number.isInteger(draft[key]) && Number(draft[key]) >= min && Number(draft[key]) <= max);
  return <>
    <Card>
      <div className="card-title"><h2>自动战斗与闪避助手</h2><Badge appearance="outline">试运行 · 待验收</Badge></div>
      <p>在战斗画面启动。脚本、声音识别和按键共用当前游戏控制权，账号配置在启动时冻结。</p>
      {loading && <Spinner size="small" label="读取战斗配置与资源"/>}
      {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
      {resourceError && <MessageBar intent="error"><MessageBarBody>{resourceError}</MessageBarBody></MessageBar>}
      <div className="button-row"><Button disabled={disabled || active} onClick={() => void load()}>重新读取配置与资源</Button></div>
      {instance && <>
        <p>当前账号：{instance.name}（{instance.id}） · 修订 {account?.revision}</p>
        <Checkbox label="闪避助手模式" checked={draft.dodge_only} disabled={editing} onChange={(_, data) => edit({ dodge_only: data.checked === true })}/>
        <div className="settings-grid">
          <Field label="战斗脚本"><Select value={selectedScript} disabled={editing} onChange={(_, data) => edit(draft.dodge_only?{dodge_script:data.value}:{script:data.value})}>
            {!choices.includes(selectedScript) && <option value={selectedScript}>{selectedScript}（资源未提供）</option>}{choices.map(name => <option key={name} value={name}>{name}</option>)}
          </Select></Field>
          <Field label="输入方式"><Select value={draft.mode} disabled={editing} onChange={(_, data) => { edit({ mode: data.value as InputMode, bindings: {} }); setBindings('{}'); }}>{Object.entries(modes).map(([key, label]) => <option key={key} value={key}>{label}</option>)}</Select></Field>
          {numbers.map(({ key, label, min, max }) => <Field key={key} label={label}><Input type="number" min={min} max={max} step={1} value={String(draft[key])} disabled={editing} onChange={(_, data) => edit({ [key]: Number(data.value) })}/></Field>)}
        </div>
        <Checkbox label="启用声音闪避" checked={draft.audio} disabled={editing} onChange={(_, data) => edit({ audio: data.checked === true })}/>
        <Checkbox label="使用 DirectML 识别" checked={draft.directml_device!==null} disabled={editing} onChange={(_,data)=>edit({directml_device:data.checked===true?0:null})}/>
        {draft.directml_device!==null&&<Field label="DirectML 设备编号"><Input type="number" min={0} max={15} value={String(draft.directml_device)} disabled={editing} onChange={(_,data)=>edit({directml_device:Number(data.value)})}/></Field>}
        <Checkbox label="优先使用合并脚本" checked={draft.use_merged} disabled={editing} onChange={(_,data)=>edit({use_merged:data.checked===true})}/>
        <Checkbox label="启用自动终结技状态（由脚本处理）" checked={draft.auto_ultimate} disabled={editing} onChange={(_,data)=>edit({auto_ultimate:data.checked===true})}/>
        <details><summary>自定义按键映射</summary><p>空对象使用游戏默认键位。按逻辑名称设置按键、鼠标、摇杆或扳机组合；更换输入方式会恢复默认映射。</p><Field label="按键映射 JSON"><Textarea value={bindings} resize="vertical" rows={6} disabled={editing} onChange={(_, data) => { setBindings(data.value); setConfirmed(false); }}/></Field></details>
        <div className="button-row"><Button disabled={editing || !dirty || !valid} onClick={() => void save()}>{saving ? '保存中…' : '保存战斗设置'}</Button>{dirty && <span>有未保存修改</span>}</div>
        <Checkbox label={`确认账号 ${instance.name}，允许本次战斗输入`} checked={confirmed} disabled={editing || dirty || !!resourceError || !snapshot?.window} onChange={(_, data) => setConfirmed(data.checked === true)}/>
        <Button appearance="primary" disabled={editing || dirty || !valid || !confirmed || !!resourceError || !snapshot?.window || !choices.includes(selectedScript)||!scriptRevision} onClick={() => void start()}>{starting ? '启动中…' : '开始战斗'}</Button>
      </>}
    </Card>
    <Card><h2>导入旧战斗脚本</h2><p>先校验全部脚本和模板，再保存独立快照。导入后，同名脚本采用此快照；运行前需重新读取并确认。</p>
      {imported&&<p>当前导入快照：<code>{imported.slice(0,12)}</code></p>}
      <Field label="旧项目或 config 目录"><Input value={source} disabled={editing} onChange={(_,data)=>{setSource(data.value);setPreview(null);setImportConfirmed(false);}}/></Field>
      <Button disabled={editing||!source.trim()} onClick={()=>void importScripts(false)}>预览脚本导入</Button>
      {preview&&<><p>{preview.battle.length} 个战斗脚本、{preview.dodge.length} 个闪避脚本，{preview.files} 个文件，共 {preview.bytes} 字节。</p><details><summary>核对脚本名称</summary><p>战斗：{preview.battle.join('、')}</p><p>闪避：{preview.dodge.join('、')}</p></details><Checkbox label="确认采用此脚本快照" checked={importConfirmed} disabled={editing} onChange={(_,data)=>setImportConfirmed(data.checked===true)}/><Button disabled={editing||!importConfirmed} onClick={()=>void importScripts(true)}>确认导入脚本</Button></>}
    </Card>
    {run && combat && <Card>
      <div className="card-title"><h2>战斗状态</h2><Badge>{runStateNames[run.state]}</Badge></div>
      <p>{run.message}</p><p>运行账号：{run.context?.instance_name} · 输入方式：{modes[combat.config.mode]} · {combat.config.dodge_only ? combat.config.dodge_script : combat.config.script}</p>
      <p>{combat.inputs_released ? '输入已释放' : '运行中，等待输入收尾'} · {combat.in_battle ? '当前为战斗画面' : '等待有效战斗画面'}</p>
      <p>角色：{combat.team.filter(Boolean).join(' / ') || '等待识别'}</p>
      <p>画面 {combat.frames} 帧 · 丢弃过期画面 {combat.stale_frames} 帧 · 最近画面延迟 {combat.frame_age_ms} ms · 动作 {combat.actions} 次 · 抢占 {combat.preemptions} 次</p>
      {combat.audio_correlation !== null && <p>声音相关性：{combat.audio_correlation.toFixed(4)}</p>}
      <p>当前条件：{combat.handler || '等待命中'}<br/>最近动作：{combat.action || '尚未执行'}</p>
      <div className="button-row">
        <Button disabled={disabled || run.state !== 'running'} onClick={() => onCommand({ type: 'pause', params: { run_id: run.id } })}>暂停战斗</Button>
        <Button disabled={disabled || !!snapshot?.record_error || run.state !== 'paused'} onClick={() => onCommand({ type: 'resume', params: { run_id: run.id } })}>继续战斗</Button>
        <Button disabled={disabled || !active || run.state === 'stopping'} onClick={() => onCommand({ type: 'stop', params: { run_id: run.id } })}>停止战斗</Button>
      </div>
      <details><summary>状态时间</summary><ul>{Object.entries(combat.states).map(([name, time]) => <li key={name}>{name} · {time?.toFixed(3)} s</li>)}</ul></details><p className="muted">运行 ID：{run.id}</p>
    </Card>}
    <RunSaveRecovery error={snapshot?.record_error} disabled={disabled} onCommand={onCommand}/>
  </>;
}
