// @author kongweiguang

use super::*;

impl EditorDocumentSession {
    /// 在慢速磁盘读取前捕获共享正文基线，避免迟到 reload 覆盖 peer 的新编辑。
    pub(in crate::editor) fn try_external_reload_baseline(
        &self,
    ) -> Result<(DocumentRevision, FileIdentity), EditorDocumentSessionError> {
        let handle = self.handle()?;
        let controller = handle.lock().map_err(EditorDocumentSessionError::from)?;
        Ok((
            DocumentRevision(controller.session().revision()),
            controller.session().file_identity.clone(),
        ))
    }

    /// 已由用户确认的 reload 可以丢弃 dirty，但仍经共享命令核对读取前基线。
    pub(in crate::editor) fn try_reload_prepared_document_after_confirmation(
        &self,
        expected_revision: DocumentRevision,
        expected_identity: FileIdentity,
        prepared: DocumentSession,
    ) -> Result<(), EditorDocumentSessionError> {
        self.dispatch(DocumentCommand::ReloadPreparedDocumentAfterConfirmation {
            expected_revision,
            expected_identity,
            prepared,
        })
    }
}
