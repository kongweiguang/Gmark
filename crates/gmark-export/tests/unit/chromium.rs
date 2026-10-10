// @author kongweiguang

use super::{cancellable_browser_work, export_runtime, launch_export_browser};

/// 有界等待使用同一截止时间，挂起的浏览器操作必须返回格式可识别的超时原因。
#[test]
fn stalled_browser_work_times_out() -> anyhow::Result<()> {
    let runtime = export_runtime("gmark-timeout-test", "create timeout test runtime")?;
    let error = runtime
        .block_on(cancellable_browser_work(
            std::future::pending::<anyhow::Result<()>>(),
            &std::sync::atomic::AtomicBool::new(false),
            tokio::time::Instant::now() + std::time::Duration::from_millis(20),
            "PDF",
        ))
        .err()
        .ok_or_else(|| anyhow::anyhow!("stalled operation completed"))?;
    assert_eq!(
        error.to_string(),
        "PDF export timed out while waiting for Chromium"
    );
    Ok(())
}

/// 启动失败需要保留每个候选的路径和原因，避免把已找到但不可用的浏览器误报为未安装。
#[test]
fn failed_launch_reports_every_browser() -> anyhow::Result<()> {
    let root =
        std::env::temp_dir().join(format!("gmark-missing-browsers-{}", uuid::Uuid::new_v4()));
    let executables = [root.join("missing-chrome"), root.join("missing-edge")];
    let runtime = export_runtime("gmark-launch-test", "create launch test runtime")?;
    let error = runtime
        .block_on(launch_export_browser(
            "<p>test</p>",
            "PDF",
            &executables,
            &std::sync::atomic::AtomicBool::new(false),
            tokio::time::Instant::now() + std::time::Duration::from_secs(45),
        ))
        .err()
        .ok_or_else(|| anyhow::anyhow!("missing browsers unexpectedly launched"))?;
    let message = error.to_string();
    assert!(message.contains("PDF export"));
    assert!(message.contains("All detected browsers failed"));
    assert!(message.contains("missing-chrome"));
    assert!(message.contains("missing-edge"));
    assert!(!message.contains("Install Chrome"));
    Ok(())
}
