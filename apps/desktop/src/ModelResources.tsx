import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Checkbox, MessageBar, MessageBarBody, ProgressBar, Spinner } from '@fluentui/react-components';
import type { CacheState, ModelResources as ResourceSnapshot, ResourceJobKind } from './generated/protocol';
import { errorMessage, request } from './bridge';

const cacheName = (value: CacheState | null) => value === 'valid' ? '已校验' : value === 'invalid' ? '需修复' : value === 'missing' ? '未缓存' : '尚未检查';
const jobNames = { running: '进行中', cancelling: '取消中', succeeded: '已完成', failed: '失败', cancelled: '已取消' };
const phaseNames: Record<string, string> = { checking: '检查文件', downloading: '下载模型包', extracting: '解压与校验', ready: '模型文件已就绪' };
const size = (bytes: number) => `${(bytes / 1024 / 1024).toFixed(1)} MiB`;

export function ModelResources({ connected, blocked }: { connected: boolean; blocked: boolean }) {
  const [snapshot, setSnapshot] = useState<ResourceSnapshot | null>(null);
  const [selected, setSelected] = useState<string[] | null>(null);
  const [offline, setOffline] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [loadError, setLoadError] = useState('');
  const generation = useRef(0);
  const serial = useRef(0);
  const applied = useRef(0);
  const job = snapshot?.job;
  const active = !!job && ['running', 'cancelling'].includes(job.state);
  const disabled = !connected || blocked || busy || active || !snapshot || !!loadError;

  function accept(value: ResourceSnapshot, sequence: number) {
    if (sequence < applied.current) return;
    applied.current = sequence;
    setSnapshot(value); setSelected(current => current ?? value.models.map(model => model.id));
  }

  useEffect(() => {
    const current = ++generation.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    setSnapshot(null); setSelected(null); setError(''); setLoadError(''); setBusy(false);
    async function poll() {
      const sequence = ++serial.current;
      try {
        const result = await request({ type: 'model_resources' });
        if (generation.current !== current) return;
        if (result.type !== 'model_resources') throw new Error('引擎未返回模型资源状态');
        accept(result.data, sequence);
        if (sequence >= applied.current) setLoadError('');
      } catch (reason) {
        if (generation.current === current && sequence >= applied.current) setLoadError(`读取资源状态失败：${errorMessage(reason)}`);
      } finally {
        if (generation.current === current) timer = setTimeout(() => void poll(), 750);
      }
    }
    if (connected) void poll();
    return () => { generation.current++; if (timer) clearTimeout(timer); };
  }, [connected]);

  async function act(kind: ResourceJobKind | 'cancel') {
    if (!snapshot || !connected || blocked || busy) return;
    const current = generation.current;
    const sequence = ++serial.current;
    setBusy(true); setError('');
    try {
      const result = await request(kind === 'cancel'
        ? { type: 'cancel_model_resources', params: { job_id: snapshot.job!.id } }
        : { type: 'start_model_resources', params: { kind, models: kind === 'check' ? [] : selected ?? [], offline: kind === 'prepare' && offline } });
      if (generation.current !== current) return;
      if (result.type !== 'model_resources') throw new Error('引擎未返回资源任务');
      accept(result.data, sequence);
    } catch (reason) { if (generation.current === current) setError(errorMessage(reason)); }
    finally { if (generation.current === current) setBusy(false); }
  }

  return <>
    <div className="page-heading"><div><div className="eyebrow">MODEL RESOURCES</div><h1>模型资源</h1><p>检查或准备固定模型缓存，供后续资源构建使用。</p></div><Button disabled={disabled} onClick={() => void act('check')}>检查缓存</Button></div>
    {!connected && <MessageBar intent="info"><MessageBarBody>连接桌面引擎后可查看和准备本地模型缓存。</MessageBarBody></MessageBar>}
    {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
    {loadError && <MessageBar intent="error"><MessageBarBody>{loadError}</MessageBarBody></MessageBar>}
    {snapshot && <>
      <Card className="resource-options"><div className="card-title"><h2>准备模型</h2><Badge appearance="outline">{snapshot.models.length} 个固定模型</Badge></div>
        <p>已校验的模型会直接复用。下载完成后，游戏任务继续使用当前资源包；运行资源切换将在后续版本接通。</p>
        <div className="button-row resource-actions"><Checkbox label="仅使用离线缓存" checked={offline} disabled={disabled} onChange={(_, data) => setOffline(data.checked === true)}/><Button appearance="primary" disabled={disabled || !selected?.length} onClick={() => void act('prepare')}>准备所选模型</Button></div>
        <p className="resource-path">缓存目录：{snapshot.cache_directory}</p>
      </Card>
      <div className="resource-grid">{snapshot.models.map(model => <Card key={model.id} role="group" aria-label={model.name}>
        <div className="card-title"><Checkbox label={model.name} checked={selected?.includes(model.id) ?? false} disabled={disabled} onChange={(_, data) => setSelected(current => data.checked ? [...current ?? [], model.id] : current?.filter(id => id !== model.id) ?? [])}/><Badge appearance="tint" color={model.files === 'valid' ? 'success' : 'informative'}>{cacheName(model.files)}</Badge></div>
        <code className="resource-id">{model.id}</code><div className="resource-details"><span>模型包 {size(model.archive_bytes)}</span><span>压缩包：{cacheName(model.archive)}</span></div>
      </Card>)}</div>
      {job && <Card role="status" aria-label="模型资源任务">
        <div className="card-title"><h2>{job.kind === 'check' ? '缓存检查' : '模型准备'}</h2><Badge appearance="tint" color={job.state === 'failed' ? 'danger' : job.state === 'succeeded' ? 'success' : 'informative'}>{jobNames[job.state]}</Badge></div>
        {active && <><div className="button-row"><Spinner size="tiny"/><span>{job.state === 'cancelling' ? '正在等待下载与文件收尾' : job.progress ? phaseNames[job.progress.phase] ?? job.progress.phase : '正在检查模型缓存'}</span></div><ProgressBar value={job.state === 'running' && job.progress?.phase === 'downloading' && job.progress.total_bytes > 0 ? Math.min(1, job.progress.completed_bytes / job.progress.total_bytes) : undefined}/></>}
        {job.progress && <p className="resource-id">{job.progress.id}{job.progress.phase === 'downloading' ? ` · ${size(job.progress.completed_bytes)} / ${size(job.progress.total_bytes)}` : ''}</p>}
        {job.state === 'succeeded' && <p>{job.kind === 'check' ? '缓存检查完成。各模型状态见上方列表。' : `本次准备完成，复用 ${job.prepared.filter(model => model.reused).length} 个，下载 ${job.prepared.filter(model => model.downloaded).length} 个。`}</p>}
        {job.state === 'cancelled' && <p>已取消本次任务。已完成校验的前项仍保留，可重新检查缓存后继续准备。</p>}
        {job.error && job.state !== 'cancelled' && <MessageBar intent="error"><MessageBarBody>{job.error}</MessageBarBody></MessageBar>}
        <p>离开此页面后任务继续执行；退出或切换引擎会取消当前资源任务。</p>
        {active && <div className="button-row"><Button disabled={!connected || blocked || busy || job.state === 'cancelling'} onClick={() => void act('cancel')}>取消资源任务</Button></div>}
      </Card>}
    </>}
  </>;
}
