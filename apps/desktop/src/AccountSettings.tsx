import { WorldPatrolSettings } from './WorldPatrolSettings';
import { LostVoidSettings } from './LostVoidSettings';
import { ChargePlanSettings } from './ChargePlanSettings';
import { SuibianTempleSettings } from './SuibianTempleSettings';
import { DismantleSettings } from './DismantleSettings';
import { RandomPlaySettings } from './RandomPlaySettings';
import { IntelBoardSettings } from './IntelBoardSettings';
import { TeamSettings } from './TeamSettings';
import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import type { AccountConfig, AppDefinition, AppGroup, Command, ConfigChange, GameRegion, RunPolicy, Snapshot } from './generated/protocol';
import { coffeeDefaultConfig, gameRegionOptions, runPolicyOptions, trialAppOptions } from './generated/protocol';
import { DailySettings } from './DailySettings';
import { errorMessage, request } from './bridge';
import { GroupRuns } from './GroupRuns';
import { ConfigImport } from './ConfigImport';
import { ConfigBackups } from './ConfigBackups';
import { CoffeeSettings } from './CoffeeSettings';
import { GameLaunchEditor } from './GameLaunch';
import { RunNotificationEditor } from './RunNotifications';

const groupName = (group: AppGroup) => group.name ?? (group.id === 'one_dragon' ? '一条龙' : group.id);
const validName = (name: string) => !!name.trim() && [...name].length <= 80 && !/[\u0000-\u001f\u007f-\u009f]/.test(name);

export function AccountSettings({ connected, active, blocked, apps, snapshot, onCommand, onImported }: { connected: boolean; active: boolean; blocked: boolean; apps: AppDefinition[]; snapshot: Snapshot | null; onCommand: (command: Command) => Promise<void>; onImported: (config: AccountConfig) => void }) {
  const [config, setConfig] = useState<AccountConfig | null>(null);
  const [instanceId, setInstanceId] = useState(0);
  const [groupId, setGroupId] = useState('one_dragon');
  const [name, setName] = useState('');
  const [enabled, setEnabled] = useState(true);
  const [region, setRegion] = useState<GameRegion>('cn');
  const [runPolicy, setRunPolicy] = useState<RunPolicy>('always');
  const [newName, setNewName] = useState('');
  const [groupLabel, setGroupLabel] = useState('');
  const [newGroupName, setNewGroupName] = useState('');
  const [addAppId, setAddAppId] = useState('');
  const [deleting, setDeleting] = useState(false);
  const [deleteConfirmed, setDeleteConfirmed] = useState(false);
  const [deletingAccount, setDeletingAccount] = useState(false);
  const [accountDeleteConfirmed, setAccountDeleteConfirmed] = useState(false);
  const [busy, setBusy] = useState(false);
  const [importBusy, setImportBusy] = useState(false);
  const [backupBusy, setBackupBusy] = useState(false);
  const [pickerBusy, setPickerBusy] = useState(false);
  const [error, setError] = useState('');
  const [message, setMessage] = useState('');
  const generation = useRef(0);
  const working = useRef(false);
  const instance = config?.instances.find(item => item.id === instanceId);
  const group = instance?.groups.find(item => item.id === groupId);
  const recordUnsaved = !!snapshot?.record_error;
  const disabled = !connected || blocked || busy || importBusy || backupBusy || pickerBusy || active || recordUnsaved;

  function accept(value: AccountConfig) {
    setConfig(value);
    setInstanceId(current => value.instances.some(item => item.id === current) ? current : value.active_instance);
    setDeleting(false); setDeleteConfirmed(false);
    setDeletingAccount(false); setAccountDeleteConfirmed(false);
  }

  async function load() {
    const result = await request({ type: 'account_config' });
    if (result.type !== 'account_config') throw new Error('引擎未返回账号配置');
    return result.data;
  }

  useEffect(() => {
    const requested = ++generation.current;
    setConfig(null); setError(''); setMessage(''); setBusy(false);
    working.current = false;
    setNewGroupName(''); setDeleting(false); setDeleteConfirmed(false);
    if (connected) load().then(value => {
      if (generation.current === requested) accept(value);
    }).catch(reason => { if (generation.current === requested) setError(errorMessage(reason)); });
    return () => { generation.current++; };
  }, [connected]);

  const savedTeamRevision = snapshot?.run?.daily?.find(item => item.team_check?.saved_revision != null)?.team_check?.saved_revision;
  useEffect(() => {
    if (!connected || savedTeamRevision == null || !config || config.revision >= savedTeamRevision || working.current) return;
    const requested = generation.current;
    load().then(value => { if (generation.current === requested) accept(value); })
      .catch(reason => { if (generation.current === requested) setError(errorMessage(reason)); });
  }, [connected, savedTeamRevision, config?.revision]);

  useEffect(() => {
    setName(instance?.name ?? ''); setEnabled(instance?.active_in_od ?? true);
  }, [instance?.id, instance?.name, instance?.active_in_od]);

  useEffect(() => { setRegion(instance?.game_region ?? 'cn'); }, [instance?.id, instance?.game_region]);
  useEffect(() => { setRunPolicy(group?.run_policy ?? 'always'); }, [instance?.id, group?.id, group?.run_policy]);
  useEffect(() => { setGroupLabel(group ? groupName(group) : ''); }, [instance?.id, group?.id, config?.revision]);
  useEffect(() => {
    setDeleting(false); setDeleteConfirmed(false);
    setDeletingAccount(false); setAccountDeleteConfirmed(false);
  }, [connected, instanceId, groupId, config?.revision, active, blocked, importBusy, recordUnsaved]);
  useEffect(() => { setNewGroupName(''); }, [instanceId]);
  useEffect(() => { setAddAppId(''); }, [instanceId, groupId, config?.revision]);

  useEffect(() => {
    setGroupId(current => instance?.groups.some(item => item.id === current) ? current : instance?.groups[0]?.id ?? 'one_dragon');
  }, [instance?.id, instance?.groups]);

  async function update(change: ConfigChange) {
    if (!config || disabled || working.current) return;
    const requested = generation.current;
    working.current = true;
    setBusy(true); setError(''); setMessage('');
    try {
      const result = await request({ type: 'update_account_config', params: { revision: config.revision, change } });
      if (generation.current !== requested) return;
      if (result.type !== 'account_config') throw new Error('引擎未返回保存结果');
      accept(result.data);
      if (change.type === 'create_instance') setNewName('');
      if (change.type === 'copy_instance') {
        const created = result.data.instances.find(item => !config.instances.some(old => old.id === item.id));
        if (created) setInstanceId(created.id);
        setNewName('');
        setMessage('账号配置已复制，副本默认不参与一条龙。请核对区服、启用项和实际游戏登录。');
      } else if (change.type === 'delete_instance') setMessage('账号配置已删除，历史、导入原文和账号编号已保留。');
      else if (change.type === 'reorder_instances') setMessage('账号顺序已保存，当前账号保持不变。');
      else if (change.type === 'reorder_groups') setMessage('应用组顺序已保存。');
      else if (change.type === 'save_coffee') setMessage('咖啡设置已保存，可在原生诊断中采用所选组参数。');
      else if (change.type === 'save_daily') setMessage('日常设置已保存。');
      else if (change.type === 'save_dismantle') setMessage('拆解设置已保存。');
      else if (change.type === 'save_random_play') setMessage('录像店设置已保存。');
      else if (change.type === 'save_charge_plan') setMessage('体力计划已保存。');
      else if (change.type === 'save_suibian_temple') setMessage('随便观设置已保存。');
      else if (change.type === 'save_world_patrol') setMessage('锄大地设置已保存。');
      else if (change.type === 'save_lost_void') setMessage('迷失之地设置已保存。');
      else if (change.type === 'save_intel_board') setMessage('情报板设置已保存。');
      else if (change.type === 'save_teams') setMessage('预备编队已保存。');
      else if (change.type === 'save_game_launch') setMessage('游戏启动设置已保存，可在总览核对当前账号后手动启动。');
      else if (change.type === 'save_run_notifications') setMessage('运行提醒设置已保存，下次由桌面启动的应用组采用该设置。');
      if (change.type === 'create_group') {
        const before = config.instances.find(item => item.id === change.instance_id)?.groups ?? [];
        const created = result.data.instances.find(item => item.id === change.instance_id)?.groups.find(item => !before.some(old => old.id === item.id));
        if (created) setGroupId(created.id);
        setNewGroupName('');
        setMessage(change.source_group_id ? '应用组已复制，请核对启用项和运行策略。' : '应用组已新建，所有应用默认禁用。');
      } else if (change.type === 'rename_group') setMessage('应用组名称已保存。');
      else if (change.type === 'delete_group') setMessage('应用组已删除，账号历史和导入归档仍保留。');
    } catch (reason) {
      if (generation.current !== requested) return;
      setDeletingAccount(false); setAccountDeleteConfirmed(false);
      if (typeof reason === 'object' && reason && 'code' in reason && reason.code === 'config_conflict') {
        setDeleting(false); setDeleteConfirmed(false);
        try { const value = await load(); if (generation.current === requested) { accept(value); setError('配置已被其他窗口或进程修改，已加载最新配置。请核对后重新编辑。'); } }
        catch (refreshError) { if (generation.current === requested) { setConfig(null); setError(`配置冲突，读取最新配置失败：${errorMessage(refreshError)}`); } }
      } else setError(errorMessage(reason));
    } finally { if (generation.current === requested) { working.current = false; setBusy(false); } }
  }

  function move(index: number, offset: number) {
    if (!group || !instance) return;
    const items = [...group.apps];
    [items[index], items[index + offset]] = [items[index + offset], items[index]];
    void update({ type: 'save_group', instance_id: instance.id, group_id: group.id, apps: items });
  }

  function moveAccount(offset: number) {
    if (!config || !instance) return;
    const ids = config.instances.map(item => item.id); const index = ids.indexOf(instance.id);
    if (index + offset < 0 || index + offset >= ids.length) return;
    [ids[index], ids[index + offset]] = [ids[index + offset], ids[index]];
    void update({ type: 'reorder_instances', instance_ids: ids });
  }

  function moveGroup(offset: number) {
    if (!instance || !group) return;
    const ids = instance.groups.map(item => item.id); const index = ids.indexOf(group.id);
    if (index + offset < 0 || index + offset >= ids.length) return;
    [ids[index], ids[index + offset]] = [ids[index + offset], ids[index]];
    void update({ type: 'reorder_groups', instance_id: instance.id, group_ids: ids });
  }

  return <>
    <div className="page-heading"><div><div className="eyebrow">ACCOUNTS AND APPLICATIONS</div><h1>账号与应用组</h1><p>管理本地账号配置和应用顺序。切换配置后，请自行确认游戏登录账号。</p></div><Button disabled={!connected || busy || importBusy || backupBusy || pickerBusy || blocked} onClick={async () => {
      const requested = generation.current;
      setBusy(true); setError(''); setMessage(''); setDeleting(false); setDeleteConfirmed(false);
      try { const value = await load(); if (generation.current === requested) accept(value); } catch (reason) { if (generation.current === requested) setError(errorMessage(reason)); } finally { if (generation.current === requested) setBusy(false); }
    }}>读取最新配置</Button></div>
    {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
    {message && <MessageBar intent="success"><MessageBarBody>{message}</MessageBarBody></MessageBar>}
    {active && <MessageBar intent="info"><MessageBarBody>任务仍在运行或收尾，请停止任务后再修改账号及应用组。</MessageBarBody></MessageBar>}
    {recordUnsaved && <MessageBar intent="warning"><MessageBarBody>运行记录尚未保存，请先在账号运行历史中重试保存，再修改配置。</MessageBarBody></MessageBar>}
    <ConfigImport connected={connected} active={active} blocked={blocked || busy || backupBusy || pickerBusy} current={config} apps={apps} onBusy={setImportBusy} onImported={value => { accept(value); setError(''); onImported(value); }}/>
    <ConfigBackups connected={connected} active={active} blocked={blocked || busy || importBusy || pickerBusy} current={config} apps={apps} onBusy={setBackupBusy} onLoaded={value => { accept(value); setError(''); onImported(value); }}/>
    {!config && <Card><p>{connected ? '正在读取账号配置。读取失败时可点击“读取最新配置”重试。' : '连接桌面引擎后可读取和编辑本地配置。'}</p></Card>}
    {config && <>
      <Card><div className="card-title"><h2>账号配置</h2><Badge appearance="outline">修订 {config.revision}</Badge></div>
        <p>当前账号：{config.instances.find(item => item.id === config.active_instance)?.name}</p>
        <div className="input-fields">
          <Field label="编辑账号"><Select value={instanceId} disabled={busy || importBusy || backupBusy || pickerBusy} onChange={(_, data) => { setInstanceId(Number(data.value)); setMessage(''); }}>{config.instances.map(item => <option key={item.id} value={item.id}>{item.name}{item.id === config.active_instance ? '（当前）' : ''}</option>)}</Select></Field>
          <Field label="账号名称"><Input value={name} maxLength={80} disabled={disabled} onChange={(_, data) => setName(data.value)}/></Field>
        </div>
        <Checkbox label="参与一条龙" checked={enabled} disabled={disabled} onChange={(_, data) => setEnabled(data.checked === true)}/>
        <Field label="账号区服"><Select value={region} disabled={disabled} onChange={(_, data) => setRegion(data.value as GameRegion)}>{gameRegionOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</Select></Field>
        <p>区服用于计算每日重置日期，启用按日跳过前请确认与游戏一致。</p>
        <Button disabled={disabled || !instance || region === instance.game_region} onClick={() => instance && void update({ type: 'set_game_region', instance_id: instance.id, game_region: region })}>保存区服</Button>
        <div className="button-row"><Button disabled={disabled || !instance || !name.trim()} onClick={() => instance && void update({ type: 'update_instance', instance_id: instance.id, name, active_in_od: enabled })}>保存账号</Button>
          <Button appearance="primary" disabled={disabled || !instance || instance.id === config.active_instance} onClick={() => instance && void update({ type: 'switch_instance', instance_id: instance.id })}>设为当前账号</Button>
          {busy && <Spinner size="tiny" label="保存配置"/>}
        </div>
        <div className="button-row">
          <Button disabled={disabled || !instance || config.instances[0].id === instance.id} onClick={() => moveAccount(-1)}>上移账号</Button>
          <Button disabled={disabled || !instance || config.instances.at(-1)?.id === instance.id} onClick={() => moveAccount(1)}>下移账号</Button>
          <Button disabled={disabled || !instance || config.instances.length <= 1 || instance.id === config.active_instance} onClick={() => { setDeletingAccount(true); setAccountDeleteConfirmed(false); setDeleting(false); setDeleteConfirmed(false); }}>删除账号配置</Button>
        </div>
        {deletingAccount && instance && <div role="group" aria-label="删除账号确认">
          <p>删除“{instance.name}”（编号 {instance.id}）的本地账号配置及其 {instance.groups.length} 个应用组？历史和导入原文保留，编号不会分配给新账号；保存前会备份。</p>
          <Checkbox label={`确认删除账号 ${instance.name}`} checked={accountDeleteConfirmed} disabled={disabled} onChange={(_, data) => setAccountDeleteConfirmed(data.checked === true)}/>
          <div className="button-row"><Button appearance="primary" disabled={disabled || !accountDeleteConfirmed || instance.id === config.active_instance || config.instances.length <= 1} onClick={() => void update({ type: 'delete_instance', instance_id: instance.id, confirm: true })}>确认删除账号配置</Button><Button disabled={busy} onClick={() => { setDeletingAccount(false); setAccountDeleteConfirmed(false); }}>取消删除账号</Button></div>
        </div>}
        <p>当前账号和最后一个账号不能删除。排序只改变本地展示与编排顺序，不会切换游戏登录。</p>
        <div className="account-create"><Field label="新增账号名称"><Input value={newName} maxLength={80} disabled={disabled} onChange={(_, data) => setNewName(data.value)}/></Field><Button disabled={disabled || !validName(newName) || config.instances.length >= 128} onClick={() => void update({ type: 'create_instance', name: newName })}>新增账号</Button><Button disabled={disabled || !instance || !validName(newName) || config.instances.length >= 128} onClick={() => instance && void update({ type: 'copy_instance', instance_id: instance.id, name: newName })}>复制账号配置</Button></div>
        <p>复制保留所选账号的区服、应用组顺序、启用项和运行策略；使用新编号，副本默认不参与一条龙。凭据、历史和每日完成记录不复制。</p>
      </Card>
      {instance && <GameLaunchEditor key={`launch-${instance.id}-${config.revision}-${connected}`} settings={instance.game_launch} disabled={disabled} onPickerBusy={setPickerBusy} onSave={settings => void update({ type: 'save_game_launch', instance_id: instance.id, settings })}/>}
      {instance && <RunNotificationEditor key={`notifications-${instance.id}-${config.revision}-${connected}`} settings={instance.run_notifications} disabled={disabled} onSave={settings => void update({ type: 'save_run_notifications', instance_id: instance.id, settings })}/>}
      {instance && <TeamSettings key={`teams/${instance.id}/${config.revision}`} settings={instance.predefined_teams} disabled={disabled} onSave={settings => void update({ type: 'save_teams', instance_id: instance.id, settings })}/>}
      {instance && <Card><div className="card-title"><h2>应用组顺序</h2><Badge appearance="outline">{group?.apps.length ?? 0} 项</Badge></div>
        <Field label="应用组"><Select value={groupId} disabled={busy || importBusy || backupBusy || pickerBusy} onChange={(_, data) => { setGroupId(data.value); setMessage(''); }}>{instance.groups.map(item => <option key={item.id} value={item.id}>{groupName(item)}</option>)}</Select></Field>
        <div className="button-row"><Button disabled={disabled || !group || instance.groups[0].id === group.id} onClick={() => moveGroup(-1)}>上移应用组</Button><Button disabled={disabled || !group || instance.groups.at(-1)?.id === group.id} onClick={() => moveGroup(1)}>下移应用组</Button></div>
        <div className="account-create"><Field label="应用组名称"><Input value={groupLabel} maxLength={80} disabled={disabled || !group} onChange={(_, data) => { setGroupLabel(data.value); setMessage(''); }}/></Field>
          <Button disabled={disabled || !group || !validName(groupLabel) || groupLabel.trim() === groupName(group)} onClick={() => group && void update({ type: 'rename_group', instance_id: instance.id, group_id: group.id, name: groupLabel })}>保存组名</Button>
          <Button disabled={disabled || !group || instance.groups.length <= 1} onClick={() => { setDeleting(true); setDeleteConfirmed(false); setDeletingAccount(false); setAccountDeleteConfirmed(false); setMessage(''); }}>删除应用组</Button>
        </div>
        {deleting && group && <div role="group" aria-label="删除应用组确认">
          <p>删除“{instance.name}”中的“{groupName(group)}”？账号历史和旧配置归档仍保留，修改前会备份配置。</p>
          <Checkbox label={`确认删除 ${groupName(group)}`} checked={deleteConfirmed} disabled={disabled} onChange={(_, data) => setDeleteConfirmed(data.checked === true)}/>
          <div className="button-row"><Button appearance="primary" disabled={disabled || !deleteConfirmed || instance.groups.length <= 1} onClick={() => void update({ type: 'delete_group', instance_id: instance.id, group_id: group.id })}>确认删除应用组</Button><Button disabled={busy} onClick={() => { setDeleting(false); setDeleteConfirmed(false); }}>取消删除</Button></div>
        </div>}
        <div className="account-create"><Field label="新应用组名称"><Input value={newGroupName} maxLength={80} disabled={disabled || instance.groups.length >= 64} onChange={(_, data) => setNewGroupName(data.value)}/></Field>
          <Button disabled={disabled || !validName(newGroupName) || instance.groups.length >= 64} onClick={() => void update({ type: 'create_group', instance_id: instance.id, name: newGroupName, source_group_id: null })}>新建应用组</Button>
          <Button disabled={disabled || !group || !validName(newGroupName) || instance.groups.length >= 64} onClick={() => group && void update({ type: 'create_group', instance_id: instance.id, name: newGroupName, source_group_id: group.id })}>复制当前组</Button>
        </div>
        <p>新建组的应用全部禁用；复制保留当前组的顺序、启用项和运行策略。每个账号可有 1..64 组。按日完成状态由同一账号各组共用。</p>
        <Field label="运行策略"><Select value={runPolicy} disabled={disabled} onChange={(_, data) => setRunPolicy(data.value as RunPolicy)}>{runPolicyOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</Select></Field>
        <Button disabled={disabled || !group || runPolicy === group.run_policy} onClick={() => group && void update({ type: 'set_run_policy', instance_id: instance.id, group_id: group.id, run_policy: runPolicy })}>保存运行策略</Button>
        <p>新账号的应用默认禁用。启用和排序会保存到当前编辑账号；应用完成迁移与验收后才开放运行。</p>
        {group && <div className="account-create">
          <Field label="添加应用到当前组"><Select value={addAppId} disabled={disabled} onChange={(_, data) => setAddAppId(data.value)}>
            <option value="">选择尚未加入的应用</option>
            {apps.filter(app => !group.apps.some(item => item.app_id === app.id)).map(app => <option key={app.id} value={app.id}>{app.name}</option>)}
          </Select></Field>
          <Button disabled={disabled || !addAppId || group.apps.length >= 256} onClick={() => void update({ type: 'add_group_app', instance_id: instance.id, group_id: group.id, app_id: addAppId })}>添加应用</Button>
        </div>}
        <div className="account-apps">{group?.apps.map((item, index) => {
          const app = apps.find(app => app.id === item.app_id);
          return <div className="account-app" key={item.app_id} data-app-id={item.app_id}>
            <Checkbox label={app?.name ?? item.app_id} checked={item.enabled} disabled={disabled} onChange={(_, data) => void update({ type: 'save_group', instance_id: instance.id, group_id: group.id, apps: group.apps.map((entry, i) => i === index ? { ...entry, enabled: data.checked === true } : entry) })}/>
            <Badge appearance="outline">{!app ? '未注册' : app.implemented ? '可运行' : trialAppOptions.some(option => option.id === app.id) ? '可试运行' : '待迁移'}</Badge>
            <Button size="small" disabled={disabled || index === 0} aria-label={`上移 ${app?.name ?? item.app_id}`} onClick={() => move(index, -1)}>上移</Button>
            <Button size="small" disabled={disabled || index === group.apps.length - 1} aria-label={`下移 ${app?.name ?? item.app_id}`} onClick={() => move(index, 1)}>下移</Button>
          </div>;
        })}</div>
      </Card>}
      {instance && group && <CoffeeSettings key={`${instance.id}/${group.id}/${config.revision}`} settings={group.coffee ?? coffeeDefaultConfig} disabled={disabled} onSave={settings => void update({ type: 'save_coffee', instance_id: instance.id, group_id: group.id, settings })}/>}
      {instance && group && <DismantleSettings key={`dismantle/${instance.id}/${group.id}/${config.revision}`} settings={group.dismantle} disabled={disabled} onSave={settings => void update({ type: 'save_dismantle', instance_id: instance.id, group_id: group.id, settings })}/>}
      {instance && group && <RandomPlaySettings key={`random-play/${instance.id}/${group.id}/${config.revision}`} settings={group.random_play} disabled={disabled} onSave={settings => void update({ type: 'save_random_play', instance_id: instance.id, group_id: group.id, settings })}/>}
      {instance && group && <WorldPatrolSettings key={`world-patrol/${instance.id}/${group.id}/${config.revision}`} settings={group.world_patrol} disabled={disabled} onSave={settings => void update({ type: 'save_world_patrol', instance_id: instance.id, group_id: group.id, settings })}/>}
      {instance && group && <LostVoidSettings key={`lost-void/${instance.id}/${group.id}/${config.revision}`} settings={group.lost_void} disabled={disabled} onSave={settings => void update({ type: 'save_lost_void', instance_id: instance.id, group_id: group.id, settings })}/>}
      {instance && group && <IntelBoardSettings key={`intel-board/${instance.id}/${group.id}/${config.revision}`} settings={group.intel_board} teams={instance.predefined_teams} disabled={disabled} onSave={settings => void update({ type: 'save_intel_board', instance_id: instance.id, group_id: group.id, settings })}/>}
      {instance && group && <ChargePlanSettings key={`charge-plan/${instance.id}/${group.id}/${config.revision}`} settings={group.charge_plan} teams={instance.predefined_teams} instanceId={instance.id} groupId={group.id} revision={config.revision} disabled={disabled} onSave={settings => void update({ type: 'save_charge_plan', instance_id: instance.id, group_id: group.id, settings })}/>}
      {instance && group && <SuibianTempleSettings key={`suibian-temple/${instance.id}/${group.id}/${config.revision}`} settings={group.suibian_temple} disabled={disabled} onSave={settings => void update({ type: 'save_suibian_temple', instance_id: instance.id, group_id: group.id, settings })}/>}
      {instance && group && <DailySettings key={`daily/${instance.id}/${group.id}/${config.revision}`} settings={group.daily} disabled={disabled} onSave={settings => void update({ type: 'save_daily', instance_id: instance.id, group_id: group.id, settings })}/>}
      {instance && group && <GroupRuns instanceId={instance.id} groupId={group.id} revision={config.revision} snapshot={snapshot} connected={connected} blocked={blocked || importBusy || backupBusy || busy} onCommand={onCommand}/>}
    </>}
  </>;
}
