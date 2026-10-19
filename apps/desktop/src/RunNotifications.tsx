import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Field, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import type { RunNotificationMode, RunNotificationSettings, RunNotifications as Notices } from './generated/protocol';
import { runNotificationOptions } from './generated/protocol';
import { desktop, errorMessage, runNotificationState, updateRunNotifications } from './bridge';

export const runNotificationName = (settings?: RunNotificationSettings) => runNotificationOptions.find(item => item.value === (settings?.mode ?? 'errors'))?.label ?? '仅异常';

export function RunNotificationEditor({ settings, disabled, onSave }: { settings?: RunNotificationSettings; disabled: boolean; onSave: (value: RunNotificationSettings) => void }) {
  const [mode, setMode] = useState<RunNotificationMode>(settings?.mode ?? 'errors');
  return <Card aria-label="账号运行结果提醒设置">
    <div className="card-title"><h2>运行结果提醒</h2><Badge appearance="outline">按账号保存</Badge></div>
    <Field label="应用组结果提醒"><Select value={mode} disabled={disabled} onChange={(_, data) => setMode(data.value as RunNotificationMode)}>{runNotificationOptions.map(item => <option key={item.value} value={item.value}>{item.label}</option>)}</Select></Field>
    <p>仅异常包含失败、记录未保存、输入未释放和连接中断；全部结果还包括正常完成与主动停止。设置用于下次由桌面启动的应用组。</p>
    <p>提醒保留在本次桌面会话中，可跨页面查看，隐藏到托盘后显示未读数。完整历史仍在运行记录中查看。</p>
    <Button disabled={disabled || mode === (settings?.mode ?? 'errors')} onClick={() => onSave({ mode })}>保存运行提醒设置</Button>
  </Card>;
}

export function RunNotificationBanner({ data, onOpen }: { data: Notices | null; onOpen: () => void }) {
  const unread = data?.items.filter(item => !item.read) ?? [];
  const latest = unread[0];
  if (!latest) return null;
  return <MessageBar intent={latest.level} aria-label="未读运行提醒"><MessageBarBody>
    <strong>{latest.title}</strong> · {latest.instance_name} / {latest.group_name} · {unread.length} 条未读
    <Button size="small" onClick={onOpen}>查看运行提醒</Button>
  </MessageBarBody></MessageBar>;
}

export function RunNotifications({ data, onChange, onHistory }: { data: Notices | null; onChange: (value: Notices) => void; onHistory: () => void }) {
  const [filter, setFilter] = useState('unread');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const generation = useRef(0);
  const working = useRef(false);
  async function perform(ids?: string[], remove = false) {
    if (!desktop || working.current) return;
    working.current = true; const requested = generation.current; setBusy(true); setError('');
    try { const value = ids ? await updateRunNotifications(ids, remove) : await runNotificationState(); if (generation.current === requested) onChange(value); }
    catch (reason) { if (generation.current === requested) setError(errorMessage(reason)); }
    finally { if (generation.current === requested) { setBusy(false); working.current = false; } }
  }
  useEffect(() => { generation.current++; working.current = false; setBusy(false); void perform(); return () => { generation.current++; working.current = false; }; }, []);
  const items = data?.items.filter(item => filter === 'all' || !item.read) ?? [];
  const unread = data?.items.filter(item => !item.read) ?? [];
  return <>
    <div className="page-heading"><div><div className="eyebrow">RUN NOTIFICATIONS</div><h1>运行提醒</h1><p>查看本次桌面会话中的应用组结果提醒。完整记录在运行历史中保存。</p></div><Button disabled={!desktop || busy} onClick={() => void perform()}>刷新运行提醒</Button></div>
    {!desktop && <MessageBar intent="info"><MessageBarBody>使用桌面应用后可查看本机运行提醒。</MessageBarBody></MessageBar>}
    {error && <MessageBar intent="error"><MessageBarBody>读取或更新运行提醒失败：{error}。现有提醒已保留，可刷新后重试。</MessageBarBody></MessageBar>}
    <Card aria-label="运行提醒列表">
      <div className="card-title"><h2>本次会话提醒</h2><Badge appearance="outline">{unread.length} 条未读</Badge></div>
      <Field label="提醒筛选"><Select value={filter} onChange={(_, value) => setFilter(value.value)}><option value="unread">未读</option><option value="all">全部</option></Select></Field>
      <div className="button-row"><Button disabled={!desktop || busy || !unread.length} onClick={() => void perform(unread.map(item => item.id))}>全部标为已读</Button>
        <Button disabled={!desktop || busy || !data?.items.some(item => item.read)} onClick={() => void perform(data!.items.filter(item => item.read).map(item => item.id), true)}>清除已读提醒</Button>
        <Button onClick={onHistory}>查看完整运行记录</Button>{busy && <Spinner size="tiny" label="处理运行提醒"/>}</div>
      <p>最多保留最近 32 条；已超量丢弃 {data?.dropped ?? 0} 条。桌面退出后清空提醒，重新连接引擎不会补发历史任务。</p>
      {!items.length && <p>{!data && error ? '提醒尚未读取，可刷新后重试。' : busy && !data ? '正在读取运行提醒。' : filter === 'unread' ? '暂无未读运行提醒。' : '本次会话暂无运行提醒。'}</p>}
      <ol className="group-plan notification-list">{items.map(item => <li key={item.id}>
        <div className="card-title"><strong>{item.title}</strong><Badge appearance="outline">{item.read ? '已读' : '未读'}</Badge></div>
        <p>{item.instance_name}（编号 {item.instance_id}） / {item.group_name} · {new Date(item.at).toLocaleString('zh-CN', { hour12: false })}</p>
        <MessageBar intent={item.level}><MessageBarBody>{item.message}</MessageBarBody></MessageBar>
        <p className="path-value">运行 ID：{item.run_id}</p>
        {!item.read && <Button size="small" disabled={!desktop || busy} onClick={() => void perform([item.id])}>标为已读</Button>}
      </li>)}</ol>
    </Card>
  </>;
}
