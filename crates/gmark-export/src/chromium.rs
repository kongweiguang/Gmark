// @author kongweiguang

//! Chromium-backed PDF and PNG rendering with bounded waits and RAII cleanup.

use std::fs;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use anyhow::{Context as _, anyhow};
use chromiumoxide::Handler;
use chromiumoxide::browser::{Browser, BrowserConfig};
use chromiumoxide::cdp::browser_protocol::emulation::SetDeviceMetricsOverrideParams;
use chromiumoxide::cdp::browser_protocol::page::{CaptureScreenshotFormat, PrintToPdfParams};
use chromiumoxide::detection::{DetectionOptions, default_executable};
use chromiumoxide::page::ScreenshotParams;
use futures::StreamExt;
use tokio::time::Instant;
use uuid::Uuid;

use crate::ExportCancellation;
use crate::ExportTheme;
use crate::html::{render_chromium_pdf_html_with_base_dir, render_html_with_base_dir};

const VIEWPORT_WIDTH: u32 = 1280;
const VIEWPORT_HEIGHT: u32 = 1600;
const CHROMIUM_TIMEOUT: Duration = Duration::from_secs(45);

/// Renders a full-page PNG through a local Chromium-compatible browser.
pub fn render_png(
    markdown: &str,
    theme: &ExportTheme,
    title: &str,
    base_path: Option<&Path>,
) -> anyhow::Result<Vec<u8>> {
    render_png_cancellable(markdown, theme, title, base_path, &AtomicBool::new(false))
}

/// 取消与超时只中断生成步骤；进程与临时目录收尾完成后再返回，避免 Windows 文件锁残留。
pub fn render_png_cancellable<C: ExportCancellation + ?Sized>(
    markdown: &str,
    theme: &ExportTheme,
    title: &str,
    base_path: Option<&Path>,
    cancelled: &C,
) -> anyhow::Result<Vec<u8>> {
    if cancelled.is_cancelled() {
        return Err(anyhow!("export cancelled"));
    }
    let runtime = export_runtime(
        "gmark-image-export",
        "failed to create image export runtime",
    )?;
    runtime.block_on(render_png_async(
        markdown, theme, title, base_path, cancelled,
    ))
}

/// Renders PDF bytes through Chromium's print pipeline.
pub fn render_pdf(
    markdown: &str,
    theme: &ExportTheme,
    title: &str,
    base_path: Option<&Path>,
) -> anyhow::Result<Vec<u8>> {
    render_pdf_cancellable(markdown, theme, title, base_path, &AtomicBool::new(false))
}

/// PDF 与 PNG 共用有界取消与收尾策略，保证打印失败不会遗留浏览器配置目录。
pub fn render_pdf_cancellable<C: ExportCancellation + ?Sized>(
    markdown: &str,
    theme: &ExportTheme,
    title: &str,
    base_path: Option<&Path>,
    cancelled: &C,
) -> anyhow::Result<Vec<u8>> {
    if cancelled.is_cancelled() {
        return Err(anyhow!("export cancelled"));
    }
    let runtime = export_runtime("gmark-pdf-export", "failed to create PDF export runtime")?;
    runtime.block_on(render_pdf_async(
        markdown, theme, title, base_path, cancelled,
    ))
}

/// Screenshot parameters used by the public PNG export.
pub fn png_screenshot_params() -> ScreenshotParams {
    ScreenshotParams::builder()
        .format(CaptureScreenshotFormat::Png)
        .full_page(true)
        .build()
}

/// Chromium print settings shared by all PDF exports.
pub fn chromium_pdf_params() -> PrintToPdfParams {
    PrintToPdfParams {
        print_background: Some(true),
        prefer_css_page_size: Some(true),
        paper_width: Some(8.27),
        paper_height: Some(11.69),
        margin_top: Some(0.0),
        margin_bottom: Some(0.0),
        margin_left: Some(0.0),
        margin_right: Some(0.0),
        ..Default::default()
    }
}

/// Produces a correctly encoded `file:` URL for a local temporary HTML file.
pub fn file_url_from_path(path: &Path) -> anyhow::Result<url::Url> {
    url::Url::from_file_path(path)
        .map_err(|_| anyhow!("failed to convert '{}' to a file URL", path.display()))
}

/// 临时文件先取得所有权以保证浏览器先释放；截图使用最小视口高度，让全页尺寸取自正文而非窗口留白。
async fn render_png_async<C: ExportCancellation + ?Sized>(
    markdown: &str,
    theme: &ExportTheme,
    title: &str,
    base_path: Option<&Path>,
    cancelled: &C,
) -> anyhow::Result<Vec<u8>> {
    let deadline = Instant::now() + CHROMIUM_TIMEOUT;
    let html = render_html_with_base_dir(markdown, theme, title, base_path);
    let executables = browser_executables()?;
    let (temp, mut browser, mut handler) =
        launch_export_browser(&html, "image", &executables, cancelled, deadline).await?;
    let handler_task = tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            if event.is_err() {
                break;
            }
        }
    });
    let result = cancellable_browser_work(
        async {
            let file_url = file_url_from_path(&temp.html_path)?;
            let page = browser
                .new_page(file_url.as_str())
                .await
                .context("failed to open export HTML in Chromium")?;
            page.wait_for_navigation()
                .await
                .context("Chromium did not finish loading export HTML")?;
            page.execute(SetDeviceMetricsOverrideParams::new(
                VIEWPORT_WIDTH,
                1,
                1.0,
                false,
            ))
            .await
            .context("Chromium failed to set the image export viewport")?;
            page.screenshot(png_screenshot_params())
                .await
                .context("Chromium failed to capture export HTML as PNG")
        },
        cancelled,
        deadline,
        "image",
    )
    .await;
    shutdown_export_browser(&mut browser, handler_task).await;
    drop(browser);
    temp.cleanup().await;
    result
}

/// 打印沿用文档的 CSS 分页，并按浏览器、临时文件的释放顺序保留失败与取消后的清理边界。
async fn render_pdf_async<C: ExportCancellation + ?Sized>(
    markdown: &str,
    theme: &ExportTheme,
    title: &str,
    base_path: Option<&Path>,
    cancelled: &C,
) -> anyhow::Result<Vec<u8>> {
    let deadline = Instant::now() + CHROMIUM_TIMEOUT;
    let html = render_chromium_pdf_html_with_base_dir(markdown, theme, title, base_path);
    let executables = browser_executables()?;
    let (temp, mut browser, mut handler) =
        launch_export_browser(&html, "PDF", &executables, cancelled, deadline).await?;
    let handler_task = tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            if event.is_err() {
                break;
            }
        }
    });
    let result = cancellable_browser_work(
        async {
            let file_url = file_url_from_path(&temp.html_path)?;
            let page = browser
                .new_page(file_url.as_str())
                .await
                .context("failed to open export HTML in Chromium")?;
            page.wait_for_navigation()
                .await
                .context("Chromium did not finish loading export HTML")?;
            page.pdf(chromium_pdf_params())
                .await
                .context("Chromium failed to print export HTML to PDF")
        },
        cancelled,
        deadline,
        "PDF",
    )
    .await;
    shutdown_export_browser(&mut browser, handler_task).await;
    drop(browser);
    temp.cleanup().await;
    result
}

/// 只取消可中断的浏览器操作，不丢弃拥有进程与目录的外层任务，保证每种结果都经过收尾。
async fn cancellable_browser_work<T, C: ExportCancellation + ?Sized>(
    work: impl Future<Output = anyhow::Result<T>>,
    cancelled: &C,
    deadline: Instant,
    kind: &str,
) -> anyhow::Result<T> {
    tokio::select! {
        result = tokio::time::timeout_at(deadline, work) => result
            .map_err(|_| anyhow!("{kind} export timed out while waiting for Chromium"))?,
        () = wait_for_export_cancel(cancelled) => Err(anyhow!("export cancelled")),
    }
}

/// CDP 关闭不代表进程已退出；有限等待后强制收尾，再结束 handler，避免 Windows 文件锁残留。
async fn shutdown_export_browser(browser: &mut Browser, handler: tokio::task::JoinHandle<()>) {
    let closed = tokio::time::timeout(Duration::from_secs(2), async {
        browser.close().await?;
        browser.wait().await?;
        Ok::<(), anyhow::Error>(())
    })
    .await;
    if !matches!(closed, Ok(Ok(()))) {
        let _ = tokio::time::timeout(Duration::from_secs(2), browser.kill()).await;
    }
    handler.abort();
    let _ = handler.await;
}

fn export_runtime(
    name: &str,
    error_context: &'static str,
) -> anyhow::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name(name)
        .build()
        .context(error_context)
}

/// 每次启动显式选定可执行文件和独立配置目录，避免重试复用失败进程的用户数据。
fn browser_config(temp: &ChromiumTempFiles, executable: &Path) -> anyhow::Result<BrowserConfig> {
    BrowserConfig::builder()
        .new_headless_mode()
        .window_size(VIEWPORT_WIDTH, VIEWPORT_HEIGHT)
        .user_data_dir(temp.user_data_dir.clone())
        .chrome_executable(executable)
        .build()
        .map_err(|error| anyhow!("failed to build Chromium browser config: {error}"))
}

/// 保留 CHROME 和 SDK 自动探测结果的优先级；Windows 标准安装位置补充可恢复的备选浏览器。
fn browser_executables() -> anyhow::Result<Vec<PathBuf>> {
    let candidates = default_executable(DetectionOptions::default())
        .ok()
        .into_iter()
        .collect::<Vec<_>>();
    #[cfg(windows)]
    let candidates = {
        let mut candidates = candidates;
        for root in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"]
            .into_iter()
            .filter_map(std::env::var_os)
        {
            for relative in [
                "Microsoft/Edge/Application/msedge.exe",
                "Google/Chrome/Application/chrome.exe",
                "Chromium/Application/chrome.exe",
            ] {
                let executable = PathBuf::from(&root).join(relative);
                if executable.is_file() && !candidates.contains(&executable) {
                    candidates.push(executable);
                }
            }
        }
        candidates
    };
    if candidates.is_empty() {
        return Err(anyhow!(
            "No Chromium-compatible browser was found. Install Chrome, Chromium, or Edge, or set the CHROME environment variable to the browser executable path"
        ));
    }
    Ok(candidates)
}

/// 首选浏览器可能在提供调试地址前退出；逐个尝试本地备选，并用全新的临时目录隔离不同浏览器。
async fn launch_export_browser<C: ExportCancellation + ?Sized>(
    html: &str,
    export_kind: &str,
    executables: &[PathBuf],
    cancelled: &C,
    deadline: Instant,
) -> anyhow::Result<(ChromiumTempFiles, Browser, Handler)> {
    let mut errors = Vec::new();
    for executable in executables {
        let temp = ChromiumTempFiles::create(
            "gmark-export",
            "gmark-chromium-profile",
            "failed to create Chromium profile",
            html,
        )?;
        let config = browser_config(&temp, executable)?;
        let launched = cancellable_browser_work(
            async { Browser::launch(config).await.map_err(anyhow::Error::from) },
            cancelled,
            deadline,
            export_kind,
        )
        .await;
        match launched {
            Ok((browser, handler)) => return Ok((temp, browser, handler)),
            Err(error) => {
                temp.cleanup().await;
                if cancelled.is_cancelled() || Instant::now() >= deadline {
                    return Err(error);
                }
                errors.push(format!("{}: {error}", executable.display()));
            }
        }
    }
    Err(anyhow!(
        "failed to launch Chromium for {export_kind} export. All detected browsers failed:\n{}",
        errors.join("\n")
    ))
}

async fn wait_for_export_cancel<C: ExportCancellation + ?Sized>(cancelled: &C) {
    while !cancelled.is_cancelled() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

struct ChromiumTempFiles {
    html_path: PathBuf,
    user_data_dir: PathBuf,
}

impl ChromiumTempFiles {
    fn create(
        html_prefix: &str,
        profile_prefix: &str,
        profile_error_context: &'static str,
        html: &str,
    ) -> anyhow::Result<Self> {
        let id = Uuid::new_v4();
        let html_path = std::env::temp_dir().join(format!("{html_prefix}-{id}.html"));
        let user_data_dir = std::env::temp_dir().join(format!("{profile_prefix}-{id}"));
        fs::write(&html_path, html)
            .with_context(|| format!("failed to write temporary HTML '{}'", html_path.display()))?;
        if let Err(error) = fs::create_dir_all(&user_data_dir) {
            let _ = fs::remove_file(&html_path);
            return Err(error)
                .with_context(|| format!("{profile_error_context} '{}'", user_data_dir.display()));
        }
        Ok(Self {
            html_path,
            user_data_dir,
        })
    }

    /// 浏览器子进程的文件锁可能稍晚释放；仅重试本次 UUID 目录，失败进入日志且不无限等待。
    async fn cleanup(&self) {
        let _ = fs::remove_file(&self.html_path);
        for attempt in 0..10 {
            match fs::remove_dir_all(&self.user_data_dir) {
                Ok(()) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) if attempt == 9 => {
                    eprintln!(
                        "failed to clean export profile '{}': {error}",
                        self.user_data_dir.display()
                    );
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(100)).await,
            }
        }
    }
}

impl Drop for ChromiumTempFiles {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.html_path);
        let _ = fs::remove_dir_all(&self.user_data_dir);
    }
}

#[cfg(test)]
#[path = "../tests/unit/chromium.rs"]
mod tests;
