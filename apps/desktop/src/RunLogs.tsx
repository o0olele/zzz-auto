import { useEffect, useMemo, useRef, useState } from 'react';
import { Badge, Button, Card, Field, Input, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import type { DesktopLogs, LogExport } from './generated/protocol';
import { desktopLogs, errorMessage, exportLogSession } from './bridge';

const PAGE_SIZE = 50;
const when = (value: string) => new Date(value).toLocaleString('zh-CN', { hour12: false });

export function RunLogs({ enabled, connected }: { enabled: boolean; connected: boolean }) {
  const [data, setData] = useState<DesktopLogs | null>(null);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [level, setLevel] = useState('all');
  const [runId, setRunId] = useState('all');
  const [query, setQuery] = useState('');
  const [page, setPage] = useState(0);
  const [frozen, setFrozen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [readError, setReadError] = useState('');
  const [exportError, setExportError] = useState('');
  const [saved, setSaved] = useState<LogExport | null>(null);
  const generation = useRef(0);
  const reading = useRef<number | null>(null);
  const writing = useRef(false);

  async function refresh(requested: number) {
    if (!enabled || reading.current === requested) return;
    reading.current = requested; setLoading(true);
    try {
      const value = await desktopLogs(selectedId);
      if (generation.current === requested) { setData(value); setReadError(''); }
    } catch (reason) {
      if (generation.current === requested) setReadError(errorMessage(reason));
    } finally {
      if (reading.current === requested) reading.current = null;
      if (generation.current === requested) setLoading(false);
    }
  }

  useEffect(() => {
    const requested = ++generation.current;
    setLoading(false); setExporting(false); writing.current = false;
    if (enabled && (!frozen || !data)) void refresh(requested);
    const timer = enabled && !frozen ? setInterval(() => void refresh(requested), 1000) : undefined;
    return () => { clearInterval(timer); generation.current++; };
  }, [enabled, selectedId, frozen]);

  const session = data?.selected;
  const runIds = useMemo(() => [...new Set([...(session?.events.map(event => event.run_id).filter((id): id is string => id !== null) ?? []), ...(session?.run ? [session.run.id] : []), ...(!['all', 'engine'].includes(runId) ? [runId] : [])])], [session, runId]);
  const filtered = useMemo(() => {
    const text = query.trim().toLocaleLowerCase();
    return [...(session?.events ?? [])].reverse().filter(event =>
      (level === 'all' || event.level.toLowerCase() === level)
      && (runId === 'all' || (runId === 'engine' ? event.run_id === null : event.run_id === runId))
      && (!text || `${event.message} ${event.run_id ?? ''} ${event.sequence}`.toLocaleLowerCase().includes(text)));
  }, [session, level, runId, query]);
  const pages = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const currentPage = Math.min(page, pages - 1);

  async function save() {
    if (!enabled || !session || writing.current) return;
    const requested = generation.current;
    writing.current = true; setExporting(true); setExportError(''); setSaved(null);
    try {
      const result = await exportLogSession(session.id);
      if (generation.current === requested) setSaved(result);
    } catch (reason) {
      if (generation.current === requested) setExportError(errorMessage(reason));
    } finally {
      if (generation.current === requested) { writing.current = false; setExporting(false); }
    }
  }

  return <>
    <div className="page-heading"><div><div className="eyebrow">RUN LOGS</div><h1>运行日志</h1><p>查看桌面收到的引擎日志，筛选问题并导出到本机。</p></div><div className="button-row log-actions">
      <Button disabled={!enabled || loading} onClick={() => void refresh(generation.current)}>刷新日志</Button>
      <Button disabled={!enabled || exporting} onClick={() => setFrozen(value => !value)}>{frozen ? '继续刷新日志' : '暂停日志刷新'}</Button>
    </div></div>
    {!enabled && <MessageBar intent="info"><MessageBarBody>浏览器预览不提供本机日志。请通过桌面应用查看。</MessageBarBody></MessageBar>}
    {enabled && !connected && <MessageBar intent="info"><MessageBarBody>引擎未连接，仍可查看和导出桌面已经收到的日志。</MessageBarBody></MessageBar>}
    {frozen && <MessageBar intent="info"><MessageBarBody>日志画面已暂停刷新，游戏任务继续运行。导出取所选会话最新收到的全部日志。</MessageBarBody></MessageBar>}
    {readError && <MessageBar intent="error"><MessageBarBody>{readError}。已保留上次读取结果，可刷新或选择最近会话。</MessageBarBody></MessageBar>}
    {exportError && <MessageBar intent="error"><MessageBarBody>日志导出失败：{exportError}</MessageBarBody></MessageBar>}
    <Card className="run-logs-card">
      <div className="card-title"><h2>会话与筛选</h2>{loading && <Spinner size="tiny" label="读取日志"/>}</div>
      <div className="input-fields">
        <Field label="引擎会话"><Select disabled={!enabled || exporting} value={selectedId ?? ''} onChange={(_, value) => {
          generation.current++; setSelectedId(value.value || null); setData(null); setReadError(''); setSaved(null); setExportError(''); setRunId('all'); setPage(0);
        }}><option value="">最近引擎会话</option>{data?.sessions.map(item => <option key={item.id} value={item.id}>{when(item.captured_at)} · {item.id.slice(0, 8)} · {item.event_count} 条</option>)}</Select></Field>
        <Field label="日志级别"><Select disabled={!enabled} value={level} onChange={(_, value) => { setLevel(value.value); setPage(0); }}><option value="all">全部级别</option><option value="info">INFO</option><option value="warn">WARN</option><option value="error">ERROR</option><option value="debug">DEBUG</option></Select></Field>
        <Field label="所属运行"><Select disabled={!enabled} value={runId} onChange={(_, value) => { setRunId(value.value); setPage(0); }}><option value="all">全部运行与引擎消息</option><option value="engine">仅引擎消息</option>{runIds.map(id => <option key={id} value={id}>{id === session?.run?.id ? '最近运行 · ' : ''}{id}</option>)}</Select></Field>
        <Field label="搜索日志"><Input disabled={!enabled} value={query} onChange={(_, value) => { setQuery(value.value); setPage(0); }} placeholder="文字、运行 ID 或序号"/></Field>
      </div>
      <p>最多保留最近 4 个引擎会话，每个会话 256 条。已淘汰 {data?.discarded_sessions ?? 0} 个会话；单条文字最多 8 KiB，超出时会截断。重启桌面后内存日志清空。</p>
      {session && <><p className="resource-path">会话 ID：{session.id} · 采集时间：{when(session.captured_at)} · 引擎 {session.engine_version}</p>
        <div className="log-summary"><Badge appearance="outline">保留 {session.events.length} 条</Badge><span>引擎累计丢弃 {session.dropped_events} 条 · 宿主额外略去 {session.omitted_events} 条 · 截断 {session.truncated_events} 条</span></div>
        <div className="button-row log-actions"><Button appearance="primary" disabled={!enabled || exporting} onClick={() => void save()}>导出会话日志</Button>{exporting && <Spinner size="tiny" label="导出日志"/>}</div>
        <p>导出该会话最新收到的全部保留日志，不受筛选影响。文件保存到桌面数据目录的 logs/exports，只在本机保存。</p>
      </>}
      {saved && <><MessageBar intent="success"><MessageBarBody>已导出 {saved.event_count} 条日志，会话 {saved.session_id}。</MessageBarBody></MessageBar><Field label="导出日志路径"><Input readOnly value={saved.path}/></Field></>}
    </Card>
    <Card className="run-logs-card">
      <div className="card-title"><h2>日志事件</h2><Badge appearance="outline">筛选后 {filtered.length} 条</Badge></div>
      <ol className="run-log-list" aria-label="日志事件列表">{filtered.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE).map(event => <li className="run-log-event" key={event.sequence} data-sequence={event.sequence}>
        <div><time dateTime={event.at}>{when(event.at)}</time><small>#{event.sequence} · {event.level.toUpperCase()}</small></div>
        <div><p className={`run-log-message ${event.level}`}>{event.message}</p><code>{event.run_id ? `运行 ${event.run_id}` : '引擎消息'}</code></div>
      </li>)}</ol>
      {!filtered.length && <div className="empty-row">{session ? '没有符合筛选条件的日志' : '还没有收到引擎日志'}</div>}
      <div className="log-pagination"><Button disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>上一页</Button><span>第 {currentPage + 1} / {pages} 页 · 每页 {PAGE_SIZE} 条 · 新日志在前</span><Button disabled={currentPage + 1 >= pages} onClick={() => setPage(currentPage + 1)}>下一页</Button></div>
    </Card>
  </>;
}
