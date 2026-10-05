// @author kongweiguang

//! `DocumentSession` 的正文、读写和持久化操作。
//!
//! 将这组实现放在私有子模块，是为了让根模块保留公共数据类型和错误契约，
//! 同时避免单个源文件继续承载所有后端转发逻辑。

use super::*;

impl DocumentSession {
    /// 在锁内捕获一次完整保存输入，确保锁外 IO 不会观察到后续 revision。
    pub fn snapshot(&self) -> Arc<dyn DocumentSnapshot> {
        self.store.snapshot()
    }

    /// 在锁内一次性捕获保存所需的正文、revision、编码和身份；返回值可安全
    /// 交给锁外的慢速 IO。调用方不得在保存完成前把它重新解释为当前正文。
    pub fn save_snapshot(&self) -> DocumentSaveSnapshot {
        DocumentSaveSnapshot {
            revision: self.store.revision(),
            identity: self.file_identity.clone(),
            encoding: self.profile.encoding.clone(),
            source_format: self.source_format_snapshot(),
            paged_save_plan: match &self.store {
                DocumentStore::Paged(document) => document.prepared_save_plan(),
                DocumentStore::Resident(_) => None,
            },
            snapshot: self.store.snapshot(),
            resident_baseline: self
                .resident_source_document()
                .map(|document| document.snapshot()),
            written_paged_identity: Arc::new(Mutex::new(None)),
        }
    }

    /// 直接读取会话元数据，保持编码选择与共享 profile 的单一来源。
    pub fn encoding(&self) -> &TextEncoding {
        &self.profile.encoding
    }

    /// 只推进元数据 revision，避免把编码变更伪装成正文编辑并破坏 undo 历史。
    pub fn set_encoding(&mut self, encoding: TextEncoding) -> Result<bool, SessionEditError> {
        if self.profile.encoding == encoding {
            return Ok(false);
        }
        match &mut self.store {
            DocumentStore::Resident(document) => {
                document
                    .advance_revision()
                    .map_err(SessionEditError::ResidentDocument)?;
                document.set_encoding(encoding.clone());
            }
            DocumentStore::Paged(document) => {
                document
                    .advance_revision()
                    .map_err(|error| SessionEditError::Paged(error.to_string()))?;
                document.set_encoding(encoding.clone());
            }
        }
        self.profile.encoding = encoding;
        self.dirty = true;
        Ok(true)
    }

    /// 通过统一 store 入口应用事务，保证两个后端共享 revision 和 dirty 语义。
    pub fn apply_transaction(
        &mut self,
        transaction: &Transaction,
    ) -> Result<DocumentRevision, SessionEditError> {
        self.store.apply_transaction(transaction)?;
        if !transaction.edits.is_empty() {
            self.dirty = true;
        }
        self.refresh_resident_profile();
        Ok(self.store.revision())
    }

    /// 通过共享 store 应用 Controller 标记的输入组，并统一更新 dirty 与 resident 统计。
    pub fn apply_typing_transaction(
        &mut self,
        transaction: &Transaction,
        group_id: gmark_document_core::TypingGroupId,
    ) -> Result<DocumentRevision, SessionEditError> {
        self.store.apply_typing_transaction(transaction, group_id)?;
        if !transaction.edits.is_empty() {
            self.dirty = true;
        }
        self.refresh_resident_profile();
        Ok(self.store.revision())
    }

    /// 结束 Resident 与 Paged 的输入合并资格而不改动已提交的历史项。
    pub fn break_typing_group(&mut self) {
        self.store.break_typing_group();
    }

    /// 仅允许 resident 文档规范化行尾，避免 paged 后端丢失原始编码格式信息。
    pub fn normalize_line_endings(&mut self, ending: LineEnding) -> Result<bool, SessionEditError> {
        let Some(document) = self.resident_source_document_mut() else {
            return Err(SessionEditError::Resident(
                "line-ending normalization requires a resident source".to_owned(),
            ));
        };
        let changed = document
            .normalize_line_endings(ending)
            .map_err(SessionEditError::ResidentDocument)?
            .is_some();
        if changed {
            self.dirty = true;
            self.refresh_resident_profile();
        }
        Ok(changed)
    }

    /// 通过 resident 的源格式事务恢复行尾等格式，保持源格式快照与正文同步。
    pub fn restore_source_format(
        &mut self,
        format: SourceFormatSnapshot,
    ) -> Result<bool, SessionEditError> {
        let Some(document) = self.resident_source_document_mut() else {
            return Ok(false);
        };
        let changed = document
            .restore_source_format_transaction(format)
            .map_err(SessionEditError::ResidentDocument)?
            .is_some();
        if changed {
            self.dirty = true;
            self.refresh_resident_profile();
        }
        Ok(changed)
    }

    /// 只有保存快照仍对应当前 revision 时才确认 dirty 基线，避免旧保存覆盖新编辑。
    pub fn mark_saved_snapshot(&mut self, snapshot: &DocumentSaveSnapshot, identity: FileIdentity) {
        if let (DocumentStore::Resident(document), Some(baseline)) =
            (&mut self.store, snapshot.resident_baseline.clone())
        {
            document.mark_persisted_snapshot(baseline);
        }
        if let DocumentStore::Paged(document) = &mut self.store {
            let paged_identity = snapshot
                .written_paged_identity()
                .unwrap_or_else(|| paged_identity_for_save(&identity));
            document.mark_prepared_saved(paged_identity);
        }
        let current_revision = self.store.revision();
        self.file_identity = identity;
        self.persisted_encoding = snapshot.encoding.clone();
        if current_revision == snapshot.revision {
            // Paged snapshots are immutable PieceTree views.  A successful
            // completion for the current revision therefore acknowledges the
            // current tree as its persisted baseline; an older completion must
            // leave newer edits dirty and must not overwrite that baseline.
            if let DocumentStore::Paged(document) = &mut self.store {
                document.mark_current_pristine();
            }
            self.dirty = self.profile.encoding != self.persisted_encoding || !self.is_pristine();
        } else {
            self.dirty = true;
        }
    }

    /// 确认丢弃当前修改时只更新持久化基线，保留 revision、undo 历史和正文不变。
    pub fn discard_changes(&mut self) -> bool {
        if !self.dirty {
            return false;
        }
        match &mut self.store {
            DocumentStore::Resident(document) => document.mark_persisted(),
            DocumentStore::Paged(document) => document.mark_current_pristine(),
        }
        self.persisted_encoding = self.profile.encoding.clone();
        self.dirty = false;
        true
    }

    /// 让控制器在替换准备好的会话时保留已分配的 revision。
    pub(crate) fn set_revision(&mut self, revision: DocumentRevision) {
        match &mut self.store {
            DocumentStore::Resident(document) => document.set_revision(revision),
            DocumentStore::Paged(document) => document.set_revision(revision.0),
        }
    }

    /// 用已准备的会话替换当前会话，同时把控制器 revision 写回新 store。
    pub(crate) fn replace_prepared(
        &mut self,
        mut prepared: DocumentSession,
        revision: DocumentRevision,
    ) {
        prepared.set_revision(revision);
        *self = prepared;
    }

    /// 返回共享 store 的 revision，避免调用方依赖具体后端的 revision 类型。
    pub fn revision(&self) -> u64 {
        self.store.revision().0
    }

    /// 返回逻辑正文长度，统一 resident 与 paged 的长度口径。
    pub fn len(&self) -> u64 {
        match &self.store {
            DocumentStore::Resident(document) => document.len(),
            DocumentStore::Paged(document) => document.len(),
        }
    }

    /// 用后端的空判断减少调用方对存储实现的分支。
    pub fn is_empty(&self) -> bool {
        match &self.store {
            DocumentStore::Resident(document) => document.is_empty(),
            DocumentStore::Paged(document) => document.is_empty(),
        }
    }

    /// 统一暴露持久化基线状态，供保存协调器判断是否仍可清除 dirty。
    pub fn is_pristine(&self) -> bool {
        match &self.store {
            DocumentStore::Resident(document) => document.is_pristine(),
            DocumentStore::Paged(document) => document.is_pristine(),
        }
    }

    /// 返回正文行数，保持两个后端在编辑器统计中的一致性。
    pub fn line_count(&self) -> u64 {
        match &self.store {
            DocumentStore::Resident(document) => document.line_count(),
            DocumentStore::Paged(document) => document.line_count(),
        }
    }

    /// 返回首次触发的 resident 增长原因，避免 undo 过程抹掉会话级提示。
    pub fn resident_growth_reason(&self) -> Option<OpenReason> {
        self.resident_growth_reason
    }

    /// 编辑后刷新 profile 的派生统计，并锁定本次会话的首次超限原因。
    pub(super) fn refresh_resident_profile(&mut self) {
        let DocumentStore::Resident(document) = &self.store else {
            return;
        };
        self.profile.len = document.len();
        self.profile.estimated_lines = document.line_count();
        self.profile.estimated_structural_units = document.structural_units();
        if self.resident_growth_reason.is_none() {
            self.resident_growth_reason = self.loading_limits.exceeded_reason(&self.profile);
        }
    }

    /// 返回指定行的字节范围，保持编辑器定位逻辑与后端索引一致。
    pub fn line_range(&self, line: u64) -> Option<Range<u64>> {
        match &self.store {
            DocumentStore::Resident(document) => document.line_range(line),
            DocumentStore::Paged(document) => document.line_range(line),
        }
    }

    /// 将字节偏移转换成行号，统一 resident 与 paged 的边界行为。
    pub fn line_for_offset(&self, offset: u64) -> Option<u64> {
        match &self.store {
            DocumentStore::Resident(document) => document.line_for_offset(offset),
            DocumentStore::Paged(document) => document.line_for_offset(offset),
        }
    }

    /// 仅 paged 后端提供可复用行索引，resident 由其内存文档直接计算。
    pub fn line_index(&self) -> Option<LineIndex> {
        match &self.store {
            DocumentStore::Resident(_) => None,
            DocumentStore::Paged(document) => Some(document.line_index()),
        }
    }

    /// 从权威 store 读取范围，避免调用方复制后端分支。
    pub fn read_range(&self, range: Range<u64>) -> Result<Vec<u8>, PagedDocumentError> {
        match &self.store {
            DocumentStore::Resident(document) => document.read_range(range),
            DocumentStore::Paged(document) => document.read_range(range),
        }
    }

    /// 获取可写入磁盘的完整编码字节，供保存和导出共享同一实现。
    pub fn serialized_bytes(&self) -> Result<Vec<u8>, PagedDocumentError> {
        match &self.store {
            DocumentStore::Resident(document) => document.encoded_bytes(),
            DocumentStore::Paged(document) => document.read_range(0..document.len()),
        }
    }

    /// 在大文档读取期间响应取消信号，避免后台读取阻塞保存或关闭流程。
    pub fn read_range_cancellable(
        &self,
        range: Range<u64>,
        cancellation: &SearchCancellation,
    ) -> Result<Vec<u8>, PagedDocumentError> {
        match &self.store {
            DocumentStore::Resident(document) => {
                document.read_range_cancellable(range, cancellation)
            }
            DocumentStore::Paged(document) => document.read_range_cancellable(range, cancellation),
        }
    }

    /// 以视口粒度读取正文，保持大文档 UI 只请求可见数据。
    pub fn read_viewport(
        &self,
        request: &ViewportRequest,
    ) -> Result<ViewportSnapshot, PagedDocumentError> {
        match &self.store {
            DocumentStore::Resident(document) => document.read_viewport(request),
            DocumentStore::Paged(document) => document.read_viewport(request),
        }
    }

    /// 为视口读取提供取消边界，避免快速滚动积压旧请求。
    pub fn read_viewport_cancellable(
        &self,
        request: &ViewportRequest,
        cancellation: &SearchCancellation,
    ) -> Result<ViewportSnapshot, PagedDocumentError> {
        match &self.store {
            DocumentStore::Resident(document) => {
                document.read_viewport_cancellable(request, cancellation)
            }
            DocumentStore::Paged(document) => {
                document.read_viewport_cancellable(request, cancellation)
            }
        }
    }

    /// 将正文流式写入输出，复用各后端的编码和格式处理能力。
    pub fn write_to(&self, output: impl Write) -> Result<(), PagedDocumentError> {
        match &self.store {
            DocumentStore::Resident(document) => document.write_to(output),
            DocumentStore::Paged(document) => document.write_to(output),
        }
    }

    /// 写出正文时传播取消信号，确保关闭/保存任务可以及时结束。
    pub fn write_to_cancellable(
        &self,
        output: impl Write,
        cancellation: &SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        match &self.store {
            DocumentStore::Resident(document) => {
                document.write_to_cancellable(output, cancellation)
            }
            DocumentStore::Paged(document) => document.write_to_cancellable(output, cancellation),
        }
    }

    /// 统一后端搜索入口，确保取消和匹配选项以相同方式传递。
    pub fn search(
        &self,
        query: &str,
        options: SearchOptions,
        cancellation: &SearchCancellation,
    ) -> Result<Vec<SearchMatch>, PagedDocumentError> {
        match &self.store {
            DocumentStore::Resident(document) => document.search(query, options, cancellation),
            DocumentStore::Paged(document) => document.search(query, options, cancellation),
        }
    }

    /// 查询外部文件变化，保留 resident 与 paged 后端各自的检测语义。
    pub fn external_change(&self) -> Result<ExternalChange, PagedDocumentError> {
        match &self.store {
            DocumentStore::Resident(document) => document.external_change(),
            DocumentStore::Paged(document) => document.external_change(),
        }
    }

    /// 接受外部追加时把索引只交给需要它的 paged 后端。
    pub fn accept_external_append(
        &mut self,
        source: FileSource,
        index: LineIndex,
    ) -> Result<(), PagedDocumentError> {
        match &mut self.store {
            DocumentStore::Resident(document) => document.accept_external_append(source),
            DocumentStore::Paged(document) => document.accept_external_append(source, index),
        }
    }

    /// 更新正文后立即按当前内容和编码重新计算 dirty，避免假设编辑必然产生变化。
    pub fn replace_text(
        &mut self,
        range: Range<u64>,
        replacement: &str,
    ) -> Result<(), PagedDocumentError> {
        let result = match &mut self.store {
            DocumentStore::Resident(document) => document.replace_text(range, replacement),
            DocumentStore::Paged(document) => document.replace_text(range, replacement),
        };
        if result.is_ok() {
            self.dirty = !self.is_pristine() || self.profile.encoding != self.persisted_encoding;
            self.refresh_resident_profile();
        }
        result
    }

    /// 通过 reader 执行大范围替换，避免先在调用方构造完整字符串。
    pub fn replace_text_reader(
        &mut self,
        range: Range<u64>,
        reader: impl Read,
    ) -> Result<(), PagedDocumentError> {
        let result = match &mut self.store {
            DocumentStore::Resident(document) => document.replace_text_reader(range, reader),
            DocumentStore::Paged(document) => document.replace_text_reader(range, reader),
        };
        if result.is_ok() {
            self.dirty = !self.is_pristine() || self.profile.encoding != self.persisted_encoding;
            self.refresh_resident_profile();
        }
        result
    }

    /// 将源事务适配为 paged API 错误，保持调用方只需处理一个结果类型。
    pub fn apply_source_transaction(
        &mut self,
        transaction: &Transaction,
    ) -> Result<(), PagedDocumentError> {
        self.apply_transaction(transaction)
            .map(|_| ())
            .map_err(|error| PagedDocumentError::InvalidTransaction(error.to_string()))
    }

    /// 撤销后重新计算 dirty 与 resident 派生统计，避免基线状态滞后。
    pub fn undo(&mut self) -> bool {
        let changed = match &mut self.store {
            DocumentStore::Resident(document) => document.undo(),
            DocumentStore::Paged(document) => document.undo(),
        };
        if changed {
            self.dirty = !self.is_pristine() || self.profile.encoding != self.persisted_encoding;
            self.refresh_resident_profile();
        }
        changed
    }

    /// 重做后重新计算 dirty 与 resident 派生统计，避免基线状态滞后。
    pub fn redo(&mut self) -> bool {
        let changed = match &mut self.store {
            DocumentStore::Resident(document) => document.redo(),
            DocumentStore::Paged(document) => document.redo(),
        };
        if changed {
            self.dirty = !self.is_pristine() || self.profile.encoding != self.persisted_encoding;
            self.refresh_resident_profile();
        }
        changed
    }

    /// 让后端执行原子保存，统一 resident 与 paged 的取消语义。
    pub fn save_atomic_cancellable(
        &mut self,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        match &mut self.store {
            DocumentStore::Resident(document) => {
                document.save_atomic_cancellable(path, cancellation)
            }
            DocumentStore::Paged(document) => document.save_atomic_cancellable(path, cancellation),
        }
    }

    /// 原子保存指定范围，避免为局部导出复制整个正文。
    pub fn save_range_atomic_cancellable(
        &self,
        range: Range<u64>,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<(), PagedDocumentError> {
        match &self.store {
            DocumentStore::Resident(document) => {
                document.save_range_atomic_cancellable(range, path, cancellation)
            }
            DocumentStore::Paged(document) => {
                document.save_range_atomic_cancellable(range, path, cancellation)
            }
        }
    }

    /// 暴露 paged 的预编码计划，让后台任务可在锁外复用临时文件。
    pub fn prepared_save_plan(&self) -> Option<EncodedSavePlan> {
        match &self.store {
            DocumentStore::Resident(_) => None,
            DocumentStore::Paged(document) => document.prepared_save_plan(),
        }
    }

    /// 使用已准备计划执行原子保存，并回读 resident 目标身份作为统一结果。
    pub fn save_prepared_atomic_cancellable(
        &mut self,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<gmark_paged_document::FileIdentity, PagedDocumentError> {
        let path = path.as_ref();
        match &mut self.store {
            DocumentStore::Resident(document) => {
                document.save_atomic_cancellable(path, cancellation)?;
                FileSource::open(path)?.identity()
            }
            DocumentStore::Paged(document) => {
                document.save_prepared_atomic_cancellable(path, cancellation)
            }
        }
    }

    /// 使用已准备计划执行 Save As，保持目标替换和身份回读的一致语义。
    pub fn save_prepared_atomic_as_cancellable(
        &mut self,
        path: impl AsRef<Path>,
        cancellation: &SearchCancellation,
    ) -> Result<gmark_paged_document::FileIdentity, PagedDocumentError> {
        let path = path.as_ref();
        match &mut self.store {
            DocumentStore::Resident(document) => {
                document.save_atomic_cancellable(path, cancellation)?;
                FileSource::open(path)?.identity()
            }
            DocumentStore::Paged(document) => {
                document.save_prepared_atomic_as_cancellable(path, cancellation)
            }
        }
    }

    /// 提交 paged 保存后的身份，让后续 save snapshot 能复用原子写入结果。
    pub fn mark_paged_saved(&mut self, identity: gmark_paged_document::FileIdentity) {
        if let DocumentStore::Paged(document) = &mut self.store {
            document.mark_prepared_saved(identity);
        }
    }
}

impl DocumentSnapshot for DocumentSession {
    /// 快照 revision 必须直接来自权威 store，供并发保存比较新旧版本。
    fn revision(&self) -> DocumentRevision {
        self.store.revision()
    }

    /// 复用会话长度逻辑，确保快照和编辑器观察到同一正文大小。
    fn len(&self) -> u64 {
        DocumentSession::len(self)
    }

    /// 将后端读取错误映射为快照契约，避免把具体后端类型泄漏给保存 worker。
    fn read_range(&self, range: Range<u64>) -> Result<Vec<u8>, gmark_document_core::SnapshotError> {
        DocumentSession::read_range(self, range).map_err(|error| match error {
            PagedDocumentError::InvalidRange { start, end, len } => {
                gmark_document_core::SnapshotError::InvalidRange { start, end, len }
            }
            PagedDocumentError::RangeTooLarge => gmark_document_core::SnapshotError::RangeTooLarge,
            error => gmark_document_core::SnapshotError::Read(error.to_string()),
        })
    }
}
