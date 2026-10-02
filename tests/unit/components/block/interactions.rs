// @author kongweiguang

use super::Block;
use crate::components::{
    BlockEvent, BlockKind, BlockRecord, Copy, Cut, InlineTextTree, Paste, PastedImageSource,
    SelectAll, TableData,
};
use gpui::{AppContext, ClickEvent, ClipboardItem, TestAppContext};
use std::fs;
use std::sync::{Arc, Mutex};

fn temp_image_path(name: &str) -> std::path::PathBuf {
    let root =
        std::env::temp_dir().join(format!("gmark-paste-image-path-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&root).expect("temp image dir should exist");
    let path = root.join(name);
    fs::write(
        &path,
        b"not a real image; extension is enough for paste routing",
    )
    .expect("temp image should be written");
    path
}

fn remove_temp_image(path: &std::path::Path) {
    let _ = path.parent().map(fs::remove_dir_all);
}

#[gpui::test]
async fn read_only_preview_text_can_be_selected_and_copied(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        let mut block = Block::with_record(cx, BlockRecord::paragraph("preview text"));
        block.set_read_only(true);
        block
    });

    cx.update(|window, app| {
        app.write_to_clipboard(ClipboardItem::new_string("old clipboard".to_owned()));
        block.update(app, |block, block_cx| {
            block.on_select_all(&SelectAll, window, block_cx);
            block.on_copy(&Copy, window, block_cx);
        });
    });

    block.read_with(cx, |block, _cx| {
        assert_eq!(block.selected_range, 0.."preview text".len());
        assert!(block.is_read_only());
    });
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("preview text")
    );
}

/// Prevents stale append-button callbacks from emitting writes after a table becomes read-only.
#[gpui::test]
async fn read_only_table_does_not_emit_append_requests(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        let mut block = Block::with_record(cx, BlockRecord::table(TableData::new_empty(1, 2)));
        block.set_read_only(true);
        block
    });
    let append_requests = Arc::new(Mutex::new(0usize));
    let observed_requests = append_requests.clone();
    let _subscription = cx.update(|_window, cx| {
        cx.subscribe(&block, move |_, event: &BlockEvent, _| {
            if matches!(
                event,
                BlockEvent::RequestAppendTableColumn | BlockEvent::RequestAppendTableRow
            ) {
                *observed_requests.lock().expect("event count lock") += 1;
            }
        })
    });

    cx.update(|window, cx| {
        block.update(cx, |block, block_cx| {
            block.on_append_table_column(&ClickEvent::default(), window, block_cx);
            block.on_append_table_row(&ClickEvent::default(), window, block_cx);
        });
    });

    assert_eq!(*append_requests.lock().expect("event count lock"), 0);
}

#[gpui::test]
async fn source_document_supports_select_copy_cut_and_paste(cx: &mut TestAppContext) {
    let cx = cx.add_empty_window();
    let block = cx.new(|cx| {
        let mut block = Block::with_record(cx, BlockRecord::paragraph("# source\nbody"));
        block.set_source_document_mode();
        block
    });

    cx.update(|window, app| {
        block.update(app, |block, block_cx| {
            block.on_select_all(&SelectAll, window, block_cx);
            block.on_copy(&Copy, window, block_cx);
        });
    });
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("# source\nbody")
    );

    cx.update(|window, app| {
        app.write_to_clipboard(ClipboardItem::new_string("replacement".to_owned()));
        block.update(app, |block, block_cx| {
            block.on_paste(&Paste, window, block_cx);
            block.on_select_all(&SelectAll, window, block_cx);
            block.on_cut(&Cut, window, block_cx);
        });
    });

    block.read_with(cx, |block, _cx| assert!(block.display_text().is_empty()));
    assert_eq!(
        cx.read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("replacement")
    );
}

#[gpui::test]
async fn append_column_button_stays_visible_while_crossing_hover_gap(cx: &mut TestAppContext) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));

    block.update(cx, |block, cx| {
        block.set_table_append_column_hover_part(Some(true), None, None, cx);
        assert!(block.table_append_column_hovered);

        block.set_table_append_column_hover_part(Some(false), None, Some(true), cx);
        assert!(block.table_append_column_hovered);
        assert!(!block.table_append_column_edge_hovered);
        assert!(!block.table_append_column_zone_hovered);
        assert!(block.table_append_column_button_hovered);
        assert!(block.table_append_column_close_task.is_none());
    });
}

#[test]
fn paste_image_text_accepts_plain_local_image_path() {
    let path = temp_image_path("copied.png");
    let text = path.to_string_lossy().to_string();
    #[cfg(target_os = "windows")]
    assert!(
        text.contains(':'),
        "test should exercise Windows drive-letter paths"
    );

    let source = Block::pasted_image_source_from_text(&text);

    assert_eq!(source, Some(PastedImageSource::LocalPath(path.clone())));
    remove_temp_image(&path);
}

/// 路径不存在时仍只做语法分类；真实存在性与 64 MiB 校验必须留给后台 materialize，
/// 这样慢盘或 UNC 探测不会阻塞剪贴板回调的 UI heartbeat。
#[test]
fn paste_image_text_defers_missing_path_validation_to_background() {
    let path = temp_image_path("missing.png");
    remove_temp_image(&path);
    let text = path.to_string_lossy().to_string();

    let source = Block::pasted_image_source_from_text(&text);

    assert_eq!(source, Some(PastedImageSource::LocalPath(path)));
}

#[test]
fn paste_image_text_accepts_quoted_local_image_path() {
    let path = temp_image_path("quoted image.png");
    let text = format!("\"{}\"", path.display());

    let source = Block::pasted_image_source_from_text(&text);

    assert_eq!(source, Some(PastedImageSource::LocalPath(path.clone())));
    remove_temp_image(&path);
}

#[test]
fn paste_image_text_accepts_file_url() {
    let path = temp_image_path("url image.png");
    let url = url::Url::from_file_path(&path).expect("temp image path should form file URL");

    let source = Block::pasted_image_source_from_text(url.as_str());

    assert_eq!(source, Some(PastedImageSource::LocalPath(path.clone())));
    remove_temp_image(&path);
}

#[test]
fn paste_text_accepts_non_image_resource_path() {
    let path = temp_image_path("notes.txt");
    let text = path.to_string_lossy().to_string();

    let source = Block::pasted_image_source_from_text(&text);

    assert_eq!(source, Some(PastedImageSource::LocalResource(path.clone())));
    remove_temp_image(&path);
}

#[gpui::test]
async fn append_row_button_stays_visible_while_crossing_hover_gap(cx: &mut TestAppContext) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));

    block.update(cx, |block, cx| {
        block.set_table_append_row_hover_part(Some(true), None, None, cx);
        assert!(block.table_append_row_hovered);

        block.set_table_append_row_hover_part(Some(false), None, Some(true), cx);
        assert!(block.table_append_row_hovered);
        assert!(!block.table_append_row_edge_hovered);
        assert!(!block.table_append_row_zone_hovered);
        assert!(block.table_append_row_button_hovered);
        assert!(block.table_append_row_close_task.is_none());
    });
}

#[gpui::test]
async fn column_edge_hover_reveals_only_column_append_control(cx: &mut TestAppContext) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));

    block.update(cx, |block, cx| {
        block.set_table_append_column_hover_part(Some(true), None, None, cx);
        assert!(block.table_append_column_edge_hovered);
        assert!(block.table_append_column_hovered);
        assert!(!block.table_append_row_hovered);
        assert!(block.table_append_column_close_task.is_none());
        assert!(block.table_append_row_close_task.is_none());
    });
}

#[gpui::test]
async fn row_edge_hover_reveals_only_row_append_control(cx: &mut TestAppContext) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));

    block.update(cx, |block, cx| {
        block.set_table_append_row_hover_part(Some(true), None, None, cx);
        assert!(block.table_append_row_edge_hovered);
        assert!(block.table_append_row_hovered);
        assert!(!block.table_append_column_hovered);
        assert!(block.table_append_column_close_task.is_none());
        assert!(block.table_append_row_close_task.is_none());
    });
}

#[gpui::test]
async fn multiline_quote_is_not_treated_as_leaf(cx: &mut TestAppContext) {
    let block = cx.new(|cx| Block::with_record(cx, BlockRecord::paragraph(String::new())));

    block.update(cx, |block, cx| {
        block.record.kind = BlockKind::Quote;
        block.record.set_title(InlineTextTree::plain("first\n"));
        block.sync_edit_mode_from_kind();
        block.sync_render_cache();
        cx.notify();

        assert!(!block.is_leaf_quote());
    });
}
