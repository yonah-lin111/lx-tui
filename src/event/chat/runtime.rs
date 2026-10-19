//! 会话进程运行实例：跨轮保留的进程管理器 + 本轮执行上下文。
//!
//! 归属（对应 design §5 / §8）：
//!
//! - 管理器按**聊天会话**持有，正常结束一轮模型回答**不清理**；下一轮复用同一实例与
//!   `session_id`；
//! - 切会话、`/new`、关窗格或退出 lx 时清理该会话的全部进程；
//! - 进程事件带 `session_id` 与发起调用的 `call_id`，旧运行代次的迟到事件被丢弃；
//! - 本轮新建集合与当前活动交互由 [`TurnExecution`] 维护，供 Esc 停止按 D9 取并集。

use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::Mutex;

use crate::chat::agent_loop::TurnExecution;
use crate::chat::process::{ProcessManager, ProcessOwner, ProcessUiEvent};

/// 一个聊天会话的运行实例。
pub struct SessionRuntime {
    /// 会话进程管理器（跨轮复用）。
    pub manager: Arc<ProcessManager>,
    /// 运行代次（进程归属键的一部分；同一会话重开后变化）。
    pub epoch: String,
    /// 每轮的执行上下文（按 request key）。
    turns: HashMap<String, Arc<Mutex<TurnExecution>>>,
}

impl SessionRuntime {
    /// 为会话建立新的运行实例（代次由 session + 序号生成）。
    pub fn new(
        workspace: std::path::PathBuf,
        session_id: &str,
        epoch: String,
        ui: tokio::sync::mpsc::UnboundedSender<ProcessUiEvent>,
    ) -> Self {
        let owner = ProcessOwner {
            workspace,
            chat_session_id: session_id.to_string(),
            runtime_epoch: epoch.clone(),
        };
        Self {
            manager: Arc::new(ProcessManager::new(owner, Some(ui))),
            epoch,
            turns: HashMap::new(),
        }
    }

    /// 取（必要时新建）某轮的执行上下文。
    pub fn turn(&mut self, key: &str) -> Arc<Mutex<TurnExecution>> {
        self.turns
            .entry(key.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(TurnExecution::default())))
            .clone()
    }

    /// 只读地取某轮的执行上下文。
    pub fn turn_context(&self, key: &str) -> Option<Arc<Mutex<TurnExecution>>> {
        self.turns.get(key).cloned()
    }

    /// 一轮结束：丢弃该轮上下文（进程仍留在管理器里继续跑）。
    pub fn finish_turn(&mut self, key: &str) {
        self.turns.remove(key);
    }

    /// 进程管理器（清理/查询用）。
    pub fn manager(&self) -> &std::sync::Arc<ProcessManager> {
        &self.manager
    }
}

/// 全部会话运行实例（事件循环持有唯一一份）。
#[derive(Default)]
pub struct ProcessRuntimes {
    runtimes: HashMap<String, SessionRuntime>,
    /// 代次计数器（同一会话重开时递增，旧代次事件被丢弃）。
    epochs: HashMap<String, u64>,
}

impl ProcessRuntimes {
    /// 取（必要时为该会话新建）运行实例。
    pub fn get_or_create(
        &mut self,
        workspace: std::path::PathBuf,
        session_id: &str,
        ui: tokio::sync::mpsc::UnboundedSender<ProcessUiEvent>,
    ) -> &mut SessionRuntime {
        if !self.runtimes.contains_key(session_id) {
            let counter = self.epochs.entry(session_id.to_string()).or_insert(0);
            *counter = counter.saturating_add(1);
            let epoch = format!("{session_id}:{}", *counter);
            let runtime = SessionRuntime::new(workspace, session_id, epoch, ui);
            self.runtimes.insert(session_id.to_string(), runtime);
        }
        self.runtimes.get_mut(session_id).expect("inserted above")
    }

    /// 该会话已有运行实例时返回它（不新建）。
    pub fn get(&self, session_id: &str) -> Option<&SessionRuntime> {
        self.runtimes.get(session_id)
    }

    /// 丢弃某轮的执行上下文（进程仍留在管理器里继续跑）。
    pub fn finish_turn(&mut self, session_id: &str, turn: &str) {
        if let Some(runtime) = self.runtimes.get_mut(session_id) {
            runtime.finish_turn(turn);
        }
    }

    /// 移除并返回会话运行实例（调用方负责停止其全部进程）。
    pub fn take(&mut self, session_id: &str) -> Option<SessionRuntime> {
        self.runtimes.remove(session_id)
    }

    /// 当前持有的会话 ID 列表（清理孤儿实例用）。
    pub fn sessions(&self) -> Vec<String> {
        self.runtimes.keys().cloned().collect()
    }
}
