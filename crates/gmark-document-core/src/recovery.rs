// @author kongweiguang

use crate::{DocumentViewId, PersistenceError, SourceSelection, Transaction, TypingGroupId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryAction {
    Transaction(Transaction),
    TypingTransaction {
        transaction: Transaction,
        group_id: TypingGroupId,
    },
    Undo,
    Redo,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecoveryRecord {
    pub action: RecoveryAction,
    pub selection: Option<SourceSelection>,
    pub view_id: DocumentViewId,
}

/// 两类日志可以采用不同编码，但 Controller 只提交源码 transaction 与统一视图状态。
pub trait RecoveryBackend {
    fn record(&mut self, record: &RecoveryRecord) -> Result<(), PersistenceError>;
}
