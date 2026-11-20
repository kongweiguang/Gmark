// @author kongweiguang

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

/// 与 Block 同生命周期的取消所有者，释放时通知已经运行的语法解析停止。
pub(super) struct HighlightCancellation(pub(super) Arc<AtomicUsize>);

impl Drop for HighlightCancellation {
    /// 视图关闭和新代次均通过同一标志终止旧分析。
    fn drop(&mut self) {
        self.0.store(1, Ordering::Relaxed);
    }
}

/// 大文件 Source 行的稳定布局身份。动态字体、主题、缩放与换行宽度在实际 shape
/// 时补入缓存键；这里保留文档侧不变量，避免 revision 重置或横向窗口变化时复用旧行。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceLayoutIdentity {
    pub(crate) document_epoch: u64,
    pub(crate) document_revision: u64,
    pub(crate) source_range: Range<u64>,
    pub(crate) column_window_start: u64,
    pub(crate) show_line_endings: bool,
}

/// 正文持有共享引用，保证指针比较期间不会复用已释放内存。
#[derive(Clone, Debug)]
pub(crate) struct SourceLayoutCacheKey {
    pub(crate) identity: Option<SourceLayoutIdentity>,
    pub(crate) text: SharedString,
    pub(crate) syntax_generation: u64,
    pub(crate) marked_range: Option<Range<usize>>,
    pub(crate) theme_identity: usize,
    pub(crate) text_color: Hsla,
    pub(crate) font: Font,
    pub(crate) font_size_bits: u32,
    pub(crate) line_height_bits: u32,
    pub(crate) scale_bits: u32,
    pub(crate) wrap_width_bits: Option<u32>,
    pub(crate) soft_wrap: bool,
}

impl PartialEq for SourceLayoutCacheKey {
    /// 不逐字比较长正文；缓存持有旧正文确保同地址同长度代表同一不可变文本。
    fn eq(&self, other: &Self) -> bool {
        self.same_text_style(other) && self.wrap_width_bits == other.wrap_width_bits
    }
}

impl SourceLayoutCacheKey {
    /// 分屏布局会用不同宽度试算；同一份正文和渲染环境可以共用这些试算结果。
    pub(crate) fn same_text_style(&self, other: &Self) -> bool {
        std::ptr::eq(self.text.as_ref(), other.text.as_ref()) && self.same_style(other)
    }

    /// 正文修改后，度量参数一致的旧布局仍可提供未变硬行的字形。
    pub(crate) fn same_style(&self, other: &Self) -> bool {
        self.identity == other.identity
            && self.syntax_generation == other.syntax_generation
            && self.marked_range == other.marked_range
            && self.theme_identity == other.theme_identity
            && self.text_color == other.text_color
            && self.font == other.font
            && self.font_size_bits == other.font_size_bits
            && self.line_height_bits == other.line_height_bits
            && self.scale_bits == other.scale_bits
            && self.soft_wrap == other.soft_wrap
    }
}

#[derive(Clone)]
pub(crate) struct SourceLayoutMeasurement {
    pub(crate) key: SourceLayoutCacheKey,
    pub(crate) size: Size<Pixels>,
    pub(crate) lines: Arc<Vec<WrappedLine>>,
}
