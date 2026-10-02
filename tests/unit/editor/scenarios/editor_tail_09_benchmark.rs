// @author kongweiguang

/// 在真实已存在的隔离文件上计时保存，固定编辑/绘制样本；测试平台的 draw 不代表系统呈现。
fn run_gpui_pipeline_benchmark(cx: &mut TestAppContext, target_mib: usize) {
    const INPUT_SAMPLES: usize = 30;
    const SCROLL_SAMPLES: usize = 30;
    const SAVE_SAMPLES: usize = 30;

    init_editor_test_app(cx);
    let source = mixed_projection_fixture(target_mib * 1024 * 1024);
    let save_dir = tempfile::tempdir().expect("benchmark document directory");
    let save_path = save_dir.path().join("sample.md");
    fs::write(&save_path, source.as_bytes()).expect("seed benchmark document");
    let rss_before_mib = current_process_rss_mib();
    let construction_started = Instant::now();
    let (editor, visual_cx) = cx.add_window_view({
        let source = source.clone();
        let path = save_path.clone();
        move |_window, cx| Editor::from_markdown(cx, source, Some(path))
    });
    let construction_ms = construction_started.elapsed().as_secs_f64() * 1_000.0;

    let first_draw_started = Instant::now();
    redraw(visual_cx);
    let first_draw_ms = first_draw_started.elapsed().as_secs_f64() * 1_000.0;
    let rss_after_first_draw_mib = current_process_rss_mib();
    let recovery_temp = tempfile::tempdir().expect("recovery tempdir");
    let recovery_journal =
        crate::recovery::RecoveryJournal::create(recovery_temp.path(), None, source.clone())
            .expect("recovery journal");
    editor.update(visual_cx, |editor, _cx| {
        editor.recovery_journal = Some(Arc::new(Mutex::new(recovery_journal)));
    });

    let mut input_mutation_samples = Vec::with_capacity(INPUT_SAMPLES);
    let mut input_next_draw_samples = Vec::with_capacity(INPUT_SAMPLES);
    let mut input_draw_samples = Vec::with_capacity(INPUT_SAMPLES);
    for _ in 0..INPUT_SAMPLES {
        let started = Instant::now();
        editor.update(visual_cx, |editor, cx| {
            let block = editor.document.first_root().expect("root block").clone();
            let end = block.read(cx).display_text().len();
            block.update(cx, |block, cx| {
                block.prepare_undo_capture(crate::components::UndoCaptureKind::CoalescibleText, cx);
                block.replace_text_in_visible_range(end..end, "x", None, false, cx);
            });
        });
        input_mutation_samples.push(started.elapsed().as_micros());
        let draw_started = Instant::now();
        redraw(visual_cx);
        input_next_draw_samples.push(draw_started.elapsed().as_micros());
        input_draw_samples.push(started.elapsed().as_micros());
    }

    let mut scroll_draw_samples = Vec::with_capacity(SCROLL_SAMPLES);
    for index in 0..SCROLL_SAMPLES {
        editor.update(visual_cx, |editor, _cx| {
            let max_y = f32::from(editor.scroll_handle.max_offset().height.max(px(0.0)));
            let ratio = if index % 2 == 0 { 0.25 } else { 0.75 };
            editor
                .scroll_handle
                .set_offset(point(px(0.0), px(-max_y * ratio)));
        });
        let started = Instant::now();
        redraw(visual_cx);
        scroll_draw_samples.push(started.elapsed().as_micros());
    }

    let mut save_samples = Vec::with_capacity(SAVE_SAMPLES);
    for _ in 0..SAVE_SAMPLES {
        let started = Instant::now();
        visual_cx.update(|window, cx| {
            editor.update(cx, |editor, cx| {
                assert!(editor.save_to_existing_path(&save_path, window, cx));
            });
        });
        save_samples.push(started.elapsed().as_micros());
    }
    let saved_bytes = fs::metadata(&save_path)
        .expect("benchmark save should create target")
        .len();
    let expected = editor.update(visual_cx, |editor, _cx| {
        editor.source_document.serialized_bytes()
    });
    assert_eq!(
        fs::read(&save_path).expect("read saved benchmark document"),
        expected
    );
    fs::remove_file(&save_path).expect("remove benchmark save");

    let (input_p50_ms, input_p95_ms, input_p99_ms) = benchmark_percentiles(&mut input_draw_samples);
    let (mutation_p50_ms, mutation_p95_ms, mutation_p99_ms) =
        benchmark_percentiles(&mut input_mutation_samples);
    let (next_draw_p50_ms, next_draw_p95_ms, next_draw_p99_ms) =
        benchmark_percentiles(&mut input_next_draw_samples);
    let (scroll_p50_ms, scroll_p95_ms, scroll_p99_ms) =
        benchmark_percentiles(&mut scroll_draw_samples);
    let (save_p50_ms, save_p95_ms, save_p99_ms) = benchmark_percentiles(&mut save_samples);
    println!(
        "gpui_pipeline_benchmark size_bytes={} construction_ms={construction_ms:.3} first_explicit_draw_ms={first_draw_ms:.3} input_mutation_p50_ms={mutation_p50_ms:.3} input_mutation_p95_ms={mutation_p95_ms:.3} input_mutation_p99_ms={mutation_p99_ms:.3} input_next_draw_p50_ms={next_draw_p50_ms:.3} input_next_draw_p95_ms={next_draw_p95_ms:.3} input_next_draw_p99_ms={next_draw_p99_ms:.3} input_to_draw_p50_ms={input_p50_ms:.3} input_to_draw_p95_ms={input_p95_ms:.3} input_to_draw_p99_ms={input_p99_ms:.3} scroll_draw_p50_ms={scroll_p50_ms:.3} scroll_draw_p95_ms={scroll_p95_ms:.3} scroll_draw_p99_ms={scroll_p99_ms:.3} save_total_p50_ms={save_p50_ms:.3} save_total_p95_ms={save_p95_ms:.3} save_total_p99_ms={save_p99_ms:.3} rss_before_mib={:.2} rss_after_first_draw_mib={:.2} saved_bytes={saved_bytes}",
        source.len(),
        rss_before_mib.unwrap_or(f64::NAN),
        rss_after_first_draw_mib.unwrap_or(f64::NAN),
    );
}

#[gpui::test]
#[ignore = "release-only 1 MiB GPUI pipeline benchmark; run explicitly"]
/// 普通文档使用同一固定样本量，保留手动 release 验证，避免日常回归混入优化构建耗时。
async fn gpui_pipeline_1mib_release_benchmark(cx: &mut TestAppContext) {
    run_gpui_pipeline_benchmark(cx, 1);
}

#[gpui::test]
#[ignore = "release-only 10 MiB GPUI pipeline benchmark; run explicitly"]
/// 长文复用同一流程，以实际磁盘内容核验计时保存，不能仅凭请求受理判为保存完成。
async fn gpui_pipeline_10mib_release_benchmark(cx: &mut TestAppContext) {
    run_gpui_pipeline_benchmark(cx, 10);
}
