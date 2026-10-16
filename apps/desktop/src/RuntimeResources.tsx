import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, MessageBar, MessageBarBody, ProgressBar, Spinner } from '@fluentui/react-components';
import type { DesktopRuntimeResources, RuntimeResourceRequest } from './generated/protocol';
import { desktop, errorMessage, runtimeResourceState, startRuntimeResourceJob } from './bridge';

export function useRuntimeResources(enabled: boolean) {
  const [snapshot, setSnapshot] = useState<DesktopRuntimeResources>({ catalog: null, job: null });
  const [error, setError] = useState('');
  const [readError, setReadError] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const generation = useRef(0);
  const serial = useRef(0);
  const applied = useRef(0);
  function accept(value: DesktopRuntimeResources, sequence: number) {
    if (sequence < applied.current) return;
    applied.current = sequence; setSnapshot(value); setReadError('');
  }
  useEffect(() => {
    if (!enabled || !desktop) return;
    const current = ++generation.current;
    let timer: ReturnType<typeof setTimeout> | undefined;
    async function poll() {
      const sequence = ++serial.current;
      try {
        const value = await runtimeResourceState();
        if (generation.current === current) accept(value, sequence);
      } catch (reason) {
        if (generation.current === current && sequence >= applied.current) setReadError(`读取运行资源状态失败：${errorMessage(reason)}`);
      } finally {
        if (generation.current === current) timer = setTimeout(() => void poll(), 750);
      }
    }
    void poll();
    return () => { generation.current++; if (timer) clearTimeout(timer); };
  }, [enabled]);
  async function start(request: RuntimeResourceRequest) {
    if (!desktop || submitting || snapshot.job?.state === 'running' || readError) return;
    const current = generation.current;
    const sequence = ++serial.current;
    setSubmitting(true); setError('');
    try {
      const value = await startRuntimeResourceJob(request);
      if (generation.current === current) accept(value, sequence);
    } catch (reason) { if (generation.current === current) setError(errorMessage(reason)); }
    finally { if (generation.current === current) setSubmitting(false); }
  }
  return { snapshot, error, readError, submitting, start };
}

const phases: Record<string, string> = { checking: '正在检查资源与切换条件', stopping_engine: '正在退出旧引擎并确认输入收尾', applying: '正在保存资源版本选择', starting_engine: '正在重新连接普通权限引擎' };

export function RuntimeResources({ manager, blocked }: { manager: ReturnType<typeof useRuntimeResources>; blocked: boolean }) {
  const { snapshot, error, readError, submitting, start } = manager;
  const { catalog, job } = snapshot;
  const active = job?.state === 'running';
  const disabled = !desktop || blocked || submitting || active || !!readError;
  const name = (id: string | null) => id ? catalog?.versions.find(version => version.id === id)?.version ?? `${id.slice(0, 12)}…` : '随程序资源';
  return <>
    <div className="page-heading"><div><div className="eyebrow">RUNTIME RESOURCES</div><h1>运行资源</h1><p>查看已安装版本，切换邮件和地图试运行使用的资源。</p></div><Button disabled={disabled} onClick={() => void start({ type: 'refresh' })}>刷新版本</Button></div>
    {!desktop && <MessageBar intent="info"><MessageBarBody>通过桌面应用可查看本机运行资源版本。</MessageBarBody></MessageBar>}
    {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
    {readError && <MessageBar intent="error"><MessageBarBody>{readError}</MessageBarBody></MessageBar>}
    <Card><div className="card-title"><h2>当前选择</h2><Badge appearance="outline">{catalog ? name(catalog.selection.current) : '尚未读取'}</Badge></div>
      <p>先停止游戏任务并等待输入收尾，再切换版本。目标完整校验通过后会重启引擎，请重新连接游戏；不会自动运行应用。</p>
      <p>本页管理已安装的本地资源。安装新的资源包后，点击“刷新版本”；模型缓存准备不会直接改变运行资源。</p>
      {catalog && <><p>上一选择：{catalog.selection.can_rollback ? name(catalog.selection.previous) : '尚无可回滚版本'}</p><div className="button-row"><Button disabled={disabled || !catalog.selection.can_rollback} onClick={() => void start({ type: 'rollback', revision: catalog.revision })}>回滚并重启引擎</Button></div><p className="resource-path">版本目录：{catalog.directory}</p></>}
    </Card>
    {catalog && <div className="resource-grid">{catalog.versions.map(version => <Card key={version.id} role="group" aria-label={version.version ?? version.id}>
      <div className="card-title"><h2>{version.version ?? '不可用版本'}</h2><Badge appearance="tint" color={!version.compatible ? 'danger' : 'informative'}>{version.id === catalog.selection.current ? '当前选择' : version.compatible ? '清单可用' : '不可用'}</Badge></div>
      <code className="resource-id">{version.id}</code><p>{version.compatible ? '清单与当前引擎兼容，启用前会完整校验模型和流程。' : version.error}</p>
      <div className="button-row"><Button disabled={disabled || !version.compatible || version.id === catalog.selection.current} onClick={() => void start({ type: 'activate', id: version.id, revision: catalog.revision })}>启用并重启引擎</Button></div>
    </Card>)}{catalog.versions.length === 0 && <Card><p>尚未安装独立运行资源包，继续使用随程序资源。</p></Card>}</div>}
    {job && <Card role="status" aria-label="运行资源任务"><div className="card-title"><h2>{job.request.type === 'refresh' ? '版本读取' : '版本切换'}</h2><Badge appearance="tint" color={job.state === 'failed' ? 'danger' : job.state === 'succeeded' ? 'success' : 'informative'}>{active ? '进行中' : job.state === 'failed' ? '失败' : '已完成'}</Badge></div>
      {active && <><Spinner size="tiny" label={phases[job.phase] ?? '正在处理资源'}/><ProgressBar/></>}
      {job.error && <MessageBar intent="error"><MessageBarBody>{job.error}</MessageBarBody></MessageBar>}
      {job.state === 'succeeded' && <p>{job.request.type === 'refresh' ? '本机版本已读取。' : job.reconnected ? '资源选择已保存，引擎已重新连接。请手动连接游戏，需要时再提升引擎权限。' : '资源选择已保存。'}</p>}
      {job.state === 'failed' && job.reconnected && <p>引擎已按磁盘上的版本选择重新连接，任务没有自动重试。</p>}
      <p>离开页面后任务继续；退出桌面会等待当前步骤收尾。</p>
    </Card>}
  </>;
}
