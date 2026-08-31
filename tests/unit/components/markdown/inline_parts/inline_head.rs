// @author kongweiguang

    use super::{
        InlineFragment, InlineInsertionAttributes, InlineLinkHit, InlineMathDelimiter,
        InlineScript, InlineStyle, InlineTextTree, LinkReferenceDefinitions, StyleFlag,
    };
    use crate::components::HtmlCssColor;

    #[test]
    fn projects_shared_inline_values_without_editor_runtime_state() {
        let document = gmark_markdown::parse_markdown("**bold** [link](https://example.test)");
        let values = &document.blocks[0].inlines;
        let tree = InlineTextTree::from_markdown_values(values);

        assert_eq!(tree.visible_text(), "bold link");
        assert_eq!(tree.serialize_markdown(), "**bold** [link](https://example.test)");
        assert_eq!(
            gmark_markdown::serialize_inlines_canonical(&tree.markdown_values()),
            "**bold** [link](https://example.test)"
        );
    }

    #[test]
    fn parses_supported_styles_and_serializes_canonically() {
        let tree = InlineTextTree::from_markdown("1**23**4*56*7<u>89</u>0***ab***<u>*cd*</u>");
        let serialized = tree.serialize_markdown();
        let reparsed = InlineTextTree::from_markdown(&serialized);

        assert_eq!(tree.visible_text(), "1234567890abcd");
        assert_eq!(reparsed.visible_text(), tree.visible_text());
        assert_eq!(reparsed.render_cache().spans(), tree.render_cache().spans());
    }

    #[test]
    fn parses_underscore_emphasis_and_canonicalizes_to_asterisks() {
        let tree = InlineTextTree::from_markdown("_a_ __b__");

        assert_eq!(tree.visible_text(), "a b");
        assert_eq!(tree.serialize_markdown(), "*a* **b**");
    }

    /// 锁定 Issue #6：标识符内部的下划线必须是普通文本，不能跨越后续标识符形成斜体。
    #[test]
    fn intraword_underscores_stay_literal() {
        let markdown = "On success it returns PAM_SUCCESS, otherwise it returns PAM_AUTH_ERR, PAM_SERVICE_ERR, PAM_BUF_ERR or PAM_PERM_DENIED.";
        let tree = InlineTextTree::from_markdown(markdown);

        assert_eq!(tree.visible_text(), markdown);
        assert!(
            tree.render_cache()
                .spans()
                .iter()
                .all(|span| !span.style.italic && !span.style.bold),
            "intraword underscores must not create emphasis spans"
        );
    }

    /// 覆盖下划线分隔符的相邻边界，防止修复标识符时破坏合法强调或 Unicode 标点。
    #[test]
    fn underscore_emphasis_respects_word_and_punctuation_boundaries() {
        for markdown in ["foo_bar_baz", "foo__bar__baz", "变量_名称", "при_вет"] {
            let tree = InlineTextTree::from_markdown(markdown);
            assert_eq!(tree.visible_text(), markdown);
            assert!(
                tree.render_cache()
                    .spans()
                    .iter()
                    .all(|span| !span.style.italic && !span.style.bold),
                "input {markdown:?} must remain literal"
            );
        }

        let italic = InlineTextTree::from_markdown("(_word_) 「_强调_」");
        assert_eq!(italic.visible_text(), "(word) 「强调」");
        assert_eq!(italic.serialize_markdown(), "(*word*) 「*强调*」");

        let bold = InlineTextTree::from_markdown("__word__");
        assert_eq!(bold.visible_text(), "word");
        assert_eq!(bold.serialize_markdown(), "**word**");
    }

    #[test]
    fn emphasis_delimiters_surrounded_by_spaces_stay_literal() {
        let tree = InlineTextTree::from_markdown("* a * _ b _");

        assert_eq!(tree.visible_text(), "* a * _ b _");
        assert_eq!(tree.serialize_markdown(), "\\* a \\* \\_ b \\_");
    }

    #[test]
    fn preserves_unclosed_markers_as_literal_text() {
        let tree = InlineTextTree::from_markdown("1**234");

        assert_eq!(tree.visible_text(), "1**234");
        assert_eq!(tree.serialize_markdown(), "1\\*\\*234");
    }

    #[test]
    fn empty_emphasis_spans_stay_literal() {
        // `**`, `* *`, or `**word` must not be swallowed as an empty emphasis
        // span; the markers stay literal until a non-empty body is closed.
        for input in ["*", "**", "***", "****", "~~~~", "__"] {
            let tree = InlineTextTree::from_markdown(input);
            assert_eq!(tree.visible_text(), input, "input {input:?} lost markers");
        }

        let leading = InlineTextTree::from_markdown("**word");
        assert_eq!(leading.visible_text(), "**word");
        assert_eq!(leading.serialize_markdown(), "\\*\\*word");

        let trailing = InlineTextTree::from_markdown("**word*");
        assert_eq!(trailing.visible_text(), "**word*");
    }

    #[test]
    fn non_empty_emphasis_still_parses_after_empty_guard() {
        let bold = InlineTextTree::from_markdown("**word**");
        assert_eq!(bold.visible_text(), "word");
        assert_eq!(bold.serialize_markdown(), "**word**");

        let italic = InlineTextTree::from_markdown("*a*");
        assert_eq!(italic.visible_text(), "a");
        assert_eq!(italic.serialize_markdown(), "*a*");

        let single_char_bold = InlineTextTree::from_markdown("**a**");
        assert_eq!(single_char_bold.visible_text(), "a");
        assert_eq!(single_char_bold.serialize_markdown(), "**a**");

        let bold_italic = InlineTextTree::from_markdown("***x***");
        assert_eq!(bold_italic.visible_text(), "x");
        let spans = bold_italic.render_cache();
        assert!(
            spans
                .spans()
                .iter()
                .all(|span| span.style.bold && span.style.italic)
        );
    }

    #[test]
    fn unclosed_multichar_opener_stays_fully_literal() {
        // While typing `**bold**`, the intermediate `**bold*` must stay literal;
        // otherwise the second `*` opens an italic span and the bold is lost.
        let partial = InlineTextTree::from_markdown("**bold*");
        assert_eq!(partial.visible_text(), "**bold*");
        assert!(
            partial
                .render_cache()
                .spans()
                .iter()
                .all(|span| !span.style.italic && !span.style.bold),
            "`**bold*` must be plain literal, not italic"
        );

        // The completed marker still resolves to bold (not italic).
        let complete = InlineTextTree::from_markdown("**bold**");
        assert_eq!(complete.visible_text(), "bold");
        assert!(
            complete
                .render_cache()
                .spans()
                .iter()
                .all(|span| span.style.bold && !span.style.italic),
            "`**bold**` must be bold, not italic"
        );

        // A genuine single-`*` italic opener is unaffected by the multi-char rule.
        let italic = InlineTextTree::from_markdown("*word*");
        assert_eq!(italic.visible_text(), "word");
        assert!(
            italic
                .render_cache()
                .spans()
                .iter()
                .all(|span| span.style.italic && !span.style.bold),
            "`*word*` must stay italic"
        );

        // Other unclosed multi-char openers stay literal as a unit too.
        for input in ["__bold_", "~~strike~"] {
            let tree = InlineTextTree::from_markdown(input);
            assert_eq!(tree.visible_text(), input, "input {input:?} lost markers");
            assert!(
                tree.render_cache()
                    .spans()
                    .iter()
                    .all(|span| !span.style.italic
                        && !span.style.bold
                        && !span.style.strikethrough),
                "input {input:?} should be plain literal"
            );
        }
    }

    #[test]
    fn empty_code_span_is_unaffected_by_emphasis_guard() {
        // The empty-emphasis guard must not touch code spans. `*` inside a code
        // span stays literal and the span round-trips.
        let tree = InlineTextTree::from_markdown("`*`");
        assert_eq!(tree.visible_text(), "*");
        assert_eq!(tree.serialize_markdown(), "`*`");
    }

    #[test]
    fn preserves_escaped_marker_sequences_as_literal_text() {
        let tree = InlineTextTree::from_markdown("\\*\\*\\<u>text\\</u>\\\\");

        assert_eq!(tree.visible_text(), "**<u>text</u>\\");
        assert_eq!(tree.serialize_markdown(), "\\*\\*\\<u>text\\</u>\\\\");
    }

    #[test]
    fn preserves_tibetan_spaces_through_inline_round_trip() {
        let markdown = "༄༅།།དཔལ་ལྡན་རྩ་བའི་བླ་མ་རིན་པོ་ཆེ།། བདག་གི་སྤྱི་བོར་པདྨའི་གདན་བཞུགས་ནས།། ";
        let tree = InlineTextTree::from_markdown(markdown);
        let serialized = tree.serialize_markdown();

        assert_eq!(tree.visible_text(), markdown);
        assert!(tree.visible_text().contains("།། བདག"));
        assert!(tree.visible_text().ends_with(' '));
        assert_eq!(serialized, markdown);
        assert_eq!(
            InlineTextTree::from_markdown(&serialized).visible_text(),
            markdown
        );
    }

    #[test]
    fn preserves_chinese_spaces_through_inline_round_trip() {
        let markdown = "中文 文本 ";
        let tree = InlineTextTree::from_markdown(markdown);

        assert_eq!(tree.visible_text(), markdown);
        assert_eq!(tree.serialize_markdown(), markdown);
    }

    #[test]
    fn toggle_style_operates_on_selected_slice_only() {
        let mut tree = InlineTextTree::plain("123");
        assert!(tree.toggle_bold(1..3));
        assert_eq!(tree.serialize_markdown(), "1**23**");

        assert!(tree.toggle_bold(2..3));
        assert_eq!(tree.serialize_markdown(), "1**2**3");
    }

    #[test]
    fn replaces_visible_range_and_normalizes_manual_markdown_input() {
        let tree = InlineTextTree::plain(String::new());
        let result =
            tree.replace_visible_range(0..0, "**bold**", InlineInsertionAttributes::default());

        assert_eq!(result.tree.visible_text(), "bold");
        assert_eq!(result.map_offset(8), 4);
        assert_eq!(result.tree.serialize_markdown(), "**bold**");
    }
