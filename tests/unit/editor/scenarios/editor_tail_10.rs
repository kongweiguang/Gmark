// @author kongweiguang

// 大文档复制、输入、恢复、几何与投影测试的薄聚合门面。

include!("editor_tail_10/copy_lifecycle.rs");
include!("editor_tail_10/copy_boundaries.rs");
include!("editor_tail_10/source_interaction.rs");
include!("editor_tail_10/source_accessibility.rs");
#[path = "editor_tail_10/source_save_concurrency.rs"]
mod source_save_concurrency;
#[path = "editor_tail_10/source_auto_save.rs"]
mod source_auto_save;
include!("editor_tail_10/recovery_and_structured_data.rs");
include!("editor_tail_10/viewport_geometry.rs");
include!("editor_tail_10/scrolling_behavior.rs");
include!("editor_tail_10/projection_helpers.rs");
