import type { LostVoidAppProgress } from './generated/protocol';

const phases: Record<LostVoidAppProgress['phase'], string> = {
  plan: '核对日周进度', challenge: '挑战中', commit_challenge: '保存结算', hub: '返回入口',
  bounty: '核对并领取悬赏', commit_claim: '保存领取记录', done: '流程结束',
};
const reasons = { run: '需要继续挑战', daily_limit: '达到每日上限', weekly_complete: '周目标完成' };

export function LostVoidStatus({ progress }: { progress: LostVoidAppProgress }) {
  const result = progress.end === 'timed_out' ? '超时'
    : progress.completion?.kind === 'challenge_failed' ? '本次挑战未完成'
    : progress.completion?.kind === 'skipped' ? `无需挑战：${reasons[progress.completion.decision]}`
    : progress.completion?.kind === 'completed' ? reasons[progress.completion.decision]
    : phases[progress.phase];
  const planning = progress.planning;
  return <section aria-label="迷失之地运行进度">
    <p>迷失之地 · {result} · 已核实通关 {progress.successful_challenges} 次 · 已完成领取核对 {progress.completed_claim_checks} 次</p>
    {planning && <p>游戏日 {planning.progress.day.date} · 当日已通关 {planning.progress.daily_run_times} 次 · 本周已通关 {planning.progress.weekly_run_times} 次 · 待领取结算 {planning.pending_claims.length} 份</p>}
    <p>{progress.pending_settlement != null ? '结算记录待补交' : `已保存结算 ${progress.committed_settlements.length} 份`} · {progress.pending_claim != null ? '领取记录待补交' : `已保存领取记录 ${progress.committed_claims.length} 份`}</p>
    <p>{progress.returned_world ? '已确认返回大世界' : progress.completion?.kind === 'skipped' ? '本次无需进入挑战' : '尚未确认返回大世界'} · {progress.inputs_released ? '迷失之地输入已释放' : '迷失之地输入待释放'} · 有效耗时 {progress.active_ms} ms</p>
  </section>;
}
