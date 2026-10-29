import { IntelBoardStatus } from './IntelBoardSettings';
import { SuibianTempleStatus } from './SuibianTempleSettings';
import { RedemptionStatus } from './RedemptionSettings';
import { RandomPlayStatus } from './RandomPlaySettings';
import { TeamCheckStatus } from './TeamSettings';
import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Field, Input, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import { trialAppOptions, type AccountConfig, type HistoryCursor, type HistoryPage, type HistoryQuery, type HistoryItem } from './generated/protocol';
import { errorMessage, request } from './bridge';

const emptyQuery = (): HistoryQuery => ({ group_id: null, app_id: null, game_day: null, outcome: null, before: null });
const outcomes = { active: '运行中', succeeded: '完成', failed: '失败', timed_out: '超时', stopped: '已停止', interrupted: '异常中断' };
const stepNames = { ended: '已结束（未完成）', pending: '等待执行', running: '执行中', skipped: '已跳过', succeeded: '执行完成', failed: '执行失败', stopped: '执行停止' };
const appName = (id: string) => trialAppOptions.find(app => app.id === id)?.label ?? id;

function outcome(item: HistoryItem) {
  if (item.interrupted) return '异常中断';
  if (item.record.mail?.state === 'timed_out' || item.record.coffee?.state === 'timed_out' || item.record.daily?.some(app => app.state === 'timed_out')) return '超时';
  const names: Record<string, string> = { ...outcomes, running: '运行中', pausing: '暂停中', paused: '已暂停', stopping: '停止中' };
  return names[item.record.state] ?? item.record.state;
}

export function RunHistory({ connected }: { connected: boolean }) {
  const [config, setConfig] = useState<AccountConfig | null>(null);
  const [filters, setFilters] = useState<HistoryQuery>(emptyQuery);
  const [navigation, setNavigation] = useState<{ instance: number; query: HistoryQuery; cursors: (HistoryCursor | null)[] } | null>(null);
  const [result, setResult] = useState<HistoryPage | null>(null);
  const [dirty, setDirty] = useState(false);
  const [loading, setLoading] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [error, setError] = useState('');
  const [exported, setExported] = useState('');
  const [retry, setRetry] = useState(0);
  const generation = useRef(0);

  useEffect(() => {
    let alive = true;
    setConfig(null); setNavigation(null); setResult(null); setError(''); setExported(''); setLoading(false); setExporting(false);
    if (connected) {
      setLoading(true);
      request({ type: 'account_config' }).then(value => {
        if (!alive) return;
        if (value.type !== 'account_config') throw new Error('引擎未返回账号配置');
        setConfig(value.data); setFilters(emptyQuery()); setDirty(false);
        setNavigation({ instance: value.data.active_instance, query: emptyQuery(), cursors: [null] });
      }).catch(reason => { if (alive) { setError(errorMessage(reason)); setLoading(false); } });
    }
    return () => { alive = false; generation.current++; };
  }, [connected, retry]);

  useEffect(() => {
    if (!connected || !navigation) return;
    const ticket = ++generation.current;
    setResult(null); setLoading(true); setExporting(false); setExported(''); setError('');
    request({ type: 'query_run_history', params: { instance_id: navigation.instance, query: navigation.query } }).then(value => {
      if (generation.current !== ticket) return;
      if (value.type !== 'history_page' || value.data.instance_id !== navigation.instance) throw new Error('引擎未返回所选账号的历史页面');
      setResult(value.data);
    }).catch(reason => { if (generation.current === ticket) setError(errorMessage(reason)); })
      .finally(() => { if (generation.current === ticket) setLoading(false); });
    return () => { generation.current++; };
  }, [connected, navigation]);

  function edit(next: HistoryQuery) {
    generation.current++; setFilters(next); setDirty(true); setResult(null); setError(''); setExported(''); setLoading(false); setExporting(false);
  }

  function firstPage() {
    if (!navigation) return;
    setDirty(false);
    setNavigation({ instance: navigation.instance, query: { ...filters, before: null }, cursors: [null] });
  }

  async function exportPage() {
    if (!result || !navigation) return;
    const ticket = ++generation.current;
    setExporting(true); setError(''); setExported('');
    try {
      const value = await request({ type: 'export_run_history', params: { instance_id: navigation.instance, query: result.query, content_id: result.content_id } });
      if (generation.current !== ticket) return;
      if (value.type !== 'history_export' || value.data.instance_id !== navigation.instance || value.data.content_id !== result.content_id) throw new Error('引擎未返回本页的导出结果');
      setExported(`已导出 ${value.data.record_count} 条记录：${value.data.path}`);
    } catch (reason) { if (generation.current === ticket) setError(errorMessage(reason)); }
    finally { if (generation.current === ticket) setExporting(false); }
  }

  return <>
    <div className="page-heading"><div><div className="eyebrow">SAVED RUNS</div><h1>运行记录</h1><p>核对账号已保存的结果、分项和输入释放状态。</p></div></div>
    {!connected && <MessageBar intent="info"><MessageBarBody>请连接桌面引擎后读取本机运行记录。</MessageBarBody></MessageBar>}
    <Card className="history-browser">
      <div className="card-title"><h2>筛选历史</h2><Button disabled={!connected || loading || exporting} onClick={() => { if (config) firstPage(); else setRetry(retry + 1); }}>刷新运行记录</Button></div>
      <div className="input-fields">
        <Field label="记录账号"><Select aria-label="记录账号" disabled={!connected || !config} value={navigation?.instance ?? ''} onChange={(_, data) => {
          generation.current++; setResult(null); setExported(''); setError('');
          setFilters(emptyQuery()); setDirty(false); setNavigation({ instance: Number(data.value), query: emptyQuery(), cursors: [null] });
        }}>{config?.instances.map(instance => <option key={instance.id} value={instance.id}>{instance.name} · {instance.id}</option>)}</Select></Field>
        <Field label="记录结果"><Select aria-label="记录结果" value={filters.outcome ?? ''} disabled={!connected || !config} onChange={(_, data) => edit({ ...filters, outcome: data.value as HistoryQuery['outcome'] || null })}>
          <option value="">全部结果</option>{Object.entries(outcomes).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
        </Select></Field>
        <Field label="记录应用"><Select aria-label="记录应用" value={filters.app_id ?? ''} disabled={!connected || !config} onChange={(_, data) => edit({ ...filters, app_id: data.value || null })}>
          <option value="">全部应用</option>{trialAppOptions.map(app => <option key={app.id} value={app.id}>{app.label}</option>)}
        </Select></Field>
        <Field label="记录游戏日"><Input aria-label="记录游戏日" type="date" value={filters.game_day ?? ''} disabled={!connected || !config} onChange={(_, data) => edit({ ...filters, game_day: data.value || null })}/></Field>
        <Field label="记录组标识" hint="留空查全部组；可填写已删除组的原标识。"><Input aria-label="记录组标识" maxLength={256} value={filters.group_id ?? ''} disabled={!connected || !config} onChange={(_, data) => edit({ ...filters, group_id: data.value || null })}/></Field>
      </div>
      <div className="button-row"><Button appearance="primary" disabled={!connected || !config || loading || exporting} onClick={firstPage}>查询记录</Button>
        <Button disabled={!connected || !result?.items.length || dirty || loading || exporting} onClick={() => void exportPage()}>导出本页记录</Button></div>
      <p>导出保存到本机数据目录的 runs/exports/，只包含本页已保存记录。记录变化后需刷新再导出。</p>
      {dirty && <p>筛选已更改，点击“查询记录”应用。</p>}
      {loading && <Spinner size="tiny" label="读取运行记录"/>}{exporting && <Spinner size="tiny" label="导出运行记录"/>}
      {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
      {exported && <MessageBar intent="success"><MessageBarBody><span className="resource-path">{exported}</span></MessageBarBody></MessageBar>}
    </Card>
    {result && <Card className="history-browser" aria-label="历史查询结果">
      <div className="card-title"><h2>已保存结果</h2><Badge appearance="outline">筛选匹配 {result.matching} 条</Badge></div>
      {!!result.unreadable && <MessageBar intent="warning"><MessageBarBody>{result.unreadable} 份记录无法读取，原文件已保留。</MessageBarBody></MessageBar>}
      {!result.items.length && <p>没有符合筛选条件的记录。</p>}
      <div className="history-results">{result.items.map(item => <article className="history-item" key={item.record.id}>
        <div className="card-title"><strong>{item.record.context?.instance_name} · {item.record.context?.group_name ?? item.record.context?.group_id}</strong><Badge appearance="outline">{outcome(item)}</Badge></div>
        <p>开始 {new Date(item.record.started_at).toLocaleString()} · {item.record.finished_at ? `结束 ${new Date(item.record.finished_at).toLocaleString()}` : '尚无已保存的结束时间'}</p>
        <p>组标识 {item.record.context?.group_id} · 修订 {item.record.context?.config_revision} · 游戏日 {item.record.context?.game_day?.date ?? '未记录'}</p>
        {item.record.context?.standalone && <p>独立应用运行</p>}
        {item.record.context?.rerun_completed && <p>本次重新运行已完成应用</p>}
        <p>{item.interrupted ? '上次引擎退出前未保存终态，实际结束时间未知，不会自动继续。' : item.record.message}</p>
        {item.record.daily?.filter(app => app.redemption).map(app => <RedemptionStatus key={app.app_id} progress={app.redemption!}/>)}
        {item.record.daily?.filter(app => app.random_play).map(app => <RandomPlayStatus key={app.app_id} progress={app.random_play!}/>)}
        {item.record.daily?.filter(app => app.team_check).map(app => <TeamCheckStatus key={app.app_id} progress={app.team_check!}/>)}
        {item.record.daily?.filter(app => app.intel_board).map(app => <IntelBoardStatus key={app.app_id} progress={app}/>)}
        {item.record.daily?.filter(app => app.temple).map(app => <SuibianTempleStatus key={app.app_id} progress={app}/>)}
        {item.record.group && <ol aria-label="历史分项结果">{item.record.group.steps.map(step => <li key={step.app_id}>{appName(step.app_id)} · {stepNames[step.state]} · {step.inputs_released ? '分项输入已释放' : '分项输入待释放'}</li>)}</ol>}
        {!item.record.group && item.record.mail && <p>邮件 · {item.record.mail.inputs_released ? '输入已释放' : '等待输入收尾'}</p>}
        {item.record.combat && <p>{item.record.combat.config.dodge_only ? '闪避助手' : item.record.combat.config.script} · 动作 {item.record.combat.actions} 次 · 抢占 {item.record.combat.preemptions} 次 · {item.record.combat.inputs_released ? '输入已释放' : '等待输入收尾'}</p>}
        <code>{item.record.id}</code>
      </article>)}</div>
      <div className="log-pagination"><Button disabled={!navigation || navigation.cursors.length <= 1 || loading || exporting} onClick={() => {
        if (!navigation) return;
        const cursors = navigation.cursors.slice(0, -1);
        setNavigation({ ...navigation, query: { ...navigation.query, before: cursors[cursors.length - 1] }, cursors });
      }}>上一页记录</Button><span>第 {navigation?.cursors.length ?? 1} 页 · 本页 {result.items.length} 条 · 每页最多 20 条</span><Button disabled={!result.next || loading || exporting} onClick={() => {
        if (navigation && result.next) setNavigation({ ...navigation, query: { ...navigation.query, before: result.next }, cursors: [...navigation.cursors, result.next] });
      }}>下一页记录</Button></div>
    </Card>}
  </>;
}
