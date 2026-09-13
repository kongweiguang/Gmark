// @author kongweiguang

//! GPUI 无关的偏好领域模型。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 缺失语言配置时使用的既有默认语言标识。
pub const DEFAULT_LANGUAGE_ID: &str = "en-US";

/// 启动时打开新文件还是最近文件。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StartupOpenPreference {
    /// 新建未命名文档。
    #[default]
    NewFile,
    /// 打开最近的现存文档。
    LastOpenedFile,
}

impl StartupOpenPreference {
    /// 返回稳定的 TOML 表示。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NewFile => "new_file",
            Self::LastOpenedFile => "last_opened_file",
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "last_opened_file" => Self::LastOpenedFile,
            _ => Self::NewFile,
        }
    }
}

/// 自动保存策略；崩溃恢复与其独立。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AutoSavePreference {
    /// 仅显式保存时写入 Markdown 文件。
    #[default]
    Off,
    /// 停止编辑一秒后保存无冲突的既有文件。
    AfterDelay,
}

impl AutoSavePreference {
    /// 返回稳定的 TOML 表示。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::AfterDelay => "after_delay",
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "after_delay" => Self::AfterDelay,
            _ => Self::Off,
        }
    }
}

/// 图片、附件和视频共用的插入资源存储策略。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ResourceInsertBehavior {
    /// 不复制资源。
    #[default]
    None,
    /// 复制到 Markdown 文档目录。
    CopyToDocumentFolder,
    /// 复制到 `assets` 目录。
    CopyToAssetsFolder,
    /// 复制到以 Markdown 文档命名的资源目录。
    CopyToNamedAssetsFolder,
}

impl ResourceInsertBehavior {
    /// 返回稳定的 TOML 表示。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::CopyToDocumentFolder => "copy_to_document_folder",
            Self::CopyToAssetsFolder => "copy_to_assets_folder",
            Self::CopyToNamedAssetsFolder => "copy_to_named_assets_folder",
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "copy_to_document_folder" => Self::CopyToDocumentFolder,
            "copy_to_assets_folder" => Self::CopyToAssetsFolder,
            "copy_to_named_assets_folder" => Self::CopyToNamedAssetsFolder,
            _ => Self::None,
        }
    }
}

/// 降级兼容使用的旧偏好类型名。
pub type ImagePasteBehavior = ResourceInsertBehavior;

/// 平台无关的主题明暗偏好。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemeAppearance {
    /// 深色主题。
    Dark,
    /// 浅色主题。
    Light,
    /// 由宿主平台决定。
    #[default]
    System,
}

impl ThemeAppearance {
    /// 返回稳定的 TOML 表示。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::System => "system",
        }
    }

    /// 解析稳定的 TOML 表示。
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "dark" => Some(Self::Dark),
            "light" => Some(Self::Light),
            "system" => Some(Self::System),
            _ => None,
        }
    }
}

/// 内建主题调色板。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ThemePalette {
    /// Xcode 风格调色板。
    #[default]
    Xcode,
    /// Fleet 风格调色板。
    Fleet,
    /// Obsidian 风格调色板。
    Obsidian,
    /// Claude 风格调色板。
    Claude,
}

impl ThemePalette {
    /// 返回稳定的 TOML 表示。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Xcode => "xcode",
            Self::Fleet => "fleet",
            Self::Obsidian => "obsidian",
            Self::Claude => "claude",
        }
    }

    /// 解析稳定的 TOML 表示。
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "xcode" => Some(Self::Xcode),
            "fleet" => Some(Self::Fleet),
            "obsidian" => Some(Self::Obsidian),
            "claude" => Some(Self::Claude),
            _ => None,
        }
    }
}

/// 用户对系统视觉无障碍设置的覆盖方式。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AccessibilityOverride {
    /// 跟随操作系统；平台无法可靠读取时使用安全默认值。
    #[default]
    System,
    /// 无论系统值如何都启用该能力。
    Enabled,
    /// 无论系统值如何都禁用该能力。
    Disabled,
}

impl AccessibilityOverride {
    /// 返回稳定的 TOML 表示。
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
        }
    }

    /// 解析稳定的 TOML 表示；非法值由调用方独立回退。
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "system" => Some(Self::System),
            "enabled" => Some(Self::Enabled),
            "disabled" => Some(Self::Disabled),
            _ => None,
        }
    }

    const fn resolve(self, system_value: bool) -> bool {
        match self {
            Self::System => system_value,
            Self::Enabled => true,
            Self::Disabled => false,
        }
    }
}

/// 持久化的视觉无障碍偏好。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VisualAccessibilityPreferences {
    /// 减少非必要位移、缩放和惯性动画。
    pub reduced_motion: AccessibilityOverride,
    /// 使用稳定实色替代半透明材质。
    pub reduced_transparency: AccessibilityOverride,
    /// 增强文字、边界和焦点环的区分度。
    pub high_contrast: AccessibilityOverride,
}

impl VisualAccessibilityPreferences {
    /// 以显式覆盖优先、系统设置次之的顺序解析最终值。
    #[must_use]
    pub const fn resolve(self, system: SystemVisualPreferences) -> ResolvedVisualPreferences {
        ResolvedVisualPreferences {
            reduced_motion: self.reduced_motion.resolve(system.reduced_motion),
            reduced_transparency: self
                .reduced_transparency
                .resolve(system.reduced_transparency),
            high_contrast: self.high_contrast.resolve(system.high_contrast),
        }
    }
}

/// 平台 adapter 读取到的视觉无障碍状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SystemVisualPreferences {
    pub reduced_motion: bool,
    pub reduced_transparency: bool,
    pub high_contrast: bool,
}

/// 渲染层消费的最终视觉无障碍状态。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResolvedVisualPreferences {
    pub reduced_motion: bool,
    pub reduced_transparency: bool,
    pub high_contrast: bool,
}

/// 状态栏中的自定义按钮。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusBarButton {
    /// 宿主使用的稳定按钮 ID。
    pub id: String,
    /// 用户可见标签。
    pub label: String,
    /// 交由宿主解析的操作 ID。
    pub action_id: String,
}

/// 状态栏可见性与组件开关。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusBarPreferences {
    /// 是否显示状态栏。
    pub enabled: bool,
    /// 是否显示字数。
    pub show_word_count: bool,
    /// 是否显示光标位置。
    pub show_cursor_position: bool,
    /// 是否显示侧栏开关。
    pub show_sidebar_toggle: bool,
    /// 是否显示模式切换开关。
    pub show_mode_switch: bool,
    /// 自定义状态栏按钮。
    pub custom_buttons: Vec<StatusBarButton>,
}

impl Default for StatusBarPreferences {
    fn default() -> Self {
        Self {
            enabled: true,
            show_word_count: true,
            show_cursor_position: true,
            show_sidebar_toggle: true,
            show_mode_switch: true,
            custom_buttons: Vec::new(),
        }
    }
}

/// 文档加载策略的用户覆盖值。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DocumentLoadingPreferences {
    /// 驻留文档上限（MiB）；非法值保留供用户修正。
    pub max_resident_mib: Option<u64>,
}

impl DocumentLoadingPreferences {
    const MIB_RANGE: std::ops::RangeInclusive<u64> = 1..=1_024;

    /// 转换为文档核心可直接使用的加载策略。
    #[must_use]
    pub fn policy(&self) -> gmark_document_core::LoadingPolicy {
        gmark_document_core::LoadingPolicy {
            max_resident_bytes: self
                .max_resident_mib
                .filter(|value| Self::MIB_RANGE.contains(value))
                .and_then(|mib| mib.checked_mul(1024 * 1024)),
            force_safe_source: false,
        }
    }

    /// 返回有效上限，非法覆盖值回退为核心默认值。
    #[must_use]
    pub fn effective_max_resident_mib(&self) -> u64 {
        self.max_resident_mib
            .filter(|value| Self::MIB_RANGE.contains(value))
            .unwrap_or(
                gmark_document_core::DEFAULT_LOADING_LIMITS.max_resident_bytes / (1024 * 1024),
            )
    }

    /// 判断配置是否包含可保留但不可生效的覆盖值。
    #[must_use]
    pub fn has_invalid_override(&self) -> bool {
        self.max_resident_mib
            .is_some_and(|value| !Self::MIB_RANGE.contains(&value))
    }
}

/// 持久化的快捷键覆盖；字符串由 Wave 2 的 UI adapter 解释。
pub type ShortcutConfig = BTreeMap<String, Vec<String>>;

/// 用户偏好领域模型。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppPreferences {
    /// 启动打开行为。
    pub startup_open: StartupOpenPreference,
    /// 是否自动检查更新。
    pub auto_check_updates: bool,
    /// 默认界面语言 ID。
    pub default_language_id: String,
    /// 主题明暗偏好。
    pub theme_appearance: ThemeAppearance,
    /// 主题调色板偏好。
    pub theme_palette: ThemePalette,
    /// 视觉无障碍覆盖偏好。
    pub visual_accessibility: VisualAccessibilityPreferences,
    /// 是否显示表格标题行。
    pub show_table_headers: bool,
    /// 工作区是否加载隐藏文件和目录。
    pub show_hidden_files: bool,
    /// 插入资源时的存储策略。
    pub image_paste_behavior: ImagePasteBehavior,
    /// 自动保存策略。
    pub auto_save: AutoSavePreference,
    /// 是否开启拼写检查。
    pub spell_check: bool,
    /// 是否自动补全括号。
    pub auto_pair_brackets: bool,
    /// 是否自动补全 Markdown 标记。
    pub auto_pair_markdown: bool,
    /// 是否开启代码折叠。
    pub code_folding: bool,
    /// 是否在保存时格式化。
    pub format_on_save: bool,
    /// 编辑器字体大小。
    pub editor_font_size: u8,
    /// 编辑器行高百分比。
    pub editor_line_height_percent: u16,
    /// 编辑器内容宽度。
    pub editor_content_width: u16,
    /// 编辑器字体族。
    pub editor_font_family: String,
    /// 是否显示标签栏操作。
    pub show_tab_bar_actions: bool,
    /// 最近使用的编辑命令。
    pub recent_editing_commands: Vec<String>,
    /// 用户快捷键覆盖。
    pub keybindings: ShortcutConfig,
    /// 状态栏偏好。
    pub status_bar: StatusBarPreferences,
    /// 文档加载偏好。
    pub document_loading: DocumentLoadingPreferences,
}

/// Wave 2 adapter 可使用的简短领域别名。
pub type Preferences = AppPreferences;

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            startup_open: StartupOpenPreference::NewFile,
            auto_check_updates: true,
            default_language_id: DEFAULT_LANGUAGE_ID.into(),
            theme_appearance: ThemeAppearance::System,
            theme_palette: ThemePalette::Xcode,
            visual_accessibility: VisualAccessibilityPreferences::default(),
            show_table_headers: true,
            show_hidden_files: false,
            image_paste_behavior: ImagePasteBehavior::None,
            auto_save: AutoSavePreference::Off,
            spell_check: true,
            auto_pair_brackets: true,
            auto_pair_markdown: true,
            code_folding: true,
            format_on_save: false,
            editor_font_size: 16,
            editor_line_height_percent: 160,
            editor_content_width: 1200,
            editor_font_family: String::new(),
            show_tab_bar_actions: false,
            recent_editing_commands: Vec::new(),
            keybindings: ShortcutConfig::new(),
            status_bar: StatusBarPreferences::default(),
            document_loading: DocumentLoadingPreferences::default(),
        }
    }
}

impl AppPreferences {
    /// 返回资源插入策略的现代名称。
    #[must_use]
    pub const fn resource_insert_behavior(&self) -> ResourceInsertBehavior {
        self.image_paste_behavior
    }
}
