// @author kongweiguang

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};

use gmark_export::{ExportTheme, render_pdf, render_pdf_cancellable, render_png};

/// 只比较本次测试新增的配置目录，不清理可能属于其他应用或并行任务的临时数据。
fn browser_profiles() -> anyhow::Result<HashSet<std::path::PathBuf>> {
    std::fs::read_dir(std::env::temp_dir())?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("gmark-chromium-profile-")
        })
        .map(|entry| Ok(entry.path()))
        .collect()
}

/// 真实浏览器加载本机图片后触发取消，覆盖已启动进程的收尾；测试串行运行以隔离目录观察。
#[test]
#[ignore = "requires an installed browser; run with --test-threads=1"]
fn installed_browser_cancellation_cleans_profile() -> anyhow::Result<()> {
    let before = browser_profiles()?;
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let address = listener.local_addr()?;
    let cancelled = std::sync::Arc::new(AtomicBool::new(false));
    let server_cancelled = std::sync::Arc::clone(&cancelled);
    let server = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Ok((_stream, _)) = listener.accept() {
                server_cancelled.store(true, Ordering::Release);
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        false
    });
    let result = render_pdf_cancellable(
        &format!("# 取消验收\n\n![本机验收图片](http://{address}/wait.png)"),
        &ExportTheme::default(),
        "取消验收",
        None,
        cancelled.as_ref(),
    );
    assert!(
        server
            .join()
            .map_err(|_| anyhow::anyhow!("image server stopped"))?
    );
    assert_eq!(
        result.err().map(|error| error.to_string()).as_deref(),
        Some("export cancelled")
    );
    assert!(
        browser_profiles()?.difference(&before).next().is_none(),
        "cancelled browser left a profile"
    );
    Ok(())
}

/// 子进程单独设置 CHROME，避免并行测试修改全局环境，并覆盖首选浏览器提前退出后的导出恢复。
#[cfg(windows)]
#[test]
#[ignore = "requires an installed Chromium-compatible browser"]
fn installed_browser_recovers_from_early_exit() -> anyhow::Result<()> {
    let test_executable = std::env::current_exe()?;
    let browser_stub = std::env::temp_dir().join(format!(
        "gmark-browser-exit-zero-{}.cmd",
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&browser_stub, "@exit /b 0\r\n")?;
    let output = std::process::Command::new(test_executable)
        .env("CHROME", &browser_stub)
        .args([
            "--ignored",
            "installed_browser_exports_",
            "--test-threads=1",
            "--nocapture",
        ])
        .output();
    let cleanup = std::fs::remove_file(&browser_stub);
    let output = output?;
    cleanup?;
    assert!(
        output.status.success(),
        "browser recovery failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

/// 浏览器属于外部依赖，显式运行此测试时必须真正生成 PDF，不能把启动失败视为通过。
#[test]
#[ignore = "requires an installed Chromium-compatible browser"]
fn installed_browser_exports_pdf() -> anyhow::Result<()> {
    let before = browser_profiles()?;
    let pdf = render_pdf(
        "# 中文 PDF 导出\n\n| 希望提供的内容 | 说明 |\n|---|---|\n| 患者 ID、姓名 | 表格内容 |\n\n$$\nx^2 + y^2 = 1\n$$",
        &ExportTheme::default(),
        "PDF 导出回归",
        None,
    )?;
    assert!(pdf.starts_with(b"%PDF-"));
    assert!(pdf.len() > 1_000);
    assert!(
        browser_profiles()?.difference(&before).next().is_none(),
        "completed browser left a profile"
    );
    Ok(())
}

/// 短文档不应被固定视口补成长图；长文档仍须完整截取，避免裁白修复截断正文。
#[test]
#[ignore = "requires an installed Chromium-compatible browser"]
fn installed_browser_exports_png() -> anyhow::Result<()> {
    let png = render_png(
        "# 中文 PNG 导出\n\n导出回归。",
        &ExportTheme::default(),
        "PNG 导出回归",
        None,
    )?;
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert!(png.len() > 1_000);
    let height = u32::from_be_bytes(png[20..24].try_into()?);
    assert!(
        height < 600,
        "short document contains viewport padding: {height}"
    );
    let long_png = render_png(
        &"完整的长文档内容。\n\n".repeat(100),
        &ExportTheme::default(),
        "长图导出回归",
        None,
    )?;
    let long_height = u32::from_be_bytes(long_png[20..24].try_into()?);
    assert!(
        long_height > 1_600,
        "long document was clipped: {long_height}"
    );
    Ok(())
}
