//! `PluginMetadata` — descriptive information for plugin discovery
//! `PluginMetadata` — 用于插件发现的描述性信息

/// Plugin metadata exposed to the host for discovery and display.
/// 暴露给主机的插件元数据，用于发现和显示。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PluginMetadata {
    /// Unique plugin identifier (e.g. "com.rp2350-rs.led")
    /// 唯一插件标识符（如 "com.rp2350-rs.led"）
    pub id: String,

    /// Human-readable plugin name / 人类可读的插件名称
    pub name: String,

    /// Plugin version (semver) / 插件版本（语义版本）
    pub version: String,

    /// Brief description / 简短描述
    pub description: String,

    /// Plugin author / 插件作者
    pub author: String,

    /// ABI version this plugin was compiled against
    /// 此插件编译时使用的 ABI 版本
    pub abi_version: u32,

    /// Minimum host version required / 最低主机版本要求
    pub min_host_version: String,

    /// Whether this plugin has a GUI component
    /// 此插件是否有 GUI 组件
    pub has_ui: bool,

    /// Tags for categorization and search / 分类和搜索标签
    pub tags: Vec<String>,
}

impl PluginMetadata {
    /// Create a new metadata builder / 创建新的元数据构建器
    #[must_use]
    pub fn builder() -> PluginMetadataBuilder {
        PluginMetadataBuilder::default()
    }
}

/// Builder for `PluginMetadata`
/// `PluginMetadata` 构建器
#[derive(Default)]
pub struct PluginMetadataBuilder {
    id: String,
    name: String,
    version: String,
    description: String,
    author: String,
    abi_version: u32,
    min_host_version: String,
    has_ui: bool,
    tags: Vec<String>,
}

impl PluginMetadataBuilder {
    /// Set the plugin ID / 设置插件 ID
    #[must_use]
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    /// Set the display name / 设置显示名称
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Set the version / 设置版本
    #[must_use]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = version.into();
        self
    }

    /// Set the description / 设置描述
    #[must_use]
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.description = desc.into();
        self
    }

    /// Set the author / 设置作者
    #[must_use]
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.author = author.into();
        self
    }

    /// Set ABI version / 设置 ABI 版本
    #[must_use]
    pub fn abi_version(mut self, v: u32) -> Self {
        self.abi_version = v;
        self
    }

    /// Set minimum host version / 设置最低主机版本
    #[must_use]
    pub fn min_host_version(mut self, v: impl Into<String>) -> Self {
        self.min_host_version = v.into();
        self
    }

    /// Set whether this plugin has UI / 设置是否有 UI
    #[must_use]
    pub fn has_ui(mut self, v: bool) -> Self {
        self.has_ui = v;
        self
    }

    /// Add tags / 添加标签
    #[must_use]
    pub fn tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Build the metadata / 构建元数据
    #[must_use]
    pub fn build(self) -> PluginMetadata {
        PluginMetadata {
            id: self.id,
            name: self.name,
            version: self.version,
            description: self.description,
            author: self.author,
            abi_version: self.abi_version,
            min_host_version: self.min_host_version,
            has_ui: self.has_ui,
            tags: self.tags,
        }
    }
}
