use od_platform::InputRecoveryPlan;
use od_protocol::{ApiError, Payload};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub fn recovery_error(error: impl std::fmt::Display) -> ApiError {
    ApiError::new(
        "input_recovery_required",
        format!(
            "{error}。请停止使用自动化，点击“恢复遗留输入”；失败时手动按下并松开对应按键/鼠标键，手柄异常请关闭游戏并检查虚拟设备，必要时重启 Windows。恢复前不能重连或运行任务"
        ),
    )
}

pub struct RecoveryJournal {
    path: PathBuf,
    pub plan: Option<InputRecoveryPlan>,
}

impl RecoveryJournal {
    pub fn open(data: &Path) -> Result<Self, ApiError> {
        let path = data.join("input-recovery.json");
        let plan = match fs::File::open(&path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(64 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(recovery_error)?;
                if bytes.len() > 64 * 1024 {
                    return Err(recovery_error("输入恢复记录超过大小限制"));
                }
                let plan: InputRecoveryPlan =
                    serde_json::from_slice(&bytes).map_err(recovery_error)?;
                plan.validate().map_err(recovery_error)?;
                Some(plan)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(recovery_error(error)),
        };
        Ok(Self { path, plan })
    }

    pub fn require_clean(&self) -> Result<(), ApiError> {
        if self.plan.is_some() {
            Err(recovery_error("尚未确认上次输入已释放"))
        } else {
            Ok(())
        }
    }

    pub fn require_mail_run(&self, run_id: uuid::Uuid) -> Result<(), ApiError> {
        self.require_pipeline_run(run_id, 2)
    }

    pub fn require_pipeline_run(&self, run_id: uuid::Uuid, version: u32) -> Result<(), ApiError> {
        let expected = self
            .plan
            .as_ref()
            .filter(|plan| {
                plan.version == version
                    && plan.run_id.as_deref() == Some(run_id.to_string().as_str())
            })
            .ok_or_else(|| recovery_error("流程恢复登记与当前运行不符"))?;
        let disk = Self::open(self.path.parent().expect("恢复记录有父目录"))?;
        let actual = disk
            .plan
            .as_ref()
            .ok_or_else(|| recovery_error("流程恢复登记已丢失"))?;
        if serde_json::to_value(expected).map_err(recovery_error)?
            != serde_json::to_value(actual).map_err(recovery_error)?
        {
            return Err(recovery_error("流程恢复登记已改变"));
        }
        Ok(())
    }

    pub fn arm(&mut self, plan: InputRecoveryPlan) -> Result<(), ApiError> {
        self.require_clean()?;
        plan.validate().map_err(recovery_error)?;
        // 在写盘前登记；即使磁盘错误，也不能让本会话继续提交输入。
        self.plan = Some(plan);
        let parent = self.path.parent().expect("恢复记录有父目录");
        fs::create_dir_all(parent).map_err(recovery_error)?;
        // 首次登记不可覆盖其他桌面实例刚写入的记录。
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&self.path)
            .map_err(recovery_error)?;
        file.write_all(
            &serde_json::to_vec(self.plan.as_ref().expect("已有恢复计划"))
                .map_err(recovery_error)?,
        )
        .map_err(recovery_error)?;
        file.sync_all().map_err(recovery_error)
    }

    fn save(&self) -> Result<(), ApiError> {
        let parent = self.path.parent().expect("恢复记录有父目录");
        fs::create_dir_all(parent).map_err(recovery_error)?;
        let temp = parent.join(format!("input-recovery-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<(), ApiError> {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)
                .map_err(recovery_error)?;
            file.write_all(
                &serde_json::to_vec(&self.plan.as_ref().expect("已有恢复计划"))
                    .map_err(recovery_error)?,
            )
            .map_err(recovery_error)?;
            file.sync_all().map_err(recovery_error)?;
            drop(file);
            fs::rename(&temp, &self.path).map_err(recovery_error)
        })();
        if result.is_err() {
            let _ = fs::remove_file(temp);
        }
        result
    }

    pub fn accepted(&mut self, payload: &Payload) -> Result<(), ApiError> {
        if let (Some(plan), Payload::Run(run)) = (&mut self.plan, payload) {
            plan.run_id = Some(run.id.to_string());
            self.save()?;
        }
        Ok(())
    }

    pub fn observe(&mut self, payload: &Payload) -> Result<(), ApiError> {
        let run = match payload {
            Payload::Snapshot(s) => s.run.as_ref(),
            Payload::Run(r) => Some(r),
            _ => None,
        };
        if let (Some(plan), Some(run)) = (&self.plan, run)
            && plan.run_id.as_deref() == Some(run.id.to_string().as_str())
            && run.state.is_terminal()
            && (if plan.version==8 {
                run.combat.as_ref().is_some_and(|p|p.inputs_released && p.state==run.state)
                    && run.matches_account_context() && run.inputs_released()
            } else if matches!(plan.version, 6 | 7) {
                run.group
                    .as_ref()
                    .is_some_and(|group| group.state == run.state && group.current_index.is_none())
                    && run.matches_account_context()
                    && run.inputs_released()
            } else if plan.version == 2 {
                run.app_id == "email"
                    && run
                        .mail
                        .as_ref()
                        .is_some_and(|result| result.inputs_released)
            } else if matches!(plan.version, 4 | 5) {
                run.app_id == "prototype.coffee"
                    && run.coffee.as_ref().is_some_and(|result| {
                        result.inputs_released && result.approach.is_some() == (plan.version == 5)
                    })
            } else if plan.version == 3 {
                run.app_id == "prototype.map_transport"
                    && run
                        .map
                        .as_ref()
                        .is_some_and(|result| result.inputs_released)
            } else {
                run.input_probe
                    .as_ref()
                    .is_some_and(|result| result.released)
                    || run
                        .input_pipeline
                        .as_ref()
                        .is_some_and(|result| result.pipeline.released)
            })
        {
            self.clear()?;
        }
        Ok(())
    }

    // 仅在确认释放、确认请求被拒绝或独立恢复成功后调用。
    pub fn clear(&mut self) -> Result<(), ApiError> {
        match fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(recovery_error(error)),
        }
        self.plan = None;
        Ok(())
    }
}
