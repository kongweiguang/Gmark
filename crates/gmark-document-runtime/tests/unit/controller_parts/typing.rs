// @author kongweiguang

use super::*;
use gmark_document_core::{
    DocumentRevision, DocumentViewInstanceId, SourceEdit, SourceSelection, Transaction,
    TypingGroupId,
};

/// 通过公开命令验证身份、revision 与前后选区；显式保留契约参数，让失败用例可独立改变各个边界。
#[allow(clippy::too_many_arguments)]
fn apply_typing(
    controller: &mut DocumentController,
    view_id: DocumentViewInstanceId,
    transaction_id: u64,
    revision: u64,
    range: std::ops::Range<u64>,
    replacement: &str,
    selection_before: SourceSelection,
    selection_after: SourceSelection,
    group_id: TypingGroupId,
) -> Result<(), ControllerError> {
    controller.dispatch(DocumentCommand::ApplyTypingTransaction {
        view_id,
        transaction_id: TransactionId(transaction_id),
        transaction: Transaction::new(
            DocumentRevision(revision),
            vec![SourceEdit::new(range, replacement)],
        ),
        selection_before,
        selection_after,
        group_id,
    })
}

/// Read through the immutable snapshot contract instead of inspecting backend history internals.
fn source_bytes(controller: &DocumentController) -> Vec<u8> {
    let snapshot = controller.session().snapshot();
    snapshot
        .read_range(0..snapshot.len())
        .unwrap_or_else(|error| panic!("read source snapshot: {error}"))
}

/// A selection replacement and adjacent typing share one undo item, including its original selection.
#[test]
fn typing_group_replaces_selection_then_appends_and_undoes_redoes_once() {
    let mut controller = DocumentController::new(DocumentId::new(), session());
    let view_id = DocumentViewInstanceId::new();
    let group_id = TypingGroupId::new();
    let selected = SourceSelection::from_range(0..1, false);
    let after_first = SourceSelection::collapsed(1, gmark_document_core::SourceAffinity::After);
    let after_second = SourceSelection::collapsed(2, gmark_document_core::SourceAffinity::After);
    controller.set_view_selection(view_id, selected);

    apply_typing(
        &mut controller,
        view_id,
        1,
        0,
        0..1,
        "O",
        selected,
        after_first,
        group_id,
    )
    .unwrap_or_else(|error| panic!("replace selection: {error}"));
    apply_typing(
        &mut controller,
        view_id,
        2,
        1,
        1..1,
        "X",
        after_first,
        after_second,
        group_id,
    )
    .unwrap_or_else(|error| panic!("append typing: {error}"));
    assert_eq!(source_bytes(&controller), b"OXne");

    controller
        .dispatch(DocumentCommand::Undo {
            view_id,
            transaction_id: TransactionId(3),
        })
        .unwrap_or_else(|error| panic!("undo typing group: {error}"));
    assert_eq!(source_bytes(&controller), b"one");
    assert_eq!(controller.view_selection(view_id), Some(selected));
    controller
        .dispatch(DocumentCommand::Redo {
            view_id,
            transaction_id: TransactionId(4),
        })
        .unwrap_or_else(|error| panic!("redo typing group: {error}"));
    assert_eq!(source_bytes(&controller), b"OXne");
    assert_eq!(controller.view_selection(view_id), Some(after_second));
}

/// A group ID reused by a peer view cannot merge across the view ownership boundary.
#[test]
fn peer_view_cannot_continue_an_existing_typing_group() {
    let mut controller = DocumentController::new(DocumentId::new(), session());
    let first_view = DocumentViewInstanceId::new();
    let peer_view = DocumentViewInstanceId::new();
    let group_id = TypingGroupId::new();
    let at_end = SourceSelection::collapsed(3, gmark_document_core::SourceAffinity::After);
    let after_a = SourceSelection::collapsed(4, gmark_document_core::SourceAffinity::After);
    let after_b = SourceSelection::collapsed(5, gmark_document_core::SourceAffinity::After);

    apply_typing(
        &mut controller,
        first_view,
        1,
        0,
        3..3,
        "a",
        at_end,
        after_a,
        group_id,
    )
    .unwrap_or_else(|error| panic!("first view typing: {error}"));
    apply_typing(
        &mut controller,
        first_view,
        2,
        1,
        4..4,
        "b",
        after_a,
        after_b,
        group_id,
    )
    .unwrap_or_else(|error| panic!("continue first view typing: {error}"));
    apply_typing(
        &mut controller,
        peer_view,
        3,
        2,
        0..0,
        "p",
        SourceSelection::default(),
        SourceSelection::collapsed(1, gmark_document_core::SourceAffinity::After),
        group_id,
    )
    .unwrap_or_else(|error| panic!("peer typing: {error}"));
    let after_peer = SourceSelection::collapsed(6, gmark_document_core::SourceAffinity::After);
    let after_peer_append =
        SourceSelection::collapsed(7, gmark_document_core::SourceAffinity::After);
    apply_typing(
        &mut controller,
        first_view,
        4,
        3,
        6..6,
        "c",
        after_peer,
        after_peer_append,
        group_id,
    )
    .unwrap_or_else(|error| panic!("reuse old group ID: {error}"));
    assert_eq!(source_bytes(&controller), b"poneabc");

    for (transaction_id, expected) in [(5, b"poneab".as_slice()), (6, b"oneab"), (7, b"one")] {
        controller
            .dispatch(DocumentCommand::Undo {
                view_id: first_view,
                transaction_id: TransactionId(transaction_id),
            })
            .unwrap_or_else(|error| panic!("undo view-scoped group: {error}"));
        assert_eq!(source_bytes(&controller), expected);
    }
}

/// An ordinary edit ends typing coalescing even if the next command reuses the old identity.
#[test]
fn ordinary_edit_breaks_typing_group_and_keeps_its_own_undo_boundary() {
    let mut controller = DocumentController::new(DocumentId::new(), session());
    let view_id = DocumentViewInstanceId::new();
    let group_id = TypingGroupId::new();
    let at_end = SourceSelection::collapsed(3, gmark_document_core::SourceAffinity::After);
    let after_a = SourceSelection::collapsed(4, gmark_document_core::SourceAffinity::After);
    let after_b = SourceSelection::collapsed(5, gmark_document_core::SourceAffinity::After);

    apply_typing(
        &mut controller,
        view_id,
        1,
        0,
        3..3,
        "a",
        at_end,
        after_a,
        group_id,
    )
    .unwrap_or_else(|error| panic!("first typing: {error}"));
    apply_typing(
        &mut controller,
        view_id,
        2,
        1,
        4..4,
        "b",
        after_a,
        after_b,
        group_id,
    )
    .unwrap_or_else(|error| panic!("continued typing: {error}"));
    controller
        .dispatch(DocumentCommand::ApplyTransaction {
            view_id,
            transaction_id: TransactionId(3),
            transaction: Transaction::new(DocumentRevision(2), vec![SourceEdit::new(5..5, "!")]),
            selection_before: after_b,
            selection_after: SourceSelection::collapsed(
                6,
                gmark_document_core::SourceAffinity::After,
            ),
        })
        .unwrap_or_else(|error| panic!("ordinary edit: {error}"));
    apply_typing(
        &mut controller,
        view_id,
        4,
        3,
        6..6,
        "c",
        SourceSelection::collapsed(6, gmark_document_core::SourceAffinity::After),
        SourceSelection::collapsed(7, gmark_document_core::SourceAffinity::After),
        group_id,
    )
    .unwrap_or_else(|error| panic!("typing after ordinary edit: {error}"));
    assert_eq!(source_bytes(&controller), b"oneab!c");

    for (transaction_id, expected) in [(5, b"oneab!".as_slice()), (6, b"oneab"), (7, b"one")] {
        controller
            .dispatch(DocumentCommand::Undo {
                view_id,
                transaction_id: TransactionId(transaction_id),
            })
            .unwrap_or_else(|error| panic!("undo separated edit: {error}"));
        assert_eq!(source_bytes(&controller), expected);
    }
}

/// A changed view selection breaks the group before the same ID is reused at another source position.
#[test]
fn view_selection_change_breaks_typing_group() {
    let mut controller = DocumentController::new(DocumentId::new(), session());
    let view_id = DocumentViewInstanceId::new();
    let group_id = TypingGroupId::new();
    let at_end = SourceSelection::collapsed(3, gmark_document_core::SourceAffinity::After);
    let after_a = SourceSelection::collapsed(4, gmark_document_core::SourceAffinity::After);

    apply_typing(
        &mut controller,
        view_id,
        1,
        0,
        3..3,
        "a",
        at_end,
        after_a,
        group_id,
    )
    .unwrap_or_else(|error| panic!("first typing: {error}"));
    let moved = SourceSelection::collapsed(0, gmark_document_core::SourceAffinity::Before);
    controller.set_view_selection(view_id, moved);
    apply_typing(
        &mut controller,
        view_id,
        2,
        1,
        0..0,
        "p",
        moved,
        SourceSelection::collapsed(1, gmark_document_core::SourceAffinity::After),
        group_id,
    )
    .unwrap_or_else(|error| panic!("type after moving selection: {error}"));
    assert_eq!(source_bytes(&controller), b"ponea");
    controller
        .dispatch(DocumentCommand::Undo {
            view_id,
            transaction_id: TransactionId(3),
        })
        .unwrap_or_else(|error| panic!("undo after selection change: {error}"));
    assert_eq!(source_bytes(&controller), b"onea");
}

/// A stale typing transaction ends its group so the retried old ID starts a distinct undo item.
#[test]
fn stale_typing_rejection_breaks_the_group() {
    let mut controller = DocumentController::new(DocumentId::new(), session());
    let view_id = DocumentViewInstanceId::new();
    let group_id = TypingGroupId::new();
    let at_end = SourceSelection::collapsed(3, gmark_document_core::SourceAffinity::After);
    let after_a = SourceSelection::collapsed(4, gmark_document_core::SourceAffinity::After);
    apply_typing(
        &mut controller,
        view_id,
        1,
        0,
        3..3,
        "a",
        at_end,
        after_a,
        group_id,
    )
    .unwrap_or_else(|error| panic!("first typing: {error}"));
    assert!(
        apply_typing(
            &mut controller,
            view_id,
            2,
            0,
            4..4,
            "?",
            after_a,
            SourceSelection::collapsed(5, gmark_document_core::SourceAffinity::After),
            group_id,
        )
        .is_err()
    );
    apply_typing(
        &mut controller,
        view_id,
        3,
        1,
        4..4,
        "b",
        after_a,
        SourceSelection::collapsed(5, gmark_document_core::SourceAffinity::After),
        group_id,
    )
    .unwrap_or_else(|error| panic!("retry after stale rejection: {error}"));
    controller
        .dispatch(DocumentCommand::Undo {
            view_id,
            transaction_id: TransactionId(4),
        })
        .unwrap_or_else(|error| panic!("undo retry: {error}"));
    assert_eq!(source_bytes(&controller), b"onea");
}

/// Undo and redo close the active group so a post-redo transaction cannot rejoin its old history.
#[test]
fn undo_and_redo_break_typing_group_even_when_old_id_is_reused() {
    let mut controller = DocumentController::new(DocumentId::new(), session());
    let view_id = DocumentViewInstanceId::new();
    let group_id = TypingGroupId::new();
    let at_end = SourceSelection::collapsed(3, gmark_document_core::SourceAffinity::After);
    let after_a = SourceSelection::collapsed(4, gmark_document_core::SourceAffinity::After);
    let after_b = SourceSelection::collapsed(5, gmark_document_core::SourceAffinity::After);
    apply_typing(
        &mut controller,
        view_id,
        1,
        0,
        3..3,
        "a",
        at_end,
        after_a,
        group_id,
    )
    .unwrap_or_else(|error| panic!("first typing: {error}"));
    apply_typing(
        &mut controller,
        view_id,
        2,
        1,
        4..4,
        "b",
        after_a,
        after_b,
        group_id,
    )
    .unwrap_or_else(|error| panic!("continued typing: {error}"));
    controller
        .dispatch(DocumentCommand::Undo {
            view_id,
            transaction_id: TransactionId(3),
        })
        .unwrap_or_else(|error| panic!("undo typing group: {error}"));
    controller
        .dispatch(DocumentCommand::Redo {
            view_id,
            transaction_id: TransactionId(4),
        })
        .unwrap_or_else(|error| panic!("redo typing group: {error}"));
    apply_typing(
        &mut controller,
        view_id,
        5,
        4,
        5..5,
        "c",
        after_b,
        SourceSelection::collapsed(6, gmark_document_core::SourceAffinity::After),
        group_id,
    )
    .unwrap_or_else(|error| panic!("type after redo with old ID: {error}"));
    controller
        .dispatch(DocumentCommand::Undo {
            view_id,
            transaction_id: TransactionId(6),
        })
        .unwrap_or_else(|error| panic!("undo post-redo typing: {error}"));
    assert_eq!(source_bytes(&controller), b"oneab");
}
