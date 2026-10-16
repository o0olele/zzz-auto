import { useCallback, useEffect, useMemo, useState } from 'react';
import {
  Badge, Button, Card, Divider, Field, FluentProvider, Input, MessageBar, MessageBarBody,
  ProgressBar, Select, Spinner, Tab, TabList, Text, Tooltip, webDarkTheme, webLightTheme,
} from '@fluentui/react-components';
import {
  Apps24Regular, ArrowClockwise20Regular, ArrowRight20Regular, CheckmarkCircle20Regular,
  ChevronRight20Regular, Code24Regular, Desktop24Regular, DocumentText24Regular,
  Home24Regular, Image24Regular, Play20Regular, PlugConnected24Regular, Search20Regular,
  Stop20Regular, WeatherMoon20Regular, WeatherSunny20Regular, Settings24Regular,
} from '@fluentui/react-icons';
import type { AppDefinition, CaptureMethod, EnginePrivilege, GeneralSettings as Settings, Screenshot, Snapshot, WindowInfo } from './generated/protocol';
import { desktop, elevateEngine, errorMessage, reconnect, recoverInputs, request, subscribe } from './bridge';
import catalog from '../../../resources/catalog/applications.json';
import { InputDiagnostics } from './InputDiagnostics';
import { MailDiagnostics } from './MailDiagnostics';
import { MapDiagnostics } from './MapDiagnostics';
import { AccountSettings } from './AccountSettings';
import { GeneralSettings } from './GeneralSettings';
import { captureNames, captureHints } from './captureSettings';

type Page = 'home' | 'accounts' | 'settings' | 'apps' | 'native' | 'progress';
const pages: { id: Page; label: string; icon: React.ReactElement }[] = [
  { id: 'home', label: '总览', icon: <Home24Regular /> },
  { id: 'accounts', label: '账号与应用组', icon: <Apps24Regular /> },
  { id: 'settings', label: '通用设置', icon: <Settings24Regular /> },
  { id: 'apps', label: '应用迁移', icon: <Apps24Regular /> },
  { id: 'native', label: '原生诊断', icon: <Desktop24Regular /> },
  { id: 'progress', label: '开发与验收', icon: <DocumentText24Regular /> },
];
const modeNames: Record<string, string> = { pipeline: 'Maa Pipeline', rust: 'Rust', hybrid: 'Rust + Pipeline', rust_service: 'Rust 服务' };
const runNames: Record<string, string> = { running: '运行中', pausing: '暂停中', paused: '已暂停', stopping: '停止中', succeeded: '完成', failed: '失败', stopped: '已停止' };
const milestones = [
  ['M0', '清单与基线', '清单与基线已完成，参考模型已按用户决定接受。'],
  ['M1', 'Maa 集成原型', '原生 SDK、独立引擎、桌面通信与截图入口正在验证。'],
  ['M2', '首个日常 Pipeline', '主流程通过，边界待验收；暂停、异常恢复等继续验证。'],
  ['M3', '应用与桌面基础', '进行中：账号导入、应用组邮件试运行、账号历史、通用设置与每日计划；后续推进咖啡和多应用执行。'],
  ['M4', 'Rust 自动战斗', '状态时间窗、动作抢占、声音闪避和输入释放。'],
  ['M5', '剩余游戏应用', '探索、路径、经营及剩余应用逐项迁移。'],
  ['M6', '工具与外部能力', '图像与路线编辑器、HTTP、MCP 和 Rust Agent。'],
  ['M7', '发布与替代', '安装、更新、数据恢复和干净 Windows 环境验收。'],
];

export default function App() {
  const [page, setPage] = useState<Page>('home');
  const [dark, setDark] = useState(() => localStorage.getItem('theme') === 'dark' || (!localStorage.getItem('theme') && matchMedia('(prefers-color-scheme: dark)').matches));
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [apps, setApps] = useState<AppDefinition[]>(catalog);
  const [windows, setWindows] = useState<WindowInfo[]>([]);
  const [hwnd, setHwnd] = useState('');
  const [screenshot, setScreenshot] = useState<Screenshot | null>(null);
  const [captureMethod, setCaptureMethod] = useState<CaptureMethod>('frame_pool');
  const [settings, setSettings] = useState<Settings | null>(null);
  const [query, setQuery] = useState('');
  const [filter, setFilter] = useState('all');
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [connected, setConnected] = useState(false);
  const [privilege, setPrivilege] = useState<EnginePrivilege | null>(null);
  const [elevating, setElevating] = useState(false);
  const [recoveryRequired, setRecoveryRequired] = useState(false);
  const [recoveryMessage, setRecoveryMessage] = useState('');
  const [recovering, setRecovering] = useState(false);
  const [selected, setSelected] = useState<AppDefinition | null>(null);

  const active = !!snapshot?.run && ['running', 'pausing', 'paused', 'stopping'].includes(snapshot.run.state);
  const filtered = useMemo(() => apps.filter(app => (filter === 'all' || app.execution === filter) && `${app.name} ${app.id}`.toLowerCase().includes(query.toLowerCase())), [apps, query, filter]);

  useEffect(() => { localStorage.setItem('theme', dark ? 'dark' : 'light'); document.documentElement.dataset.theme = dark ? 'dark' : 'light'; }, [dark]);
  const acceptSettings = useCallback((value: Settings) => {
    setSettings(value); setCaptureMethod(value.capture_method); setScreenshot(null);
  }, []);
  useEffect(() => {
    let alive = true;
    setSettings(null);
    if (connected) request({ type: 'account_config' }).then(value => {
      if (value.type !== 'account_config') throw new Error('引擎未返回通用设置');
      if (alive) acceptSettings(value.data.general);
    }).catch(reason => { if (alive) setError(`读取通用设置失败：${errorMessage(reason)}`); });
    return () => { alive = false; };
  }, [connected, acceptSettings]);
  useEffect(() => {
    let alive = true;
    subscribe(update => {
      if (!alive) return;
      if (update.type === 'privilege') { setPrivilege(update.data); }
      else if (update.type === 'snapshot') { setSnapshot(update.data); setConnected(true); }
      else { setConnected(false); setSnapshot(null); setScreenshot(null); setError(update.data.message); if (update.data.code.startsWith('input_recovery')) setRecoveryRequired(true); }
    }).catch(reason => { if (alive) setError(errorMessage(reason)); });
    if (desktop) request({ type: 'applications' }).then(value => { if (alive && value.type === 'applications') setApps(value.data); }).catch(() => {});
    return () => { alive = false; };
  }, []);

  async function act(operation: () => Promise<void>) {
    setError(''); setBusy(true);
    try { await operation(); } catch (reason) { setError(errorMessage(reason)); if (typeof reason === 'object' && reason && 'code' in reason && String(reason.code).startsWith('input_recovery')) { setRecoveryRequired(true); setConnected(false); setSnapshot(null); setScreenshot(null); } }
    finally { setBusy(false); }
  }

  async function scanWindows() {
    const value = await request({ type: 'windows' });
    if (value.type === 'windows') { setWindows(value.data); setHwnd(value.data[0]?.hwnd ?? ''); }
  }

  async function capture() {
    setScreenshot(null);
    const targetHwnd = snapshot?.window?.hwnd ?? hwnd;
    if (!snapshot?.window || captureMethod !== snapshot.capture_method) await connectWindow(targetHwnd);
    const value = await request({ type: 'screenshot' });
    if (value.type === 'screenshot') setScreenshot(value.data);
  }

  async function connectWindow(targetHwnd = hwnd) {
    setScreenshot(null);
    const value = await request({ type: 'attach', params: { hwnd: targetHwnd, capture_method: captureMethod } });
    if (value.type === 'snapshot') setSnapshot(value.data);
  }

  const captureSelect = <Field label="截图方式"><Select value={captureMethod} onChange={(_, data) => { setCaptureMethod(data.value as CaptureMethod); setScreenshot(null); }} disabled={busy || active || !settings}>{Object.entries(captureNames).map(([value, label]) => <option key={value} value={value}>{label}{value === settings?.capture_method ? '（已保存默认）' : ''}</option>)}</Select></Field>;

  const status = !desktop ? '界面预览' : !connected ? '引擎未连接' : snapshot?.sdk.ready ? '引擎已就绪' : 'SDK 待配置';
  return (
    <FluentProvider theme={dark ? webDarkTheme : webLightTheme} className="app-shell">
      <aside className="sidebar">
        <div className="brand"><div className="brand-mark">Z</div><div><strong>绝区零一条龙</strong><span>Rust · 原生自动化</span></div></div>
        <div className="workspace-label">工作台</div>
        <nav aria-label="主导航">{pages.map(item => <button key={item.id} className={`nav-item ${page === item.id ? 'selected' : ''}`} onClick={() => setPage(item.id)} aria-current={page === item.id ? 'page' : undefined}>{item.icon}<span>{item.label}</span>{page === item.id && <ChevronRight20Regular />}</button>)}</nav>
        <div className="sidebar-note"><Code24Regular/><strong>迁移原型 · v0.1</strong><p>邮件主流程通过，边界继续验收；账号区服与应用组每日计划已接通。</p><Badge appearance="tint" color="brand">M3 进行中</Badge></div>
        <div className="sidebar-footer"><span><i className={`status-dot ${connected && snapshot?.sdk.ready ? 'ready' : ''}`}/>{status}</span><Tooltip content="切换明暗主题" relationship="label"><Button appearance="subtle" icon={dark ? <WeatherSunny20Regular/> : <WeatherMoon20Regular/>} onClick={() => setDark(!dark)} aria-label="切换明暗主题"/></Tooltip></div>
      </aside>

      <main className="main">
        <header className="topbar"><span>工作台 <ChevronRight20Regular/> {pages.find(item => item.id === page)?.label}</span><Badge appearance="outline">Windows x64</Badge></header>
        <div className="content">
          {!desktop && <MessageBar intent="info"><MessageBarBody>当前为浏览器预览。使用 <code>npm run tauri dev</code> 启动桌面，连接真实 Maa 引擎。</MessageBarBody></MessageBar>}
          {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
          {(recoveryRequired || recoveryMessage) && <Card><div className="card-title"><h2>输入恢复</h2><Badge appearance="outline">{recoveryRequired ? '需要恢复' : '已处理'}</Badge></div>
            <p>{recoveryRequired ? '引擎退出前未确认输入已释放。请停止使用自动化；恢复将只释放记录中的输入，管理员引擎可能需要再次授权 Windows。恢复成功后请确认游戏状态，再手动重连。' : recoveryMessage}</p>
            {recoveryRequired && <Button disabled={!desktop || busy} onClick={() => act(async () => {
              setRecovering(true);
              try { const message = await recoverInputs(); setRecoveryRequired(false); setRecoveryMessage(message); setSnapshot(null); setWindows([]); setHwnd(''); setScreenshot(null); }
              finally { setRecovering(false); }
            })}>恢复遗留输入</Button>}
            {recovering && <Spinner size="tiny" label="等待 Windows 授权与输入释放"/>}
          </Card>}

          {page === 'home' && <>
            <div className="page-heading"><div><div className="eyebrow">AUTOMATION WORKSPACE</div><h1>控制台</h1><p>连接游戏窗口，查看引擎状态与迁移进展。</p></div><Button icon={<ArrowClockwise20Regular/>} disabled={!desktop || busy || active || recoveryRequired} onClick={() => act(async () => { await reconnect(); await scanWindows(); })}>重连引擎</Button></div>
            <div className="stats-grid">
              <Card className="stat"><span className="stat-label">原生引擎<PlugConnected24Regular/></span><strong>{snapshot?.sdk.ready ? '已就绪' : '等待连接'}</strong><span>{snapshot?.sdk.version ? `MaaFramework ${snapshot.sdk.version}` : 'Rust + MaaFramework'}</span></Card>
              <Card className="stat"><span className="stat-label">游戏窗口<Desktop24Regular/></span><strong>{snapshot?.window ? '已连接' : '未连接'}</strong><span>{snapshot?.window ? `${snapshot.window.width} × ${snapshot.window.height}` : '1080p 基准 · 客户区'}</span></Card>
              <Card className="stat"><span className="stat-label">应用登记<Apps24Regular/></span><strong>{apps.length}<small> 个</small></strong><span>{apps.filter(app => app.implemented).length} 个完成迁移与验收</span></Card>
            </div>
            <Card><div className="card-title"><h2>引擎权限</h2><Badge appearance="outline">{!connected ? '未连接' : privilege ? (privilege.engine_integrity >= 12288 ? '管理员引擎' : '普通引擎') : '正在读取'}</Badge></div>
              <p>游戏以管理员身份运行时，可以只提升自动化引擎的权限。Windows 会请求授权，桌面界面保持当前权限；请先停止任务并等待收尾。</p>
              <div className="button-row"><Button disabled={!desktop || !connected || recoveryRequired || busy || active || !privilege || privilege.engine_integrity >= 12288} onClick={() => act(async () => {
                setElevating(true);
                try {
                  await elevateEngine(); setScreenshot(null); setSnapshot(null);
                  const value = await request({ type: 'snapshot' });
                  if (value.type === 'snapshot') setSnapshot(value.data);
                  await scanWindows();
                } finally { setElevating(false); }
              })}>提升引擎权限</Button>{elevating && <Spinner size="tiny" label="等待 Windows 授权与引擎切换"/>}</div>
              {privilege && privilege.engine_integrity >= 12288 && <p>引擎已提升权限，请重新连接游戏。权限切换不会自动运行任务。</p>}
              {privilege && privilege.desktop_integrity >= 12288 && <p>当前桌面本身也以管理员身份运行。要使用普通权限的界面，请退出后正常启动应用。</p>}
            </Card>
            <div className="home-grid">
              <Card className="connect-card"><div className="card-title"><h2>连接游戏</h2><Badge appearance="tint" color={snapshot?.window ? 'success' : 'informative'}>{snapshot?.window ? '已连接' : '尚未连接'}</Badge></div><p>启动绝区零并设置 1920 × 1080 客户区，再选择游戏窗口。</p><Field label="游戏窗口"><Select value={hwnd} onChange={(_, data) => setHwnd(data.value)} disabled={busy || active}><option value="">{windows.length ? '选择游戏窗口' : '尚未发现窗口'}</option>{windows.map(window => <option key={window.hwnd} value={window.hwnd}>{window.title} · PID {window.pid} · {window.width}×{window.height}</option>)}</Select></Field>{captureSelect}<div className="button-row"><Button disabled={!connected || busy || active} icon={<ArrowClockwise20Regular/>} onClick={() => act(scanWindows)}>扫描窗口</Button><Button appearance="primary" disabled={!hwnd || !connected || recoveryRequired || busy || active || !settings} icon={<PlugConnected24Regular/>} onClick={() => act(() => connectWindow())}>连接窗口</Button></div><Divider/><div className="connection-detail"><Text size={200}>SDK 状态</Text><p>{snapshot?.sdk.message ?? '桌面启动后自动校验随附 SDK 与引擎。'}</p></div></Card>
              <Card className="next-card"><div className="card-title"><h2>当前阶段</h2><Badge appearance="tint" color="brand">M3 进行中</Badge></div><h3>应用组每日计划</h3><p>M2 主流程通过，边界待验收；M3 已接入账号配置、应用组邮件试运行、账号历史、通用设置及每日跳过策略。</p><div className="milestone-mini"><CheckmarkCircle20Regular/><span>邮件大世界 / 菜单起点主流程通过</span></div><div className="milestone-mini"><div className="step-dot"/><span>应用组预检、修订核对与按账号记录</span></div><div className="milestone-mini muted"><div className="step-dot hollow"/><span>咖啡、多应用执行与资源下载</span></div><Button appearance="subtle" icon={<ArrowRight20Regular/>} iconPosition="after" onClick={() => setPage('accounts')}>管理账号与应用组</Button></Card>
            </div>
            <Card><div className="card-title"><h2>最近运行</h2><Badge appearance="tint" color={active ? 'brand' : 'informative'}>{snapshot?.run ? runNames[snapshot.run.state] : '空闲'}</Badge></div>{snapshot?.run ? <><strong>{['prototype.input_probe', 'prototype.input_pipeline'].includes(snapshot.run.app_id) ? '真实输入诊断' : snapshot.run.app_id === 'email' ? '邮件试运行' : 'Maa 原生健康检查'}</strong><p className="run-message">{snapshot.run.message}</p><Text size={200}>运行 ID：{snapshot.run.id}</Text></> : <div className="empty-row"><Play20Regular/><span>尚无运行记录。连接窗口后可在原生诊断中执行健康检查。</span></div>}</Card>
          </>}

          {page === 'accounts' && <AccountSettings connected={desktop && connected} active={active} blocked={recoveryRequired || busy} apps={apps} snapshot={snapshot} onCommand={command => act(async () => { const value = await request(command); if (value.type === 'run') setSnapshot(current => current ? { ...current, run: value.data } : current); else if (value.type === 'snapshot') setSnapshot(value.data); })}/>}

          {page === 'settings' && <GeneralSettings connected={desktop && connected} active={active} blocked={recoveryRequired || busy} onLoaded={acceptSettings}/>}

          {page === 'apps' && <>
            <div className="page-heading"><div><div className="eyebrow">APPLICATION CATALOG</div><h1>应用迁移</h1><p>保留现有应用 ID，逐项记录实现与验收状态。</p></div><Badge appearance="tint" color="brand">{apps.length} 个已登记</Badge></div>
            <div className="filter-row"><Input contentBefore={<Search20Regular/>} placeholder="搜索应用名称或 ID" value={query} onChange={(_, data) => setQuery(data.value)} aria-label="搜索应用"/><TabList selectedValue={filter} onTabSelect={(_, data) => setFilter(String(data.value))}><Tab value="all">全部</Tab><Tab value="pipeline">Pipeline</Tab><Tab value="hybrid">混合流程</Tab><Tab value="rust">Rust</Tab><Tab value="rust_service">服务</Tab></TabList></div>
            <Card className="app-table"><div className="table-head"><span>应用</span><span>执行方式</span><span>迁移状态</span><span/></div>{filtered.map(app => <button className={`app-row ${selected?.id === app.id ? 'active-row' : ''}`} key={app.id} onClick={() => setSelected(app)}><span><strong>{app.name}</strong><small>{app.id}</small></span><span>{modeNames[app.execution]}</span><span><Badge appearance="tint" color={app.implemented ? 'success' : 'informative'}>{app.implemented ? '已验收' : '待迁移'}</Badge></span><ChevronRight20Regular/></button>)}{!filtered.length && <div className="empty-row">没有匹配的应用</div>}</Card>
            {selected && <Card className="app-detail"><div className="card-title"><h2>{selected.name}</h2><Badge appearance="outline">{modeNames[selected.execution]}</Badge></div><p>{selected.reason}</p><Text size={200}>来源：{selected.factory}</Text><p>{selected.configs.length} 份配置 · {selected.run_records.length} 份运行记录定义 · {selected.settings.length} 个专用设置界面</p><Button disabled icon={<Play20Regular/>}>验收后开放运行</Button></Card>}
          </>}

          {page === 'native' && <>
            <div className="page-heading"><div><div className="eyebrow">NATIVE INTEGRATION</div><h1>原生诊断</h1><p>验证真实截图、输入与 Pipeline 的运行和停止行为。</p></div><Badge appearance="outline">CPU · {snapshot?.capture_method ? captureNames[snapshot.capture_method] : "未连接"}</Badge></div>
            <Card className="preview-card"><div className="card-title"><h2>截图预览</h2><Button icon={<Image24Regular/>} disabled={(!snapshot?.window && !hwnd) || !connected || recoveryRequired || busy || active || !settings} onClick={() => act(capture)}>获取截图</Button></div>{captureSelect}<p>{captureHints[captureMethod]} 更改方式后，点击“获取截图”会重新连接并应用。</p>{screenshot?.quality.near_black && <MessageBar intent="warning"><MessageBarBody>检测到疑似黑帧：暗像素 {(screenshot.quality.dark_pixel_ratio * 100).toFixed(2)}%，平均亮度 {screenshot.quality.mean_luma.toFixed(2)}/255。请等待游戏退出加载画面；若游戏已有正常画面，尝试 PrintWindow 或 FramePool。使用 DXGI / ScreenDC 时，请把游戏完整显示在屏幕上。</MessageBarBody></MessageBar>}<div className="preview">{screenshot ? <img alt="Maa 捕获的游戏截图" src={`data:image/png;base64,${screenshot.png_base64}`} onError={() => { setScreenshot(null); setError("截图图片解码失败，请重新获取并检查引擎日志。"); }}/> : <div className="preview-empty"><Image24Regular/><strong>尚无截图</strong><span>连接游戏窗口后，手动获取一帧画面。</span></div>}</div><div className="preview-meta"><span>{screenshot ? `${screenshot.width} × ${screenshot.height} · ${screenshot.capture_method ? captureNames[screenshot.capture_method] : "离线"} · ${screenshot.color_order}` : '1920 × 1080 · PNG'}</span><span>{screenshot ? `采集用时 ${screenshot.elapsed_ms} ms` : '按需预览'}</span></div></Card>
            <MailDiagnostics snapshot={snapshot} connected={connected && !recoveryRequired} busy={busy} defaults={settings?.mail ?? null} onCommand={command => act(async () => { const value = await request(command); if (value.type === 'run') setSnapshot(current => current ? { ...current, run: value.data } : current); })}/>
            <MapDiagnostics snapshot={snapshot} connected={connected && !recoveryRequired} busy={busy} onCommand={command => act(async () => { const value = await request(command); if (value.type === 'run') setSnapshot(current => current ? { ...current, run: value.data } : current); else if (value.type === 'snapshot') setSnapshot(value.data); })}/>
            <InputDiagnostics snapshot={snapshot} connected={connected && !recoveryRequired} busy={busy} onStart={(config, pipelineTimeout) => act(async () => { const value = await request(pipelineTimeout === null ? { type: 'start_input_probe', params: { config, allow_input: true } } : { type: 'start_input_pipeline', params: { config: { input: config, timeout_ms: pipelineTimeout }, allow_input: true } }); if (value.type === 'run') setSnapshot(current => current ? { ...current, run: value.data } : current); })} onStop={() => act(async () => { if (snapshot?.run) { const value = await request({ type: 'stop', params: { run_id: snapshot.run.id } }); if (value.type === 'run') setSnapshot(current => current ? { ...current, run: value.data } : current); } })}/>
            <Card><div className="card-title"><h2>Pipeline 健康检查</h2><div className="button-row"><Button appearance="primary" icon={<Play20Regular/>} disabled={!snapshot?.window || !connected || recoveryRequired || busy || active} onClick={() => act(async () => { const value = await request({ type: 'start_probe' }); if (value.type === 'run') setSnapshot(current => current ? { ...current, run: value.data } : current); })}>运行检查</Button><Button icon={<Stop20Regular/>} disabled={!active || !connected || snapshot?.run?.app_id !== 'prototype.health_check' || snapshot?.run?.state === 'stopping'} onClick={() => act(async () => { if (snapshot?.run) await request({ type: 'stop', params: { run_id: snapshot.run.id } }); })}>停止</Button></div></div><p>执行固定的 DirectHit / DoNothing 节点，不产生游戏点击或按键。停止后等待原生任务与队列完成收尾。</p>{active && snapshot?.run?.app_id === 'prototype.health_check' && <div className="running-indicator"><Spinner size="tiny" label={snapshot?.run?.state === 'stopping' ? '正在等待停止完成' : '原生 Pipeline 运行中'}/><ProgressBar/></div>}{snapshot?.run?.app_id === 'prototype.health_check' && <div className="result-line"><Badge appearance="tint">{runNames[snapshot.run.state]}</Badge><code>{snapshot.run.message}</code></div>}</Card>
            <Card className="log-card"><div className="card-title"><h2>引擎日志</h2><Text size={200}>最多保留 256 条 · 已丢弃 {snapshot?.dropped_events ?? 0} 条</Text></div><div className="logs" role="log" aria-label="引擎运行日志">{snapshot?.events.slice(-80).map(event => <div className={`log-line ${event.level}`} key={event.sequence}><time>{new Date(event.at).toLocaleTimeString('zh-CN', { hour12: false })}</time><span>{event.level.toUpperCase()}</span><p>{event.message}</p></div>)}{!snapshot?.events.length && <div className="empty-row">连接引擎后显示诊断日志</div>}</div></Card>
          </>}

          {page === 'progress' && <>
            <div className="page-heading"><div><div className="eyebrow">MIGRATION ROADMAP</div><h1>开发与验收</h1><p>阶段完成以验收证据为准，编译与界面预览仅是其中一部分。</p></div><Badge appearance="tint" color="brand">M2 主流程通过，边界待验收 · M3 进行中</Badge></div>
            <Card className="roadmap-card">{milestones.map(([id, title, description], index) => <div className={`milestone ${index === 3 ? 'current' : ''}`} key={id}><div className="milestone-number">{id}</div><div><h3>{title}</h3><p>{description}</p></div><Badge appearance="tint" color={index === 0 ? 'success' : index < 4 ? 'brand' : 'informative'}>{index === 0 ? '已完成' : index === 1 ? '待验证' : index === 2 ? '主流程通过，边界待验收' : index === 3 ? '进行中' : '待开始'}</Badge></div>)}</Card>
            <Card><div className="card-title"><h2>本轮验证边界</h2><Badge appearance="outline">首版 Windows x64</Badge></div><p>CPU / DirectML 模型、前后台键鼠、Xbox / DS4、权限提升、崩溃后的独立输入恢复与长时间稳定性，需要固定语料和实机记录。日常、战斗、账号导入与第三方插件按后续阶段逐项开放。</p><p className="muted">开发入口与命令见工程 README；迁移决策与验收要求见 docs/ROADMAP.md。</p></Card>
          </>}
        </div>
        <footer className="main-footer"><span>绝区零一条龙 · Rust 原型</span><span>Tauri 2 <span className="footer-dot">·</span> MaaFramework <span className="footer-dot">·</span> 1080p</span></footer>
      </main>
    </FluentProvider>
  );
}
