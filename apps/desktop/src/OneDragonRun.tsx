import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import type { AccountConfig, AfterDoneAction, AppDefinition, Command, Snapshot } from './generated/protocol';
import { afterDoneOptions, gameRegionOptions, trialAppOptions } from './generated/protocol';
import { errorMessage, request } from './bridge';
import { GroupRuns } from './GroupRuns';
import { GroupRunStatus, groupInputsReleased, isRunActive, RunSaveRecovery } from './GroupRunStatus';

type Destination = 'home' | 'accounts' | 'history' | 'native';

export function OneDragonRun({ connected, blocked, apps, snapshot, onCommand, onNavigate, standalone = false }: {
  connected: boolean; blocked: boolean; apps: AppDefinition[]; snapshot: Snapshot | null;
  standalone?: boolean; onCommand: (command: Command) => Promise<void>; onNavigate: (page: Destination) => void;
}) {
  const [config, setConfig] = useState<AccountConfig | null>(null);
  const [groupId, setGroupId] = useState('');
  const [standaloneChoice, setStandaloneChoice] = useState('');
  const [selection, setSelection] = useState<{ key: string; appIds: string[] } | null>(null);
  const [rerunKey, setRerunKey] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [afterDoneChoice, setAfterDoneChoice] = useState<AfterDoneAction>('none');
  const [savingAfterDone, setSavingAfterDone] = useState(false);
  const generation = useRef(0);
  const run = snapshot?.run;
  const active = isRunActive(run);
  const instance = config?.instances.find(item => item.id === config.active_instance);
  const runningGroup = active && run?.context?.instance_id === instance?.id ? run?.context?.group_id : undefined;
  const selectedId = runningGroup ?? groupId;
  const group = instance?.groups.find(item => item.id === selectedId) ?? instance?.groups[0];
  const availableApps: string[] = trialAppOptions.map(app => app.id);
  const standaloneApps = group?.apps.filter(item => availableApps.includes(item.app_id)) ?? [];
  const standaloneAppId = standaloneApps.find(item => item.app_id === standaloneChoice)?.app_id ?? standaloneApps[0]?.app_id;
  const selectionKey = `${instance?.id}/${group?.id}/${config?.revision}`;
  const appIds = selection?.key === selectionKey ? selection.appIds : undefined;
  const rerunCompleted = rerunKey === selectionKey;
  const groupRun = run?.context ? run : null;
  const unreleased = !!groupRun && !active && !groupInputsReleased(groupRun);
  const disabled = !connected || blocked;

  async function load() {
    const requested = ++generation.current;
    setConfig(null); setSelection(null); setRerunKey(null); setError(''); setLoading(true); setSavingAfterDone(false);
    try {
      const value = await request({ type: 'account_config' });
      if (generation.current !== requested) return;
      if (value.type !== 'account_config') throw new Error('引擎未返回账号配置');
      if (!value.data.instances.some(item => item.id === value.data.active_instance)) throw new Error('当前账号不在配置中，请检查账号配置。');
      setConfig(value.data);
    } catch (reason) { if (generation.current === requested) setError(errorMessage(reason)); }
    finally { if (generation.current === requested) setLoading(false); }
  }

  useEffect(() => {
    setConfig(null); setGroupId(''); setSelection(null); setRerunKey(null); setError(''); setLoading(false); setSavingAfterDone(false);
    if (connected) void load();
    return () => { generation.current++; };
  }, [connected, standalone]);

  useEffect(() => { if (group) setGroupId(group.id); }, [group?.id]);
  useEffect(() => { setAfterDoneChoice(config?.general.after_done ?? 'none'); }, [config]);

  async function saveAfterDone() {
    if (!config || disabled || active || savingAfterDone || snapshot?.record_error) return;
    const requested = generation.current;
    setSavingAfterDone(true); setError('');
    try {
      const value = await request({ type: 'update_account_config', params: { revision: config.revision, change: { type: 'set_after_done', action: afterDoneChoice } } });
      if (generation.current !== requested) return;
      if (value.type !== 'account_config') throw new Error('引擎未返回保存结果');
      setConfig(value.data);
    } catch (reason) {
      if (generation.current !== requested) return;
      if (typeof reason === 'object' && reason && 'code' in reason && reason.code === 'config_conflict') {
        const reloaded = generation.current + 1;
        await load(); if (generation.current === reloaded) setError('配置已变化，已重新读取。请核对结束后动作并重新保存。');
      } else setError(errorMessage(reason));
    } finally { if (generation.current === requested) setSavingAfterDone(false); }
  }

  return <>
    <div className="page-heading"><div><div className="eyebrow">ONE DRAGON</div><h1>{standalone ? '独立应用运行' : '一条龙运行'}</h1><p>{standalone ? '选择参数来源和一个应用，核对计划后运行。' : '选择当前账号的应用组，核对计划后运行。'}</p></div><Button disabled={disabled || loading || savingAfterDone} onClick={() => void load()}>读取最新运行配置</Button></div>
    {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
    <Card className="one-dragon-current" aria-label="当前任务">
      <div className="card-title"><h2>当前任务</h2><Badge appearance="outline">{!connected ? '引擎未连接' : active ? '占用运行槽' : snapshot?.record_error ? '记录待保存' : unreleased ? '等待输入收尾' : '空闲'}</Badge></div>
      <RunSaveRecovery error={snapshot?.record_error} disabled={disabled} onCommand={onCommand}/>
      {groupRun ? <>
        <p>{groupRun.context!.instance_name} · {groupRun.context!.group_name ?? groupRun.context!.group_id} · 修订 {groupRun.context!.config_revision}</p>
        {groupRun.context!.game_day && <p>游戏日 {groupRun.context!.game_day.date}</p>}
        {groupRun.context!.standalone && <Badge appearance="outline">独立应用运行</Badge>}
        <p>本次应用：{groupRun.context!.app_ids.map(id => apps.find(app => app.id === id)?.name ?? id).join(' → ')}</p>
        {groupRun.context!.rerun_completed && <p>本次重新运行：包括本游戏日已完成的应用。</p>}
        <GroupRunStatus run={groupRun} disabled={disabled} recordError={!!snapshot?.record_error} onCommand={onCommand}/>
        {unreleased && <MessageBar intent="error"><MessageBarBody>上次任务尚未确认输入释放，请先处理输入收尾再开始新任务。</MessageBarBody></MessageBar>}
      </> : active ? <>
        <p>原生诊断任务占用运行槽：{run?.message}</p><p className="run-message">运行 ID：{run?.id}</p>
        <Button onClick={() => onNavigate('native')}>查看原生诊断任务</Button>
      </> : <p>{connected ? standalone ? '当前没有账号任务。预览下方单应用计划并确认后开始。' : '当前没有应用组任务。预览下方计划并确认后开始。' : '连接桌面引擎后可查看当前任务。'}</p>}
    </Card>
    {!connected && <Card><p>连接桌面引擎和游戏窗口后可读取运行配置。</p><Button onClick={() => onNavigate('home')}>前往总览连接游戏</Button></Card>}
    {loading && <Spinner size="small" label="读取运行配置"/>}
    {instance && <Card className="one-dragon-config" aria-label="当前账号运行配置">
      <div className="card-title"><h2>{instance.name}</h2><Badge appearance="outline">当前账号 · {String(instance.id).padStart(4, '0')}</Badge></div>
      <p>{gameRegionOptions.find(option => option.value === instance.game_region)?.label} · 配置修订 {config!.revision}</p>
      <Field label={standalone ? '设置来源应用组' : '运行应用组'}><Select value={group?.id ?? ''} disabled={disabled || loading || active} onChange={(_, data) => { setSelection(null); setRerunKey(null); setStandaloneChoice(''); setGroupId(data.value); }}>{instance.groups.map(item => <option key={item.id} value={item.id}>{item.name ?? (item.id === 'one_dragon' ? '一条龙' : item.id)}</option>)}</Select></Field>
      {group && standalone && <>
        <Field label="独立运行应用"><Select value={standaloneAppId ?? ''} disabled={disabled || loading || active} onChange={(_, data) => setStandaloneChoice(data.value)}>
          {!standaloneApps.length && <option value="">该组没有可运行的应用</option>}
          {group.apps.map(item => <option key={item.app_id} value={item.app_id} disabled={!availableApps.includes(item.app_id)}>{apps.find(app => app.id === item.app_id)?.name ?? item.app_id}{!availableApps.includes(item.app_id) ? '（待迁移）' : ''}</option>)}
        </Select></Field>
        <p>每次只运行所选应用，包括本游戏日已完成或保存为禁用的应用。参数来自所选组，成功会更新每日记录；不会执行一条龙结束后动作。</p>
      </>}
      {group && !standalone && <>
        <p>{group.run_policy === 'skip_daily_success' ? '跳过本游戏日已完成的应用' : '每次运行'} · 启用 {group.apps.filter(item => item.enabled).length} / {group.apps.length} 项</p>
        <Checkbox label="仅本次重新运行已完成应用" checked={rerunCompleted} disabled={disabled || loading || active} onChange={(_, data) => setRerunKey(data.checked === true ? selectionKey : null)}/>
        {rerunCompleted && <p>本次所选应用全部重新执行，不增加游戏内领取次数。保存的运行策略及每日记录保留，刷新配置或离开页面后恢复默认。</p>}
        <Checkbox label="仅本次选择应用" checked={appIds !== undefined} disabled={disabled || loading || active} onChange={(_, data) => setSelection(data.checked === true ? { key: selectionKey, appIds: group.apps.filter(item => item.enabled).map(item => item.app_id) } : null)}/>
        {appIds !== undefined && <>
          <p>本次选择 {appIds.length} 项，按保存顺序执行。临时选择不保存，刷新配置或离开页面后恢复默认。</p>
          <div className="button-row"><Button disabled={disabled || active} onClick={() => setSelection({ key: selectionKey, appIds: group.apps.map(item => item.app_id) })}>选择全部应用</Button><Button disabled={disabled || active} onClick={() => setSelection({ key: selectionKey, appIds: group.apps.filter(item => item.enabled).map(item => item.app_id) })}>恢复已保存的启用项</Button></div>
        </>}
        <ol className="one-dragon-apps" aria-label="保存的应用顺序">{group.apps.map(item => { const name = apps.find(app => app.id === item.app_id)?.name ?? item.app_id; return <li key={item.app_id}>{appIds === undefined ? <span>{name}</span> : <Checkbox label={name} aria-label={`本次运行${name}`} checked={appIds.includes(item.app_id)} disabled={disabled || loading || active} onChange={(_, data) => setSelection({ key: selectionKey, appIds: group.apps.filter(app => app.app_id === item.app_id ? data.checked === true : appIds.includes(app.app_id)).map(app => app.app_id) })}/>}<Badge appearance="outline">{appIds === undefined ? item.enabled ? '已启用' : '未启用' : item.enabled ? '保存：启用' : '保存：禁用'}</Badge></li>; })}</ol>
      </>}
      <p>请确认实际游戏登录账号。更换当前账号、启用项和顺序请进入账号与应用组设置。</p>
      {!standalone && <>
      <Field label="结束后动作"><Select value={afterDoneChoice} disabled={disabled || active || savingAfterDone || !!snapshot?.record_error} onChange={(_, data) => setAfterDoneChoice(data.value as AfterDoneAction)}>{afterDoneOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</Select></Field>
      <Button disabled={disabled || active || savingAfterDone || !!snapshot?.record_error || afterDoneChoice === (config!.general.after_done ?? 'none')} onClick={() => void saveAfterDone()}>保存结束后动作</Button>
      <p>所有账号共用；本次计划正常完成、记录保存且输入释放后开始 60 秒倒计时，可在任意页面取消。运行采用启动时的保存值；失败、停止和独立诊断不执行。</p>
      {afterDoneChoice !== (config!.general.after_done ?? 'none') && <p>结束后动作尚未保存，请保存或恢复原值后再预览运行。</p>}
      </>}
      {!snapshot?.window && <MessageBar intent="info"><MessageBarBody>尚未连接游戏窗口，可以先预览计划，连接后需重新预览并确认。</MessageBarBody></MessageBar>}
      <div className="button-row"><Button onClick={() => onNavigate('accounts')}>编辑账号与应用组</Button><Button onClick={() => onNavigate('home')}>前往总览连接游戏</Button><Button onClick={() => onNavigate('history')}>查看完整运行记录</Button></div>
    </Card>}
    {config && instance && group && (!standalone || standaloneAppId) && <GroupRuns key={`${standalone}/${instance.id}/${group.id}/${config.revision}`} instanceId={instance.id} groupId={group.id} revision={config.revision} snapshot={snapshot} connected={connected} blocked={blocked || loading || unreleased || savingAfterDone || (!standalone && afterDoneChoice !== (config.general.after_done ?? 'none'))} onCommand={onCommand} showStatus={false} appIds={appIds} rerunCompleted={rerunCompleted} standaloneAppId={standalone ? standaloneAppId : undefined}/>}
  </>;
}
