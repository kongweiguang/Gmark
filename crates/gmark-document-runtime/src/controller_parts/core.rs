// @author kongweiguang

//! Controller 的共享状态、保存队列和外部状态迁移。

use super::super::*;

impl SaveQueue {
    /// 将新的不可变快照放入唯一的串行保存通道，避免后台保存读取到后续 revision。
    fn request(&mut self, snapshot: DocumentSaveSnapshot) -> Option<DocumentSaveSnapshot> {
        match self.in_flight.as_ref() {
            Some(in_flight) if in_flight.revision == snapshot.revision => None,
            Some(_) => {
                self.pending = Some(snapshot);
                None
            }
            None => {
                self.in_flight = Some(snapshot.clone());
                Some(snapshot)
            }
        }
    }

    /// 只有当前在途快照完成后才提升 pending，保证错误的完成通知不会改变队列状态。
    fn complete(
        &mut self,
        revision: DocumentRevision,
    ) -> Result<Option<DocumentSaveSnapshot>, ControllerError> {
        if self.in_flight.as_ref().map(|snapshot| snapshot.revision) != Some(revision) {
            return Err(ControllerError::UnexpectedSaveCompletion {
                expected: self.in_flight.as_ref().map(|snapshot| snapshot.revision),
                actual: revision,
            });
        }
        self.in_flight = None;
        let next = self
            .pending
            .take()
            .filter(|snapshot| snapshot.revision != revision);
        if let Some(next) = next.clone() {
            self.in_flight = Some(next);
        }
        Ok(next)
    }

    /// 失败只结束当前在途快照；pending 必须由调用方显式重试以避免隐式写盘。
    fn fail(&mut self, revision: DocumentRevision) -> Result<(), ControllerError> {
        if self.in_flight.as_ref().map(|snapshot| snapshot.revision) != Some(revision) {
            return Err(ControllerError::UnexpectedSaveCompletion {
                expected: self.in_flight.as_ref().map(|snapshot| snapshot.revision),
                actual: revision,
            });
        }
        self.in_flight = None;
        Ok(())
    }
}

impl DocumentController {
    /// 创建共享 Controller，并把所有跨视图状态初始化为同一正文基线。
    pub fn new(document_id: DocumentId, session: DocumentSession) -> Self {
        Self {
            document_id,
            session,
            save_queue: SaveQueue::default(),
            events: VecDeque::new(),
            event_sequence: 0,
            views: BTreeMap::new(),
            undo_transactions: Vec::new(),
            redo_transactions: Vec::new(),
            active_typing_group: None,
            next_transaction_id: 1,
        }
    }

    pub fn document_id(&self) -> DocumentId {
        self.document_id
    }

    pub fn session(&self) -> &DocumentSession {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut DocumentSession {
        &mut self.session
    }

    pub fn save_in_flight_revision(&self) -> Option<DocumentRevision> {
        self.save_queue
            .in_flight
            .as_ref()
            .map(|snapshot| snapshot.revision)
    }

    pub fn save_pending_revision(&self) -> Option<DocumentRevision> {
        self.save_queue
            .pending
            .as_ref()
            .map(|snapshot| snapshot.revision)
    }

    /// 捕获同一把 Controller 锁内的正文和保存元数据，使慢速 IO 不依赖可变会话。
    pub fn save_snapshot(&self) -> DocumentSaveSnapshot {
        self.session.save_snapshot()
    }

    /// 请求保存最新 revision，并在队列空闲时把快照交给调用方执行 IO。
    pub fn request_save_snapshot(
        &mut self,
    ) -> Result<Option<DocumentSaveSnapshot>, ControllerError> {
        Ok(self.save_queue.request(self.session.save_snapshot()))
    }

    /// 提交保存结果并更新 dirty/identity 事件；返回已排队的下一份快照供调用方继续写入。
    pub fn complete_save(
        &mut self,
        revision: DocumentRevision,
        identity: FileIdentity,
    ) -> Result<Option<DocumentSaveSnapshot>, ControllerError> {
        let snapshot = self
            .save_queue
            .in_flight
            .as_ref()
            .filter(|snapshot| snapshot.revision == revision)
            .cloned()
            .ok_or_else(|| ControllerError::UnexpectedSaveCompletion {
                expected: self.save_in_flight_revision(),
                actual: revision,
            })?;
        let before_identity = self.session.file_identity.clone();
        let before_dirty = self.session.dirty;
        let promoted = self.save_queue.complete(revision)?;
        self.session
            .mark_saved_snapshot(&snapshot, identity.clone());
        self.emit(DocumentEvent::Saved {
            sequence: 0,
            document_id: self.document_id,
            revision,
            dirty: self.session.dirty,
            identity: identity.clone(),
        });
        if before_identity != identity {
            self.emit(DocumentEvent::IdentityChanged {
                sequence: 0,
                document_id: self.document_id,
                revision: DocumentRevision(self.session.revision()),
                identity,
            });
        }
        if before_dirty != self.session.dirty {
            self.emit(DocumentEvent::DirtyChanged {
                sequence: 0,
                document_id: self.document_id,
                revision: DocumentRevision(self.session.revision()),
                dirty: self.session.dirty,
            });
        }
        Ok(promoted)
    }

    /// 结束失败保存并清除在途状态；错误码只由上层状态机消费，不伪造成功事件。
    pub fn fail_save(
        &mut self,
        revision: DocumentRevision,
        _code: SaveFailureCode,
    ) -> Result<Option<DocumentSaveSnapshot>, ControllerError> {
        self.save_queue.fail(revision)?;
        Ok(None)
    }

    /// 在锁内验证外部观察基线，防止无锁 IO 结果覆盖本地新编辑。
    pub fn validate_external_transition(
        &mut self,
        expected_revision: DocumentRevision,
        expected_identity: &FileIdentity,
    ) -> Result<(), ControllerError> {
        self.validate_external_reload_baseline(expected_revision, expected_identity)?;
        if self.session.dirty {
            self.emit_external_conflict(self.session.file_identity.clone());
            return Err(ControllerError::DocumentDirty);
        }
        Ok(())
    }

    /// 重载只绕过 dirty 检查；revision 与身份仍必须和 IO 前观察值一致。
    fn validate_external_reload_baseline(
        &mut self,
        expected_revision: DocumentRevision,
        expected_identity: &FileIdentity,
    ) -> Result<(), ControllerError> {
        if DocumentRevision(self.session.revision()) != expected_revision {
            self.emit_external_conflict(self.session.file_identity.clone());
            return Err(ControllerError::ExternalRevisionMismatch {
                expected: expected_revision,
                actual: DocumentRevision(self.session.revision()),
            });
        }
        if self.session.file_identity != *expected_identity {
            self.emit_external_conflict(self.session.file_identity.clone());
            return Err(ControllerError::ExternalIdentityMismatch {
                expected: expected_identity.clone(),
                actual: self.session.file_identity.clone(),
            });
        }
        Ok(())
    }

    /// 把已经由 watcher 准备好的 append 原子提交为一个新的干净 revision。
    pub fn accept_external_append(
        &mut self,
        expected_revision: DocumentRevision,
        expected_identity: FileIdentity,
        source: FileSource,
        index: LineIndex,
        identity: FileIdentity,
    ) -> Result<(), ControllerError> {
        self.break_typing_group();
        self.validate_external_transition(expected_revision, &expected_identity)?;
        let before_identity = self.session.file_identity.clone();
        let before_dirty = self.session.dirty;
        self.session
            .accept_external_append(source, index)
            .map_err(|error| ControllerError::Mutation(error.to_string()))?;
        // Paged installation advances its backend generation while the resident
        // path starts from zero; use the validated baseline so both advance once.
        let revision = super::super::next_revision(expected_revision)?;
        self.session.set_revision(revision);
        self.session.file_identity = identity.clone();
        self.session.dirty = false;
        self.session.refresh_resident_source_state();
        self.emit_external_revision(revision);
        if before_identity != identity {
            self.emit(DocumentEvent::IdentityChanged {
                sequence: 0,
                document_id: self.document_id,
                revision,
                identity,
            });
        }
        if before_dirty != self.session.dirty {
            self.emit(DocumentEvent::DirtyChanged {
                sequence: 0,
                document_id: self.document_id,
                revision,
                dirty: self.session.dirty,
            });
        }
        Ok(())
    }

    /// 安装已解析的新会话并清理旧视图的 undo/redo 关系，避免跨正文恢复历史。
    pub fn reload_prepared_document(
        &mut self,
        expected_revision: DocumentRevision,
        expected_identity: FileIdentity,
        prepared: DocumentSession,
    ) -> Result<(), ControllerError> {
        self.break_typing_group();
        self.validate_external_transition(expected_revision, &expected_identity)?;
        self.install_prepared_document(prepared)
    }

    /// 仅供显式确认丢弃本地修改后的重载使用，并继续拒绝读取期间发生的并发变化。
    pub fn reload_prepared_document_after_confirmation(
        &mut self,
        expected_revision: DocumentRevision,
        expected_identity: FileIdentity,
        prepared: DocumentSession,
    ) -> Result<(), ControllerError> {
        self.break_typing_group();
        self.validate_external_reload_baseline(expected_revision, &expected_identity)?;
        self.install_prepared_document(prepared)
    }

    /// 统一安装 prepared session，确保所有 view 的选择、历史和事件一起切换。
    fn install_prepared_document(
        &mut self,
        prepared: DocumentSession,
    ) -> Result<(), ControllerError> {
        self.break_typing_group();
        let before_identity = self.session.file_identity.clone();
        let before_dirty = self.session.dirty;
        let revision = super::super::next_revision(DocumentRevision(self.session.revision()))?;
        self.session.replace_prepared(prepared, revision);
        self.undo_transactions.clear();
        self.redo_transactions.clear();
        for view in self.views.values_mut() {
            view.selection = SourceSelection::default();
        }
        let identity = self.session.file_identity.clone();
        self.emit_external_revision(revision);
        if before_identity != identity {
            self.emit(DocumentEvent::IdentityChanged {
                sequence: 0,
                document_id: self.document_id,
                revision,
                identity,
            });
        }
        if before_dirty != self.session.dirty {
            self.emit(DocumentEvent::DirtyChanged {
                sequence: 0,
                document_id: self.document_id,
                revision,
                dirty: self.session.dirty,
            });
        }
        Ok(())
    }

    /// 修改共享编码元数据但不制造正文 undo 项，保持保存 revision 与事件可追踪。
    pub fn set_encoding(
        &mut self,
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
        encoding: TextEncoding,
    ) -> Result<DocumentRevision, ControllerError> {
        self.break_typing_group();
        self.register_view(view_id);
        let before_dirty = self.session.dirty;
        if self.session.set_encoding(encoding)? {
            let revision = DocumentRevision(self.session.revision());
            self.emit(DocumentEvent::RevisionChanged {
                sequence: 0,
                document_id: self.document_id,
                view_id,
                transaction_id,
                revision,
                dirty: self.session.dirty,
                mutation: DocumentMutationMap::empty(),
                selection: self.view_selection(view_id).unwrap_or_default(),
            });
            if before_dirty != self.session.dirty {
                self.emit(DocumentEvent::DirtyChanged {
                    sequence: 0,
                    document_id: self.document_id,
                    revision,
                    dirty: self.session.dirty,
                });
            }
        }
        Ok(DocumentRevision(self.session.revision()))
    }

    /// 仅在 revision 仍是调用方观察值时确认 discard，避免清掉另一视图刚产生的编辑。
    pub fn discard_changes(
        &mut self,
        expected_revision: DocumentRevision,
    ) -> Result<bool, ControllerError> {
        self.break_typing_group();
        let current = DocumentRevision(self.session.revision());
        if current != expected_revision {
            return Err(ControllerError::ExternalRevisionMismatch {
                expected: expected_revision,
                actual: current,
            });
        }
        let changed = self.session.discard_changes();
        if changed {
            self.emit(DocumentEvent::DirtyChanged {
                sequence: 0,
                document_id: self.document_id,
                revision: current,
                dirty: false,
            });
        }
        Ok(changed)
    }

    pub fn resident_snapshot(&self) -> Option<gmark_document::DocumentSnapshot> {
        self.session.resident_snapshot()
    }

    pub fn source_format_snapshot(&self) -> Option<SourceFormatSnapshot> {
        self.session.source_format_snapshot()
    }

    pub fn view_selection(&self, view_id: DocumentViewInstanceId) -> Option<SourceSelection> {
        self.views.get(&view_id).map(|view| view.selection)
    }

    /// 注册视图状态，使 selection 与共享正文生命周期解耦。
    pub fn register_view(&mut self, view_id: DocumentViewInstanceId) {
        self.views.entry(view_id).or_insert(ViewRuntimeState {
            selection: SourceSelection::default(),
        });
    }

    /// 关闭输入组所属 view 时清除合并资格，再释放该视图状态。
    pub fn close_view(&mut self, view_id: DocumentViewInstanceId) {
        if self
            .active_typing_group
            .as_ref()
            .is_some_and(|group| group.view_id == view_id)
        {
            self.break_typing_group();
        }
        self.views.remove(&view_id);
    }

    /// 只有真实 selection 变化才断开输入组，避免无效同步信号切碎撤销历史。
    pub fn set_view_selection(
        &mut self,
        view_id: DocumentViewInstanceId,
        selection: SourceSelection,
    ) {
        self.register_view(view_id);
        if self
            .views
            .get(&view_id)
            .is_some_and(|view| view.selection != selection)
        {
            self.break_typing_group();
        }
        if let Some(view) = self.views.get_mut(&view_id) {
            view.selection = selection;
        }
    }

    pub fn next_transaction_id(&mut self) -> TransactionId {
        let id = TransactionId(self.next_transaction_id);
        self.next_transaction_id = self.next_transaction_id.saturating_add(1);
        id
    }

    /// 从有限事件日志中取出事件；订阅者使用 sequence 游标而不是 drain 读取。
    pub fn drain_events(&mut self) -> impl Iterator<Item = DocumentEvent> + '_ {
        self.events.drain(..)
    }

    /// 为新事件分配唯一序号并限制日志长度，防止长期订阅者拖垮共享文档。
    pub(super) fn emit(&mut self, event: DocumentEvent) {
        self.event_sequence = self.event_sequence.saturating_add(1);
        self.events
            .push_back(super::super::with_sequence(event, self.event_sequence));
        while self.events.len() > MAX_EVENT_LOG {
            self.events.pop_front();
        }
    }

    /// 统一构造外部变更事件，避免外部 watcher 伪造某个 view 的事务身份。
    fn emit_external_revision(&mut self, revision: DocumentRevision) {
        self.emit(DocumentEvent::RevisionChanged {
            sequence: 0,
            document_id: self.document_id,
            view_id: DocumentViewInstanceId::from_uuid(Uuid::nil()),
            transaction_id: TransactionId(0),
            revision,
            dirty: self.session.dirty,
            mutation: DocumentMutationMap::empty(),
            selection: SourceSelection::default(),
        });
    }

    /// 外部基线失配也进入事件日志，让 UI 和恢复层能统一显示冲突。
    pub(super) fn emit_external_conflict(&mut self, identity: FileIdentity) {
        self.emit(DocumentEvent::ExternalConflict {
            sequence: 0,
            document_id: self.document_id,
            revision: DocumentRevision(self.session.revision()),
            identity,
        });
    }
}
