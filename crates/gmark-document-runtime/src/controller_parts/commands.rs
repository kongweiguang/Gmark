// @author kongweiguang

//! Controller 命令分发；每个命令只在这一层串联会话、视图映射和事件。

use super::super::*;

impl DocumentController {
    /// 将所有正文和元数据命令串行化，确保 selection、undo 与保存 revision 同步更新。
    pub fn dispatch(&mut self, command: DocumentCommand) -> Result<(), ControllerError> {
        if matches!(
            &command,
            DocumentCommand::ApplyTransaction { .. }
                | DocumentCommand::NormalizeLineEndings { .. }
                | DocumentCommand::RestoreSourceFormat { .. }
                | DocumentCommand::SetEncoding { .. }
                | DocumentCommand::AcceptExternalAppend { .. }
                | DocumentCommand::ReloadPreparedDocument { .. }
                | DocumentCommand::ReloadPreparedDocumentAfterConfirmation { .. }
                | DocumentCommand::Undo { .. }
                | DocumentCommand::Redo { .. }
                | DocumentCommand::DiscardChanges { .. }
        ) {
            self.break_typing_group();
        }
        match command {
            DocumentCommand::ApplyTransaction {
                view_id,
                transaction_id,
                transaction,
                selection_before,
                selection_after,
            } => self.apply_transaction_command(
                view_id,
                transaction_id,
                transaction,
                selection_before,
                selection_after,
            ),
            DocumentCommand::ApplyTypingTransaction {
                view_id,
                transaction_id,
                transaction,
                selection_before,
                selection_after,
                group_id,
            } => self.apply_typing_transaction_command(
                view_id,
                transaction_id,
                transaction,
                selection_before,
                selection_after,
                group_id,
            ),
            DocumentCommand::NormalizeLineEndings {
                view_id,
                transaction_id,
                ending,
                selection_before,
                selection_after,
            } => self.normalize_line_endings_command(
                view_id,
                transaction_id,
                ending,
                selection_before,
                selection_after,
            ),
            DocumentCommand::RestoreSourceFormat {
                view_id,
                transaction_id,
                format,
                selection_before,
                selection_after,
            } => self.restore_source_format_command(
                view_id,
                transaction_id,
                format,
                selection_before,
                selection_after,
            ),
            DocumentCommand::SetEncoding {
                view_id,
                transaction_id,
                encoding,
            } => {
                self.set_encoding(view_id, transaction_id, encoding)?;
                Ok(())
            }
            DocumentCommand::AcceptExternalAppend {
                expected_revision,
                expected_identity,
                source,
                index,
                identity,
            } => self.accept_external_append(
                expected_revision,
                expected_identity,
                source,
                index,
                identity,
            ),
            DocumentCommand::ReloadPreparedDocument {
                expected_revision,
                expected_identity,
                prepared,
            } => self.reload_prepared_document(expected_revision, expected_identity, prepared),
            DocumentCommand::ReloadPreparedDocumentAfterConfirmation {
                expected_revision,
                expected_identity,
                prepared,
            } => self.reload_prepared_document_after_confirmation(
                expected_revision,
                expected_identity,
                prepared,
            ),
            DocumentCommand::Undo {
                view_id,
                transaction_id,
            } => self.undo_command(view_id, transaction_id),
            DocumentCommand::Redo {
                view_id,
                transaction_id,
            } => self.redo_command(view_id, transaction_id),
            DocumentCommand::DiscardChanges { expected_revision } => {
                self.discard_changes(expected_revision).map(|_| ())
            }
            DocumentCommand::RequestSave => {
                let _ = self.request_save_snapshot()?;
                Ok(())
            }
            DocumentCommand::SaveSucceeded { revision, identity } => {
                let _ = self.complete_save(revision, identity)?;
                Ok(())
            }
            DocumentCommand::SaveFailed { revision, code } => {
                self.fail_save(revision, code).map(|_| ())
            }
            DocumentCommand::ExternalConflict { identity } => {
                self.emit_external_conflict(identity);
                Ok(())
            }
        }
    }

    /// 先基于不可变快照建立逆映射，再应用编辑，避免读取被修改正文而失去 undo 定位。
    fn apply_transaction_command(
        &mut self,
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
        transaction: Transaction,
        selection_before: SourceSelection,
        selection_after: SourceSelection,
    ) -> Result<(), ControllerError> {
        self.register_view(view_id);
        let snapshot = self.session.snapshot();
        let mutation = super::super::build_mutation_map(&transaction, snapshot.as_ref())?;
        let before_dirty = self.session.dirty;
        let revision = self.session.apply_transaction(&transaction)?;
        for (other_view_id, view) in &mut self.views {
            if *other_view_id != view_id {
                view.selection = mutation.map_selection(view.selection);
            }
        }
        if let Some(view) = self.views.get_mut(&view_id) {
            view.selection = selection_after;
        }
        self.undo_transactions.push(TransactionRuntimeRecord {
            view_id,
            mutation: mutation.clone(),
            selection_before,
            selection_after,
            typing_group: None,
        });
        self.redo_transactions.clear();
        self.emit(DocumentEvent::RevisionChanged {
            sequence: 0,
            document_id: self.document_id,
            view_id,
            transaction_id,
            revision,
            dirty: self.session.dirty,
            mutation,
            selection: selection_after,
        });
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

    /// 仅合并同 view、连续 revision/selection 且坐标相邻的单行尾部输入。
    fn apply_typing_transaction_command(
        &mut self,
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
        transaction: Transaction,
        selection_before: SourceSelection,
        selection_after: SourceSelection,
        group_id: TypingGroupId,
    ) -> Result<(), ControllerError> {
        self.register_view(view_id);
        let snapshot = self.session.snapshot();
        let mutation = match super::super::build_mutation_map(&transaction, snapshot.as_ref()) {
            Ok(mutation) => mutation,
            Err(error) => {
                self.break_typing_group();
                return Err(error);
            }
        };
        let current_revision = DocumentRevision(self.session.revision());
        let record_is_current = self.undo_transactions.last().is_some_and(|record| {
            record.view_id == view_id
                && record.typing_group == Some(group_id)
                && record.selection_after == selection_before
        });
        let combined = if is_continuable_typing_transaction(&transaction) {
            self.active_typing_group.as_ref().and_then(|active| {
                (active.group_id == group_id
                    && active.view_id == view_id
                    && active.revision == transaction.base_revision
                    && active.revision == current_revision
                    && active.selection_after == selection_before
                    && self.view_selection(view_id) == Some(selection_before)
                    && record_is_current)
                    .then(|| active.mutation.append_adjacent_insertion(&mutation))
                    .flatten()
            })
        } else {
            None
        };
        if combined.is_none() {
            self.break_typing_group();
        }

        let before_dirty = self.session.dirty;
        let revision = match self
            .session
            .apply_typing_transaction(&transaction, group_id)
        {
            Ok(revision) => revision,
            Err(error) => {
                self.break_typing_group();
                return Err(error.into());
            }
        };
        for (other_view_id, view) in &mut self.views {
            if *other_view_id != view_id {
                view.selection = mutation.map_selection(view.selection);
            }
        }
        if let Some(view) = self.views.get_mut(&view_id) {
            view.selection = selection_after;
        }

        let accumulated_mutation = if let Some(combined) = combined {
            if let Some(record) = self.undo_transactions.last_mut() {
                record.mutation = combined.clone();
                record.selection_after = selection_after;
            }
            combined
        } else {
            self.undo_transactions.push(TransactionRuntimeRecord {
                view_id,
                mutation: mutation.clone(),
                selection_before,
                selection_after,
                typing_group: Some(group_id),
            });
            mutation.clone()
        };
        self.redo_transactions.clear();
        if is_continuable_typing_transaction(&transaction) {
            self.active_typing_group = Some(ActiveTypingGroup {
                group_id,
                view_id,
                revision,
                selection_after,
                mutation: accumulated_mutation,
            });
        } else {
            self.break_typing_group();
        }
        self.emit(DocumentEvent::RevisionChanged {
            sequence: 0,
            document_id: self.document_id,
            view_id,
            transaction_id,
            revision,
            dirty: self.session.dirty,
            mutation,
            selection: selection_after,
        });
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

    /// 将行尾规范化视为无正文坐标映射的共享事务，防止各视图自行推断格式变化。
    fn normalize_line_endings_command(
        &mut self,
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
        ending: LineEnding,
        selection_before: SourceSelection,
        selection_after: SourceSelection,
    ) -> Result<(), ControllerError> {
        self.register_view(view_id);
        let before_dirty = self.session.dirty;
        if !self.session.normalize_line_endings(ending)? {
            return Ok(());
        }
        let revision = DocumentRevision(self.session.revision());
        self.set_all_selections(view_id, selection_after, &DocumentMutationMap::empty());
        self.undo_transactions.push(TransactionRuntimeRecord {
            view_id,
            mutation: DocumentMutationMap::empty(),
            selection_before,
            selection_after,
            typing_group: None,
        });
        self.redo_transactions.clear();
        self.emit(DocumentEvent::RevisionChanged {
            sequence: 0,
            document_id: self.document_id,
            view_id,
            transaction_id,
            revision,
            dirty: self.session.dirty,
            mutation: DocumentMutationMap::empty(),
            selection: selection_after,
        });
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

    /// 格式恢复同样只影响共享正文 revision，不伪造一组具体 byte edits。
    fn restore_source_format_command(
        &mut self,
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
        format: SourceFormatSnapshot,
        selection_before: SourceSelection,
        selection_after: SourceSelection,
    ) -> Result<(), ControllerError> {
        self.register_view(view_id);
        let before_dirty = self.session.dirty;
        if !self.session.restore_source_format(format)? {
            return Ok(());
        }
        let revision = DocumentRevision(self.session.revision());
        self.set_all_selections(view_id, selection_after, &DocumentMutationMap::empty());
        self.undo_transactions.push(TransactionRuntimeRecord {
            view_id,
            mutation: DocumentMutationMap::empty(),
            selection_before,
            selection_after,
            typing_group: None,
        });
        self.redo_transactions.clear();
        self.emit(DocumentEvent::RevisionChanged {
            sequence: 0,
            document_id: self.document_id,
            view_id,
            transaction_id,
            revision,
            dirty: self.session.dirty,
            mutation: DocumentMutationMap::empty(),
            selection: selection_after,
        });
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

    /// Undo 使用逆映射移动其它视图，并仅恢复原事务所属 view 的 selection。
    fn undo_command(
        &mut self,
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
    ) -> Result<(), ControllerError> {
        let before_dirty = self.session.dirty;
        if !self.session.undo() {
            return Ok(());
        }
        let record = self.undo_transactions.pop();
        let mutation = record
            .as_ref()
            .and_then(|record| record.mutation.inverse())
            .unwrap_or_else(DocumentMutationMap::empty);
        self.map_all_selections(&mutation);
        if let Some(record) = record {
            if let Some(origin) = self.views.get_mut(&record.view_id) {
                origin.selection = record.selection_before;
            }
            self.redo_transactions.push(record);
        }
        let revision = DocumentRevision(self.session.revision());
        self.emit(DocumentEvent::RevisionChanged {
            sequence: 0,
            document_id: self.document_id,
            view_id,
            transaction_id,
            revision,
            dirty: self.session.dirty,
            mutation,
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
        Ok(())
    }

    /// Redo reapplies the original coordinate map and restores the post-edit selection.
    fn redo_command(
        &mut self,
        view_id: DocumentViewInstanceId,
        transaction_id: TransactionId,
    ) -> Result<(), ControllerError> {
        let before_dirty = self.session.dirty;
        if !self.session.redo() {
            return Ok(());
        }
        let record = self.redo_transactions.pop();
        let mutation = record
            .as_ref()
            .map(|record| record.mutation.clone())
            .unwrap_or_else(DocumentMutationMap::empty);
        self.map_all_selections(&mutation);
        if let Some(record) = record {
            if let Some(origin) = self.views.get_mut(&record.view_id) {
                origin.selection = record.selection_after;
            }
            self.undo_transactions.push(record);
        }
        let revision = DocumentRevision(self.session.revision());
        self.emit(DocumentEvent::RevisionChanged {
            sequence: 0,
            document_id: self.document_id,
            view_id,
            transaction_id,
            revision,
            dirty: self.session.dirty,
            mutation,
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
        Ok(())
    }

    /// 映射所有非 origin view 的 selection；origin 由命令携带的精确 selection 覆盖。
    fn set_all_selections(
        &mut self,
        origin_view: DocumentViewInstanceId,
        origin_selection: SourceSelection,
        mutation: &DocumentMutationMap,
    ) {
        for (view_id, view) in &mut self.views {
            if *view_id == origin_view {
                view.selection = origin_selection;
            } else {
                view.selection = mutation.map_selection(view.selection);
            }
        }
    }

    /// Undo/redo 的命令 view 也必须先随逆映射移动，关闭 origin view 后仍能保留其它视图位置。
    fn map_all_selections(&mut self, mutation: &DocumentMutationMap) {
        for view in self.views.values_mut() {
            view.selection = mutation.map_selection(view.selection);
        }
    }
}

/// 允许组首替换 selection，但后续可合并输入必须是非空单行文本。
fn is_continuable_typing_transaction(transaction: &Transaction) -> bool {
    let [edit] = transaction.edits.as_slice() else {
        return false;
    };
    !edit.replacement.is_empty()
        && !edit.replacement.contains('\r')
        && !edit.replacement.contains('\n')
}
