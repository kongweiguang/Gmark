// @author kongweiguang

use gmark_document_core::{DocumentRevision, RecoveryRecord};
use gmark_document_runtime::DocumentSaveSnapshot;
use gmark_paged_document::{ExternalChange, PagedDocumentError, SearchCancellation};
use gpui::{SharedString, Task};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::DocumentRecoveryJournal;
use super::SharedDocument;
use super::recovery_worker::RecoveryWorker;
use super::session::RetiredRecoveryJournal;

#[cfg(test)]
pub(crate) trait RecoveryDocument {
    fn checkpoint_recovery(
        &self,
        journal: &mut DocumentRecoveryJournal,
    ) -> Result<(), PagedDocumentError>;
}

#[cfg(test)]
impl RecoveryDocument for SharedDocument {
    fn checkpoint_recovery(
        &self,
        journal: &mut DocumentRecoveryJournal,
    ) -> Result<(), PagedDocumentError> {
        self.with_session(|session| journal.checkpoint(session))
            .map_err(|error| PagedDocumentError::InvalidTransaction(error.to_string()))
            .and_then(|result| result)
    }
}

#[cfg(test)]
impl RecoveryDocument for gmark_document_runtime::DocumentSession {
    fn checkpoint_recovery(
        &self,
        journal: &mut DocumentRecoveryJournal,
    ) -> Result<(), PagedDocumentError> {
        journal.checkpoint(self)
    }
}

/// 保留日志安装前唯一一个 Resident 恢复命令，避免首个编辑在后台创建日志
/// 的窗口内被静默丢弃，同时不为 Paged 的有序命令制造可合并的旁路队列。
pub(crate) struct PendingRecoveryRecord {
    pub(crate) snapshot: DocumentSaveSnapshot,
    pub(crate) record: RecoveryRecord,
}

/// 保存请求只保留目标和视图身份；真正执行时再捕获最新快照，避免旧 IO 期间的修改被遗漏。
pub(crate) struct PendingHostSave {
    pub(crate) document_epoch: u64,
    pub(crate) path: PathBuf,
    pub(crate) save_as: bool,
    pub(crate) window: gpui::AnyWindowHandle,
}

pub(crate) struct SaveCoordinator {
    pub(crate) generation: u64,
    pub(crate) cancellation: Option<SearchCancellation>,
    pub(crate) task: Task<()>,
    pub(crate) pending_requests: VecDeque<PendingHostSave>,
}

impl Default for SaveCoordinator {
    /// 保存执行与等待请求分开，关闭视图只清理自身请求，不制造后台共享快照。
    fn default() -> Self {
        Self {
            generation: 0,
            cancellation: None,
            task: Task::ready(()),
            pending_requests: VecDeque::new(),
        }
    }
}

pub(crate) struct AutoSaveCoordinator {
    pub(crate) generation: u64,
    pub(crate) revision: Option<DocumentRevision>,
    pub(crate) scheduled_revision: Option<DocumentRevision>,
    pub(crate) waiting_revision: Option<DocumentRevision>,
    pub(crate) attempted_revision: Option<DocumentRevision>,
    pub(crate) task: Option<Task<()>>,
}

impl Default for AutoSaveCoordinator {
    /// 每个视图只跟踪当前修订的一次期限，实际保存仍由共享 Controller 仲裁。
    fn default() -> Self {
        Self {
            generation: 0,
            revision: None,
            scheduled_revision: None,
            waiting_revision: None,
            attempted_revision: None,
            task: None,
        }
    }
}

impl AutoSaveCoordinator {
    /// 先使旧定时回调失效，再释放任务，避免候选或路径切换后提交旧期限。
    pub(crate) fn cancel_task(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.scheduled_revision = None;
        self.task = None;
    }

    /// 正文变 clean 或身份切换后清除调度身份；空状态的重复渲染不会推进无意义代次。
    pub(crate) fn reset(&mut self) {
        if self.revision.is_some() || self.task.is_some() || self.scheduled_revision.is_some() {
            self.cancel_task();
        }
        self.revision = None;
        self.waiting_revision = None;
        self.attempted_revision = None;
    }

    /// 只由正文修订重置期限，选择、滚动与无关重绘不会让用户的内容迟迟不能保存。
    pub(crate) fn prepare_revision(&mut self, revision: DocumentRevision) {
        if self.revision != Some(revision) {
            self.cancel_task();
            self.revision = Some(revision);
            self.waiting_revision = None;
            self.attempted_revision = None;
        }
    }
}

/// 恢复正文在明确保存前不得覆盖磁盘；门禁属于共享文档，不能由新窗格绕过。
#[derive(Default)]
pub(crate) struct RecoveredAutoSaveGate {
    explicit_save_required: AtomicBool,
}

impl RecoveredAutoSaveGate {
    /// 将门禁安装到 Controller 的共享扩展，所有已打开及随后创建的视图都受同一约束。
    pub(crate) fn require_explicit_save(document: &SharedDocument) {
        let handle = document.handle();
        let gate = match handle.shared_extension::<Self>() {
            Ok(Some(gate)) => Some(gate),
            Ok(None) => handle
                .install_shared_extension(Arc::new(Self::default()))
                .ok(),
            Err(_) => None,
        };
        if let Some(gate) = gate {
            gate.explicit_save_required.store(true, Ordering::Release);
        }
    }

    /// 共享扩展读取失败时继续阻止自动保存，不能因锁错误静默覆盖待确认的恢复正文。
    pub(crate) fn blocks_auto_save(document: &SharedDocument) -> bool {
        match document.handle().shared_extension::<Self>() {
            Ok(Some(gate)) => gate.explicit_save_required.load(Ordering::Acquire),
            Ok(None) => false,
            Err(_) => true,
        }
    }

    /// 只有显式快照被 Controller 接受后才解除门禁，失败保存仍保留恢复内容的确认边界。
    pub(crate) fn explicit_save_succeeded(document: &SharedDocument) {
        if let Ok(Some(gate)) = document.handle().shared_extension::<Self>() {
            gate.explicit_save_required.store(false, Ordering::Release);
        }
    }
}

/// 统一拥有文档后台任务、取消令牌和代次门禁。
///
/// Controller 可以发起任务，但只有这里的 generation 与 cancellation 决定结果能否安装。
pub(crate) struct DocumentCoordinator {
    /// 超长词/字素解析属于当前视图的选择请求，独立取消，不能阻塞视口读取或保存。
    pub(crate) source_boundary_generation: u64,
    pub(crate) source_boundary_cancellation: Option<SearchCancellation>,
    pub(crate) source_boundary_task: Task<()>,
    pub(crate) source_boundary_actions: VecDeque<super::source_ime::DeferredSourceAction>,
    pub(crate) source_boundary_input_owner: Option<gpui::Entity<crate::components::Block>>,
    /// 失效请求中的已确认文字保留到剪贴板接收成功，不能写进用户的新选区。
    pub(crate) source_boundary_recovery_text: Option<String>,
    pub(crate) source_boundary_completion:
        Option<super::source_boundary_requests::SourceBoundaryCompletion>,
    pub(crate) source_generation: u64,
    pub(crate) source_cancellation: Option<SearchCancellation>,
    pub(crate) search_generation: u64,
    pub(crate) search_cancellation: Option<SearchCancellation>,
    pub(crate) external_status: Option<SharedString>,
    pub(crate) pending_external_change: Option<ExternalChange>,
    pub(crate) external_monitor_paused: bool,
    pub(crate) external_generation: u64,
    pub(crate) index_generation: u64,
    pub(crate) index_cancellation: Option<SearchCancellation>,
    pub(crate) save: SaveCoordinator,
    pub(crate) auto_save: AutoSaveCoordinator,
    pub(crate) recovery_journal: Option<DocumentRecoveryJournal>,
    /// Distinguish an intentionally disabled journal from one still being
    /// created so an early edit can report degraded recovery rather than fake durability.
    pub(crate) recovery_enabled: bool,
    /// Journal ownership moves here before the first edit; the UI only keeps
    /// the bounded sender and never performs journal I/O itself.
    pub(crate) recovery_worker: Option<RecoveryWorker>,
    /// 空文件日志异步创建期间只允许保留最新 Resident 快照；Paged 命令必须保持顺序，
    /// 因而不会进入这个旁路槽位。
    pub(crate) pending_recovery_record: Option<PendingRecoveryRecord>,
    /// 已保存但暂未删除的旧日志单独排队；不能放回 active 槽，否则安装新日志时会丢失重试权。
    pub(crate) retired_recovery_journals: Vec<RetiredRecoveryJournal>,
    pub(crate) recovery_error: Option<SharedString>,
    /// Recovery setup and worker callbacks use this host generation so a
    /// closed/reloaded view cannot install a late journal result.
    pub(crate) recovery_generation: u64,
    pub(crate) lifetime_cancellation: SearchCancellation,
    pub(crate) index_task: Task<()>,
    pub(crate) source_task: Task<()>,
    pub(crate) search_task: Task<()>,
    pub(crate) external_task: Task<()>,
}

impl DocumentCoordinator {
    /// 各类后台任务保留独立代次与取消权，源码边界查找不能复用保存或视口任务的生命周期。
    pub(crate) fn new(lifetime_cancellation: SearchCancellation) -> Self {
        Self {
            source_boundary_generation: 0,
            source_boundary_cancellation: None,
            source_boundary_task: Task::ready(()),
            source_boundary_actions: VecDeque::new(),
            source_boundary_input_owner: None,
            source_boundary_recovery_text: None,
            source_boundary_completion: None,
            source_generation: 0,
            source_cancellation: None,
            search_generation: 0,
            search_cancellation: None,
            external_status: None,
            pending_external_change: None,
            external_monitor_paused: false,
            external_generation: 0,
            index_generation: 0,
            index_cancellation: None,
            save: SaveCoordinator::default(),
            auto_save: AutoSaveCoordinator::default(),
            recovery_journal: None,
            recovery_enabled: false,
            recovery_worker: None,
            pending_recovery_record: None,
            retired_recovery_journals: Vec::new(),
            recovery_error: None,
            recovery_generation: 0,
            lifetime_cancellation,
            index_task: Task::ready(()),
            source_task: Task::ready(()),
            search_task: Task::ready(()),
            external_task: Task::ready(()),
        }
    }

    /// 在日志尚未安装时暂存一个不可变 Resident 命令，保证异步创建窗口不丢首个编辑。
    /// 返回 `false` 表示 Paged 命令不能被合并，调用方必须报告明确的降级错误。
    pub(crate) fn stage_pending_recovery(
        &mut self,
        snapshot: DocumentSaveSnapshot,
        record: RecoveryRecord,
    ) -> bool {
        if snapshot.source_format.is_none() {
            return false;
        }
        let revision = snapshot.revision;
        if self
            .pending_recovery_record
            .as_ref()
            .is_none_or(|pending| pending.snapshot.revision <= revision)
        {
            self.pending_recovery_record = Some(PendingRecoveryRecord { snapshot, record });
        }
        true
    }

    /// 只转移 pending 的所有权，让 worker 在安装成功后立即提交而不复制正文。
    pub(crate) fn take_pending_recovery(&mut self) -> Option<PendingRecoveryRecord> {
        self.pending_recovery_record.take()
    }

    /// 将 worker 尚未接受的命令放回有界槽位，以便调用方保留可重试状态和错误证据。
    pub(crate) fn restore_pending_recovery(&mut self, pending: PendingRecoveryRecord) {
        let _ = self.stage_pending_recovery(pending.snapshot, pending.record);
    }

    /// 丢弃已被保存或明确 discard 的旧 pending，避免日志稍后安装时重放已不再脏的正文。
    pub(crate) fn clear_pending_recovery_through(&mut self, revision: DocumentRevision) {
        if self
            .pending_recovery_record
            .as_ref()
            .is_some_and(|pending| pending.snapshot.revision <= revision)
        {
            self.pending_recovery_record = None;
        }
    }

    /// 释放视图任务时保留保存终态回调，让已捕获的共享快照能完成或失败，而不会堵塞其它视图。
    pub(crate) fn cancel_all(&mut self) {
        self.recovery_generation = self.recovery_generation.wrapping_add(1);
        self.lifetime_cancellation.cancel();
        self.auto_save.reset();
        for cancellation in [
            self.source_boundary_cancellation.take(),
            self.source_cancellation.take(),
            self.search_cancellation.take(),
            self.index_cancellation.take(),
            self.save.cancellation.take(),
        ]
        .into_iter()
        .flatten()
        {
            cancellation.cancel();
        }
        self.source_task = Task::ready(());
        self.source_boundary_task = Task::ready(());
        self.source_boundary_completion = None;
        self.source_boundary_actions.clear();
        self.source_boundary_input_owner = None;
        self.search_task = Task::ready(());
        self.index_task = Task::ready(());
        self.external_task = Task::ready(());
        std::mem::replace(&mut self.save.task, Task::ready(())).detach();
        self.save.pending_requests.clear();
        // Do not clear `pending_recovery_record`: a journal-creation failure
        // must leave the latest immutable Resident evidence available for a
        // later install/retry rather than turning close into silent loss.
    }

    /// Keep the old failure-injection contract available to unit tests; the
    /// production reload path uses the recovery worker and never calls this
    /// synchronous compatibility hook.
    #[cfg(test)]
    pub(crate) fn replace_recovery_journal_after_persistence(
        &mut self,
        replacement: Option<DocumentRecoveryJournal>,
        document: &impl RecoveryDocument,
    ) -> Result<(), PagedDocumentError> {
        let retry_error = self.retry_retired_recovery_journals().err();
        let previous = std::mem::replace(&mut self.recovery_journal, replacement);
        let checkpoint_error = previous.and_then(|mut journal| {
            let result = document.checkpoint_recovery(&mut journal);
            match result {
                Ok(()) => None,
                Err(error) => {
                    let (retired, retirement_error) = journal.retire_for_cleanup();
                    self.retired_recovery_journals.push(retired);
                    Some(retirement_error.unwrap_or(error))
                }
            }
        });

        match (retry_error, checkpoint_error) {
            (Some(error), _) | (None, Some(error)) => Err(error),
            (None, None) => Ok(()),
        }
    }

    /// Keeps every failed removal in the queue so one bad path cannot discard later work.
    pub(crate) fn retry_retired_recovery_journals(&mut self) -> Result<(), PagedDocumentError> {
        let mut pending = Vec::with_capacity(self.retired_recovery_journals.len());
        let mut first_error = None;
        for journal in std::mem::take(&mut self.retired_recovery_journals) {
            match journal.retry_cleanup() {
                Ok(()) => {}
                Err(error) => {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                    pending.push(journal);
                }
            }
        }
        self.retired_recovery_journals = pending;
        first_error.map_or(Ok(()), Err)
    }
}
