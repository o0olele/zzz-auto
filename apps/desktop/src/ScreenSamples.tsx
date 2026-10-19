import { useEffect, useRef, useState } from 'react';
import { Badge, Button, Card, Field, Input, MessageBar, MessageBarBody, Select, Spinner } from '@fluentui/react-components';
import type { CaptureMethod, ScreenSampleKind, ScreenSampleState, Snapshot } from './generated/protocol';
import { screenSampleKindOptions } from './generated/protocol';
import { captureScreenSample, desktop, errorMessage, saveScreenSample, screenSampleState } from './bridge';
import { captureNames } from './captureSettings';

export function useScreenSamples(enabled: boolean) {
  const [state, setState] = useState<ScreenSampleState>({ preview: null, saved: null });
  const [loading, setLoading] = useState(false);
  const [capturing, setCapturing] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const visited = useRef(false);
  const generation = useRef(0);
  const working = useRef(false);
  useEffect(() => () => { generation.current++; }, []);

  async function refresh() {
    if (!desktop || working.current) return;
    const requested = ++generation.current;
    setLoading(true); setError('');
    try { const value = await screenSampleState(); if (requested === generation.current) setState(value); }
    catch (reason) { if (requested === generation.current) setError(errorMessage(reason)); }
    finally { if (requested === generation.current) setLoading(false); }
  }
  useEffect(() => {
    if (enabled && !visited.current) { visited.current = true; void refresh(); }
  }, [enabled]);

  async function capture(hwnd: string, method: CaptureMethod) {
    if (!desktop || working.current) return;
    const requested = ++generation.current;
    working.current = true; setCapturing(true); setLoading(false); setError(''); setState({ preview: null, saved: null });
    try { const value = await captureScreenSample(hwnd, method); if (requested === generation.current) setState(value); }
    catch (reason) { if (requested === generation.current) setError(errorMessage(reason)); }
    finally { if (requested === generation.current) { working.current = false; setCapturing(false); } }
  }
  async function save(kind: ScreenSampleKind, note: string) {
    if (!desktop || !state.preview || state.saved || working.current) return;
    const requested = ++generation.current;
    working.current = true; setSaving(true); setLoading(false); setError('');
    try { const value = await saveScreenSample(state.preview.id, kind, note); if (requested === generation.current) setState(value); }
    catch (reason) { if (requested === generation.current) setError(errorMessage(reason)); }
    finally { if (requested === generation.current) { working.current = false; setSaving(false); } }
  }
  return { state, loading, capturing, saving, error, refresh, capture, save };
}

export function ScreenSamples({ manager, connected, blocked, snapshot }: { manager: ReturnType<typeof useScreenSamples>; connected: boolean; blocked: boolean; snapshot: Snapshot | null }) {
  const { state, loading, capturing, saving, error } = manager;
  const [kind, setKind] = useState<ScreenSampleKind>('engagement_confirm');
  const [note, setNote] = useState('');
  const [decodedId, setDecodedId] = useState<string | null>(null);
  const [decodeErrorId, setDecodeErrorId] = useState<string | null>(null);
  const preview = state.preview;
  const image = preview?.screenshot;
  const decoded = !!preview && decodedId === preview.id;
  const decodeError = !!preview && decodeErrorId === preview.id;
  const working = loading || capturing || saving;
  const active = !!snapshot?.run && !['succeeded', 'failed', 'stopped'].includes(snapshot.run.state);
  const captureDisabled = !desktop || !connected || !snapshot?.window || !snapshot.capture_method || blocked || active || !!snapshot.record_error || working;
  const noteInvalid = [...note.trim()].length > 240 || /[\u0000-\u001f\u007f-\u009f]/.test(note.trim());
  useEffect(() => { if (state.saved) { setKind(state.saved.kind); setNote(state.saved.note); } }, [state.saved?.id]);

  return <>
    <div className="page-heading"><div><div className="eyebrow">SCREEN SAMPLES</div><h1>画面素材</h1><p>手动进入需要留档的游戏画面，采集一帧并核对后保存。</p></div><Badge appearance="tint">按需采集 · 本机留档</Badge></div>
    {!desktop && <MessageBar intent="info"><MessageBarBody>浏览器仅供界面预览，素材采集和保存需要桌面应用。</MessageBarBody></MessageBar>}
    {desktop && !connected && <MessageBar intent="info"><MessageBarBody>引擎未连接。已经采集的预览仍可保存，重新采集前请在总览连接游戏。</MessageBarBody></MessageBar>}
    {(active || snapshot?.record_error || blocked) && <MessageBar intent="info"><MessageBarBody>请等待当前任务、记录保存或维护工作完成后再采集。已有预览仍可保存。</MessageBarBody></MessageBar>}
    {error && <MessageBar intent="error"><MessageBarBody>{error}</MessageBarBody></MessageBar>}
    <Card className="preview-card screen-samples-card">
      <div className="card-title"><h2>当前预览</h2><div className="button-row"><Button disabled={!desktop || working} onClick={() => void manager.refresh()}>重读素材预览</Button><Button appearance="primary" disabled={captureDisabled} onClick={() => { if (snapshot?.window && snapshot.capture_method) void manager.capture(snapshot.window.hwnd, snapshot.capture_method); }}>采集当前画面</Button></div></div>
      <p>先在总览选择截图方式并连接游戏。这里只读取一帧，不操作游戏；采集不会自动保存文件。</p>
      {working && <Spinner size="tiny" label={capturing ? '正在采集画面' : saving ? '正在保存素材' : '读取素材预览'}/>}
      {image?.quality.near_black && <MessageBar intent="warning"><MessageBarBody>预览疑似黑帧：暗像素 {(image.quality.dark_pixel_ratio * 100).toFixed(2)}%。请核对游戏是否处于加载画面，必要时重新连接其他截图方式。</MessageBarBody></MessageBar>}
      {decodeError && <MessageBar intent="error"><MessageBarBody>预览图片解码失败，请重新采集后保存。</MessageBarBody></MessageBar>}
      <div className="preview">{image ? <img key={preview!.id} alt="待保存的游戏画面素材" src={`data:image/png;base64,${image.png_base64}`} onLoad={() => setDecodedId(preview!.id)} onError={() => { setDecodedId(null); setDecodeErrorId(preview!.id); }}/> : <div className="preview-empty"><strong>尚无素材预览</strong><span>手动进入奖励等所需画面后，点击“采集当前画面”。</span></div>}</div>
      {preview && image && <><div className="preview-meta"><span>{image.width} × {image.height} · {captureNames[preview.source.capture_method]}</span><time>{new Date(image.captured_at).toLocaleString('zh-CN', { hour12: false })}</time></div><p className="resource-path">本张来源：进程 {preview.source.pid} · 窗口 {preview.source.hwnd} · 会话 {preview.source.session_id}</p><p className="resource-path">SHA-256：{preview.sha256}</p></>}
      <div className="input-fields"><Field label="画面状态"><Select value={kind} disabled={working || !!state.saved} onChange={(_, value) => setKind(value.value as ScreenSampleKind)}>{screenSampleKindOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</Select></Field><Field label="画面备注" validationState={noteInvalid ? 'error' : 'none'} validationMessage={noteInvalid ? '备注最多 240 字且不能包含控制字符' : undefined}><Input value={note} disabled={working || !!state.saved} onChange={(_, value) => setNote(value.value)} placeholder="可填写咖啡种类、活跃度或弹窗状态"/></Field></div>
      <p>状态和备注是人工标注，不代表已完成识别或领奖。保存这张预览后，PNG 和元信息会写入本机数据目录 diagnostics/screens。</p>
      <div className="button-row"><Button appearance="primary" disabled={!desktop || !preview || !decoded || noteInvalid || working || !!state.saved} onClick={() => void manager.save(kind, note)}>保存这张预览</Button>{state.saved && <Badge color="success" appearance="tint">本张已保存</Badge>}</div>
      {state.saved && <><Field label="素材图片路径"><Input readOnly value={state.saved.image_path}/></Field><Field label="素材元信息路径"><Input readOnly value={state.saved.metadata_path}/></Field><p>可从这些路径取出文件用于后续画面建档。更换标注或继续采集其他状态时，请重新采集一帧。</p></>}
    </Card>
  </>;
}
