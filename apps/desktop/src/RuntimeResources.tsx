import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Field, Input, MessageBar, MessageBarBody, ProgressBar, Spinner } from '@fluentui/react-components';
import type { DesktopRuntimeResources, RuntimeResourceRequest, RuntimeDownloadSource } from './generated/protocol';
import { desktop, errorMessage, pickRuntimeResourceArchive, runtimeResourceState, startRuntimeResourceJob, startRuntimeResourceDownload, cancelRuntimeResourceDownload } from './bridge';

const isActive = (state: string | undefined) => state === 'running' || state === 'cancelling';

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
  async function submit(action: () => Promise<DesktopRuntimeResources>, cancelling = false) {
    if (!desktop || submitting || (!cancelling && isActive(snapshot.job?.state)) || readError) return;
    const current = generation.current;
    const sequence = ++serial.current;
    setSubmitting(true); setError('');
    try {
      const value = await action();
      if (generation.current === current) accept(value, sequence);
    } catch (reason) { if (generation.current === current) setError(errorMessage(reason)); }
    finally { if (generation.current === current) setSubmitting(false); }
  }
  const start = (request: RuntimeResourceRequest) => submit(() => startRuntimeResourceJob(request));
  const download = (source: RuntimeDownloadSource) => submit(() => startRuntimeResourceDownload(source));
  const cancel = (id: string) => submit(() => cancelRuntimeResourceDownload(id), true);
  return { snapshot, error, readError, submitting, start, download, cancel };
}

const phases: Record<string, string> = { checking: '正在完整检查资源与操作条件', connecting: '正在连接下载服务器', downloading: '正在下载资源包', validating: '正在完整校验下载内容', publishing: '正在保存已校验资源包', ready: '资源包已就绪', stopping_engine: '正在退出旧引擎并确认输入收尾', installing: '正在安装并核对资源文件', applying: '正在保存资源版本选择', starting_engine: '正在重新连接普通权限引擎' };

function validHttps(value: string) {
  try {
    const parsed = new URL(value);
    return value.length <= 8192 && !/[\u0000-\u0020\u007f]/.test(value) && parsed.protocol === 'https:' && !!parsed.hostname && !parsed.username && !parsed.password && !value.includes('#');
  } catch { return false; }
}

export function RuntimeResources({ manager, blocked, maintenance = false }: { manager: ReturnType<typeof useRuntimeResources>; blocked: boolean; maintenance?: boolean }) {
  const { snapshot, error, readError, submitting, start, download, cancel } = manager;
  const { catalog, job } = snapshot;
  const [archive, setArchive] = useState('');
  const [sha256, setSha256] = useState('');
  const [picking, setPicking] = useState(false);
  const [pickerError, setPickerError] = useState('');
  const [url, setUrl] = useState('');
  const [bytes, setBytes] = useState('');
  const [downloadHash, setDownloadHash] = useState('');
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const active = isActive(job?.state);
  const disabled = !desktop || blocked || submitting || active || picking || !!readError;
  const downloadDisabled = !desktop || maintenance || submitting || active || picking || !!readError;
  const validBytes = /^\d+$/.test(bytes) && Number.isSafeInteger(Number(bytes)) && Number(bytes) >= 1 && Number(bytes) <= 536870912;
  const validDownload = validHttps(url) && validBytes && /^[0-9a-f]{64}$/.test(downloadHash);
  const validHash = /^[0-9a-f]{64}$/.test(sha256);
  async function pickArchive() {
    setPicking(true); setPickerError('');
    try {
      const path = await pickRuntimeResourceArchive();
      if (mounted.current && path) { setArchive(path); setSha256(''); }
    } catch (reason) { if (mounted.current) setPickerError(errorMessage(reason)); }
    finally { if (mounted.current) setPicking(false); }
  }
  const name = (id: string | null) => id ? catalog?.versions.find(version => version.id === id)?.version ?? `${id.slice(0, 12)}…` : '随程序资源';
  return <>
    <div className="page-heading"><div><div className="eyebrow">RUNTIME RESOURCES</div><h1>运行资源</h1><p>下载或安装资源包，切换邮件和地图试运行使用的资源。</p></div><Button disabled={disabled} onClick={() => void start({ type: 'refresh' })}>刷新版本</Button></div>
    {!desktop && <MessageBar intent="info"><MessageBarBody>通过桌面应用可查看本机运行资源版本。</MessageBarBody></MessageBar>}
    {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
    {readError && <MessageBar intent="error"><MessageBarBody>{readError}</MessageBarBody></MessageBar>}
    <Card aria-label="下载运行资源包"><h2>下载资源包</h2>
      <p>填写发布方提供的 HTTPS 地址、ZIP 字节数与 SHA-256。下载并完整校验后可安装，下载期间引擎继续运行。</p>
      <Field label="资源包 HTTPS 地址" validationState={url && !validHttps(url) ? 'error' : 'none'} validationMessage={url && !validHttps(url) ? '请填写不含用户名、密码或片段的 HTTPS 地址' : undefined}><Input value={url} maxLength={8192} disabled={downloadDisabled} onChange={(_, data) => setUrl(data.value.trim())} placeholder="发布方提供的 HTTPS 下载地址" autoComplete="off" /></Field>
      <Field label="预期 ZIP 字节数" validationState={bytes && !validBytes ? 'error' : 'none'} validationMessage={bytes && !validBytes ? '请输入 1..536870912 之间的整数' : undefined}><Input value={bytes} inputMode="numeric" maxLength={10} disabled={downloadDisabled} onChange={(_, data) => setBytes(data.value.trim())} /></Field>
      <Field label="下载 ZIP SHA-256" validationState={downloadHash && !/^[0-9a-f]{64}$/.test(downloadHash) ? 'error' : 'none'}><Input value={downloadHash} maxLength={64} disabled={downloadDisabled} onChange={(_, data) => setDownloadHash(data.value.trim())} /></Field>
      <div className="button-row"><Button appearance="primary" disabled={downloadDisabled || !validDownload} onClick={() => { void download({ url, bytes: Number(bytes), sha256: downloadHash }); setUrl(''); }}>下载并校验资源包</Button></div>
      <p>文件保存到应用数据目录。已校验的同一资源包会直接复用；安装与启用分别操作。</p>
    </Card>
    <Card aria-label="安装本地运行资源包"><h2>安装本地资源包</h2>
      <p>选择 ZIP，填写发布方提供的 SHA-256。完整检查通过后会退出引擎并安装，完成后重连普通权限引擎；当前资源选择保持不变。</p>
      <Field label="运行资源 ZIP"><Input value={archive} readOnly placeholder="尚未选择资源包" /><Button disabled={disabled} onClick={() => void pickArchive()}>{picking ? '正在选择…' : '选择 ZIP 资源包'}</Button></Field>
      <Field label="预期 ZIP SHA-256" validationState={sha256 && !validHash ? 'error' : 'none'} validationMessage={sha256 && !validHash ? '请输入 64 位小写十六进制校验值' : undefined}><Input value={sha256} maxLength={64} disabled={disabled} onChange={(_, data) => setSha256(data.value.trim())} placeholder="发布方提供的 64 位小写 SHA-256" /></Field>
      {pickerError && <MessageBar intent="error"><MessageBarBody>{pickerError}</MessageBarBody></MessageBar>}
      <div className="button-row"><Button appearance="primary" disabled={disabled || !archive || !validHash} onClick={() => void start({ type: 'install', archive, sha256 })}>安装资源包并重连引擎</Button></div>
      <p>安装完成后在下方选择版本并点击启用。安装与切换期间请等待输入收尾，不会自动运行游戏任务。</p>
    </Card>
    <Card><div className="card-title"><h2>当前选择</h2><Badge appearance="outline">{catalog ? name(catalog.selection.current) : '尚未读取'}</Badge></div>
      <p>先停止游戏任务并等待输入收尾，再切换版本。目标完整校验通过后会重启引擎，请重新连接游戏；不会自动运行应用。</p>
      <p>安装后自动刷新版本列表；通过 CLI 安装的版本可点击“刷新版本”读取。模型缓存准备不会直接改变运行资源。</p>
      {catalog && <><p>上一选择：{catalog.selection.can_rollback ? name(catalog.selection.previous) : '尚无可回滚版本'}</p><div className="button-row"><Button disabled={disabled || !catalog.selection.can_rollback} onClick={() => void start({ type: 'rollback', revision: catalog.revision })}>回滚并重启引擎</Button></div><p className="resource-path">版本目录：{catalog.directory}</p></>}
    </Card>
    {catalog && <div className="resource-grid">{catalog.versions.map(version => <Card key={version.id} role="group" aria-label={version.version ?? version.id}>
      <div className="card-title"><h2>{version.version ?? '不可用版本'}</h2><Badge appearance="tint" color={!version.compatible ? 'danger' : 'informative'}>{version.id === catalog.selection.current ? '当前选择' : version.compatible ? '清单可用' : '不可用'}</Badge></div>
      <code className="resource-id">{version.id}</code><p>{version.compatible ? '清单与当前引擎兼容，启用前会完整校验模型和流程。' : version.error}</p>
      <div className="button-row"><Button disabled={disabled || !version.compatible || version.id === catalog.selection.current} onClick={() => void start({ type: 'activate', id: version.id, revision: catalog.revision })}>启用并重启引擎</Button></div>
    </Card>)}{catalog.versions.length === 0 && <Card><p>尚未安装独立运行资源包，继续使用随程序资源。</p></Card>}</div>}
    {job && <Card role="status" aria-label="运行资源任务"><div className="card-title"><h2>{job.request.type === 'refresh' ? '版本读取' : job.request.type === 'install' ? '资源包安装' : job.request.type === 'download' ? '资源包下载' : '版本切换'}</h2><Badge appearance="tint" color={job.state === 'failed' ? 'danger' : job.state === 'succeeded' ? 'success' : 'informative'}>{job.state === 'cancelling' ? '正在取消' : active ? '进行中' : job.state === 'failed' ? '失败' : job.state === 'cancelled' ? '已取消' : '已完成'}</Badge></div>
      {job.request.type === 'install' && <p className="resource-path">资源包：{job.request.archive}</p>}
      {active && <><Spinner size="tiny" label={job.state === 'cancelling' ? '正在等待下载与文件收尾' : phases[job.phase] ?? '正在处理资源'}/><ProgressBar value={job.state === 'running' && job.phase === 'downloading' && job.progress?.total_bytes ? job.progress.completed_bytes / job.progress.total_bytes : undefined}/></>}
      {active && job.request.type === 'download' && <div className="button-row"><Button disabled={submitting || job.state === 'cancelling' || !!readError} onClick={() => void cancel(job.id)}>取消资源包下载</Button></div>}
      {job.progress && job.request.type === 'download' && <p>已处理 {job.progress.completed_bytes.toLocaleString()} / {job.progress.total_bytes.toLocaleString()} 字节</p>}
      {job.state === 'succeeded' && job.downloaded && <><p>{job.downloaded.reused ? '已复用本机完整资源包。' : '资源包已下载并通过完整校验。'}安装后请另行启用。</p><p className="resource-path">资源包：{job.downloaded.archive}</p><div className="button-row"><Button disabled={disabled} onClick={() => void start({ type: 'install', archive: job.downloaded!.archive, sha256: job.downloaded!.sha256 })}>安装下载的资源包并重连引擎</Button></div></>}
      {job.error && <MessageBar intent="error"><MessageBarBody>{job.error}</MessageBarBody></MessageBar>}
      {job.state === 'succeeded' && job.request.type !== 'download' && <p>{job.request.type === 'refresh' ? '本机版本已读取。' : job.request.type === 'install' ? `资源包已安装，当前选择保持不变。${job.reconnected ? '引擎已重新连接。' : ''}请在版本列表中另行启用。` : job.reconnected ? '资源选择已保存，引擎已重新连接。请手动连接游戏，需要时再提升引擎权限。' : '资源选择已保存。'}</p>}
      {job.state === 'cancelled' && <p>下载已取消，本次临时文件已清理。</p>}
      {job.state === 'failed' && job.reconnected && <p>引擎已按磁盘上的版本选择重新连接，任务没有自动重试。</p>}
      <p>离开页面后任务继续；退出桌面会取消下载并等待当前步骤收尾。</p>
    </Card>}
  </>;
}
