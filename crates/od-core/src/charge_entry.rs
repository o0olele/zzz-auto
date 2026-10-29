//! 试运行入口只开放已有完整执行器；内部选关开发入口继续独立。
use crate::charge_plan::ChargePlanConfig;
pub const TIMEOUT_MS: u32 = 86_400_000;
pub fn has_work(config: &ChargePlanConfig) -> bool {
    config.double_reward || config.plan_list.iter().any(|p| p.plan_times > 0)
}
pub fn needs_combat(config: &ChargePlanConfig) -> bool {
    config
        .plan_list
        .iter()
        .any(|p| p.plan_times > 0 && p.category_name != "合成电池")
}
pub fn validate(config: &ChargePlanConfig) -> Result<(), &'static str> {
    config.validate()?;
    if config.double_reward
        || config
            .plan_list
            .iter()
            .any(|p| p.plan_times > 0 && p.category_name == "实战模拟室")
    {
        return Err("模拟室生产选中证明尚未接通，请先禁用模拟室及双倍计划；本次不会输入");
    }
    if config
        .plan_list
        .iter()
        .any(|p| p.plan_times > 0 && p.category_name != "恶名狩猎" && p.level != "默认等级")
    {
        return Err("当前只有深度追猎支持指定等级，请先将其他启用计划设为默认等级");
    }
    Ok(())
}
