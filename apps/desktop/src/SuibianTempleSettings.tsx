import { useState } from 'react';
import { Badge, Button, Card, Checkbox, Field, Input, Select } from '@fluentui/react-components';
import { suibianTempleDefaults, suibianDurationOptions, suibianMissionOptions, suibianPriceOptions, type SuibianTempleConfig, type LegacyPawnshopConfig, type DailyProgress } from './generated/protocol';

const yes = (value: boolean) => value ? '开' : '关';
const optionName = (options: readonly { value: string; label: string }[], value: string) => options.find(o => o.value === value)?.label ?? value;
const goodsNames: Record<string, string> = { hifi_master_copy: '高保真母盘', senior_investigator_log: '资深调查员记录', w_engine_energy_module: '音擎能源模块', ether_plating_agent: '以太镀剂', prepaid_power_card: '储值电卡', bangboo_system_widget: '邦布系统控件', denny: '丁尼' };
export function suibianTempleSummary(settings: SuibianTempleConfig = suibianTempleDefaults) {
  return `自动托管${yes(settings.auto_manage_enabled)} · 饮茶仙${yes(settings.yum_cha_sin)} / 定期采办刷新${yes(settings.yum_cha_sin_period_refresh)} · 游历${optionName(suibianDurationOptions, settings.adventure_duration)}：${settings.adventure_missions.map(m => optionName(suibianMissionOptions, m)).join('、')} · 制造下拉${settings.craft_drag_times}次 · 好物铺${yes(settings.good_goods_purchase_enabled)} · 邦巢${yes(settings.boo_box_purchase_enabled)}（游历${optionName(suibianPriceOptions, settings.boo_box_adventure_price)} / 制造${optionName(suibianPriceOptions, settings.boo_box_craft_price)} / 售卖${optionName(suibianPriceOptions, settings.boo_box_sell_price)}） · 总超时${settings.timeout_ms}ms`;
}
export function LegacyPawnshopSummary({ settings = suibianTempleDefaults.legacy_pawnshop }: { settings?: LegacyPawnshopConfig }) {
  return <p>旧德丰大押设置（不执行）：百通宝{yes(settings.omnicoin_enabled)}，优先级 {settings.omnicoin_priority.map(n => goodsNames[n] ?? n).join(' → ') || '空'}；云纹徽{yes(settings.crest_enabled)}，优先级 {settings.crest_priority.map(n => goodsNames[n] ?? n).join(' → ') || '空'}；不限购丁尼{yes(settings.crest_unlimited_denny_enabled)}。</p>;
}
export function SuibianTempleSummary({ settings = suibianTempleDefaults }: { settings?: SuibianTempleConfig }) {
  return <div><p>随便观设置：{suibianTempleSummary(settings)}。</p><LegacyPawnshopSummary settings={settings.legacy_pawnshop}/></div>;
}
export function SuibianTempleSettings({ settings = suibianTempleDefaults, disabled, onSave }: {
  settings?: SuibianTempleConfig; disabled: boolean; onSave: (settings: SuibianTempleConfig) => void;
}) {
  const [draft, setDraft] = useState(() => structuredClone(settings));
  const [drags, setDrags] = useState(String(settings.craft_drag_times));
  const [timeout, setTimeout] = useState(String(settings.timeout_ms));
  const validDrags = /^\d{1,2}$/.test(drags) && Number(drags) <= 50;
  const validTimeout = /^\d{1,7}$/.test(timeout) && Number(timeout) >= 1000 && Number(timeout) <= 3600000;
  const candidate = { ...draft, craft_drag_times: Number(drags), timeout_ms: Number(timeout) };
  const changed = JSON.stringify(candidate) !== JSON.stringify(settings);
  const toggle = (key: 'auto_manage_enabled' | 'yum_cha_sin' | 'yum_cha_sin_period_refresh' | 'good_goods_purchase_enabled' | 'boo_box_purchase_enabled', value: boolean) => setDraft(current => ({ ...current, [key]: value }));
  return <Card aria-label="应用组随便观设置">
    <div className="card-title"><h2>随便观</h2><Badge appearance="outline">按账号与应用组保存</Badge></div>
    <p>按满级随便观的托管、游历、制造、售卖及可选购买保存设置。可从独立应用或一条龙试运行，完整游戏流程待实机验收。</p>
    <Checkbox label="随便观自动托管" checked={draft.auto_manage_enabled} disabled={disabled} onChange={(_, d) => toggle('auto_manage_enabled', d.checked === true)}/>
    <p>开启后停止托管领取收益并重新开始托管；关闭后采用手动经营设置。好物铺与邦巢按各自开关处理。</p>
    <Checkbox label="随便观饮茶仙" checked={draft.yum_cha_sin} disabled={disabled} onChange={(_, d) => toggle('yum_cha_sin', d.checked === true)}/>
    <Checkbox label="随便观定期采办刷新" checked={draft.yum_cha_sin_period_refresh} disabled={disabled} onChange={(_, d) => toggle('yum_cha_sin_period_refresh', d.checked === true)}/>
    <Field label="随便观游历时长"><Select value={draft.adventure_duration} disabled={disabled} onChange={(_, d) => setDraft(c => ({ ...c, adventure_duration: d.value as SuibianTempleConfig['adventure_duration'] }))}>
      {suibianDurationOptions.map(o => <option key={o.value} value={o.value}>{o.label}</option>)}
    </Select></Field>
    <div className="input-fields">{draft.adventure_missions.map((mission, index) => <Field key={index} label={`随便观游历副本 ${index + 1}`}><Select value={mission} disabled={disabled} onChange={(_, d) => setDraft(c => { const missions = [...c.adventure_missions] as SuibianTempleConfig['adventure_missions']; missions[index] = d.value; return { ...c, adventure_missions: missions }; })}>
      {suibianMissionOptions.map(o => <option key={o.value} value={o.value}>{o.label}</option>)}
    </Select></Field>)}</div>
    <Field label="随便观制造下拉次数" validationState={validDrags ? 'none' : 'error'} validationMessage={validDrags ? undefined : '请输入 0..50 的整数。'}><Input value={drags} inputMode="numeric" maxLength={2} disabled={disabled} onChange={(_, d) => setDrags(d.value)}/></Field>
    <Checkbox label="随便观好物铺购买" checked={draft.good_goods_purchase_enabled} disabled={disabled} onChange={(_, d) => toggle('good_goods_purchase_enabled', d.checked === true)}/>
    <p>开启后购买邦布能源插件，将消耗随便观货币。</p>
    <Checkbox label="随便观邦巢购买" checked={draft.boo_box_purchase_enabled} disabled={disabled} onChange={(_, d) => toggle('boo_box_purchase_enabled', d.checked === true)}/>
    <div className="input-fields">{([['boo_box_adventure_price', '游历'], ['boo_box_craft_price', '制造'], ['boo_box_sell_price', '售卖']] as const).map(([key, name]) => <Field key={key} label={`随便观${name}邦布最低价格`}><Select value={draft[key]} disabled={disabled} onChange={(_, d) => setDraft(c => ({ ...c, [key]: d.value as SuibianTempleConfig[typeof key] }))}>
      {suibianPriceOptions.map(o => <option key={o.value} value={o.value}>{o.label}</option>)}
    </Select></Field>)}</div>
    <Field label="随便观总超时（ms）" validationState={validTimeout ? 'none' : 'error'} validationMessage={validTimeout ? undefined : '请输入 1000..3600000 的整数。'}><Input value={timeout} inputMode="numeric" maxLength={7} disabled={disabled} onChange={(_, d) => setTimeout(d.value)}/></Field>
    <details><summary>保留的旧德丰大押设置</summary><p>旧应用已停用此功能；仅保留旧配置，不执行兑换，也不自动转换为新商店。</p><LegacyPawnshopSummary settings={draft.legacy_pawnshop}/></details>
    <Button disabled={disabled || !validDrags || !validTimeout || !changed} onClick={() => onSave(candidate)}>保存随便观设置</Button>
  </Card>;
}

const operationNames: Record<string, string> = { auto_manage: '自动托管', adventure_claim_only: '游历收获', adventure_claim_and_dispatch: '游历收获与派遣', yum_cha_sin: '饮茶仙与补料', adventure_dispatch_only: '游历派遣', craft: '制造坊', sales: '售卖铺', good_goods: '好物铺', boo_box: '邦巢' };
export function SuibianTempleStatus({ progress }: { progress: DailyProgress }) {
  const flow = progress.temple;
  if (!flow) return null;
  const phaseNames: Record<string, string> = { entering: '前往随便观', ready: '子流程交接', child: '处理经营', returning: '返回大世界', done: '流程完成' };
  return <section aria-label="随便观运行进度">
    <p>随便观 · {flow.config.auto_manage_enabled ? '自动托管' : '手动经营'} · {phaseNames[flow.phase] ?? flow.phase}{flow.current ? ` · ${operationNames[flow.current] ?? flow.current}` : ''}</p>
    <p>已完成：{flow.completed.map(op => operationNames[op] ?? op).join('、') || '暂无'} · {flow.returned_world ? '已返回大世界' : '等待返回大世界'} · {progress.inputs_released ? '输入已释放' : '等待输入收尾'}</p>
  </section>;
}
