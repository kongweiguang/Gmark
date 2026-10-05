// @author kongweiguang

//! 未提交的确认文字属于原投影树；恢复前不能随焦点变化或后台同步丢弃。

use super::DocumentTree;

impl DocumentTree {
    /// 已确认文字按事件顺序保留，分配失败交由上层剪贴板恢复，不先修改正文。
    pub(in crate::editor) fn retain_source_commit_replacement(
        &mut self,
        text: &str,
    ) -> Result<(), String> {
        let pending = self
            .source_commit_replacement
            .get_or_insert_with(String::new);
        pending
            .try_reserve(text.len())
            .map_err(|_| "无法暂存未提交文字".to_owned())?;
        pending.push_str(text);
        Ok(())
    }

    /// 恢复复制同时读取可见树和未发布文字，不能把确认结果遗漏在临时事件中。
    pub(in crate::editor) fn source_commit_replacement(&self) -> Option<&str> {
        self.source_commit_replacement.as_deref()
    }
}
