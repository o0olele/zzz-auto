//! 锄大地资源入口；输入由当前截图决定，不使用固定坐标盲点传送点。
pub fn pipeline() -> serde_json::Value {
    serde_json::json!({"Patrol.Ready":{"recognition":"DirectHit","action":"DoNothing"}})
}
