use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Interrupt {
    Pause,
    Stop,
}

#[derive(Default)]
struct State {
    requested: Option<Interrupt>,
    finished: bool,
    paused: bool,
}

// 请求仅表明已收到；调用方必须等待原生队列与输入收尾后发布 Paused / Stopped。
#[derive(Clone, Default)]
pub struct PipelineControl {
    state: Arc<Mutex<State>>,
    interrupted: Arc<AtomicBool>,
}

// 发给 GUI / 其他线程的句柄只能请求，不能提前清除取消或确认队列已静止。
#[derive(Clone)]
pub struct PipelineRequest(PipelineControl);

impl PipelineRequest {
    pub fn pause(&self) -> bool {
        self.0.pause()
    }
    pub fn stop(&self) -> bool {
        self.0.stop()
    }
}

impl PipelineControl {
    pub fn requester(&self) -> PipelineRequest {
        PipelineRequest(self.clone())
    }
    pub fn pause(&self) -> bool {
        self.request(Interrupt::Pause)
    }
    pub fn stop(&self) -> bool {
        self.request(Interrupt::Stop)
    }

    fn request(&self, requested: Interrupt) -> bool {
        let mut state = self.state.lock().expect("Pipeline 控制锁失效");
        if state.finished {
            return false;
        }
        if state.requested != Some(Interrupt::Stop) {
            state.requested = Some(requested);
        }
        self.interrupted.store(true, Ordering::Release);
        true
    }

    pub fn requested(&self) -> Option<Interrupt> {
        self.state.lock().expect("Pipeline 控制锁失效").requested
    }

    pub fn cancellation(&self) -> Arc<AtomicBool> {
        self.interrupted.clone()
    }

    pub fn resume(&self) -> bool {
        let mut state = self.state.lock().expect("Pipeline 控制锁失效");
        if state.finished || !state.paused || state.requested != Some(Interrupt::Pause) {
            return false;
        }
        state.requested = None;
        state.paused = false;
        self.interrupted.store(false, Ordering::Release);
        true
    }

    pub fn acknowledge_pause(&self) -> bool {
        let mut state = self.state.lock().expect("Pipeline 控制锁失效");
        if state.finished || state.requested != Some(Interrupt::Pause) {
            return false;
        }
        state.paused = true;
        true
    }
    /// 最终应用结果与暂停 / 停止共用控制锁；已有请求时保留继续处理的能力。
    pub fn finish_if_uninterrupted(&self) -> Result<(), Interrupt> {
        let mut state = self.state.lock().expect("Pipeline 控制锁失效");
        if let Some(requested) = state.requested {
            return Err(requested);
        }
        state.finished = true;
        Ok(())
    }

    // 与并发停止请求按同一把锁确定终态的先后。
    pub fn finish(&self) -> Option<Interrupt> {
        let mut state = self.state.lock().expect("Pipeline 控制锁失效");
        state.finished = true;
        state.requested
    }
}
