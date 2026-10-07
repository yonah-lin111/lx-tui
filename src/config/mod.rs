//! 运行时配置模型与默认值；暂不加载外部配置文件。

/// 运行时配置。
#[derive(Debug, Clone)]
pub struct Config {
    /// 侧栏展开宽度（列）。
    pub sidebar_width: u16,
    /// 侧栏展开最小宽度（列，对齐 herdr 默认下限）。
    pub sidebar_min_width: u16,
    /// 侧栏展开最大宽度（列，herdr 默认上限再放大）。
    pub sidebar_max_width: u16,
    /// 低于该宽度隐藏侧栏。
    pub narrow_width: u16,
    /// 最小可用宽度。
    pub min_width: u16,
    /// 最小可用高度。
    pub min_height: u16,
    /// 窗格最小宽度（含边框列）。
    pub min_pane_width: u16,
    /// 窗格最小高度（含边框行）。
    pub min_pane_height: u16,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            sidebar_width: 24,
            sidebar_min_width: 18,
            sidebar_max_width: 40,
            narrow_width: 80,
            min_width: 40,
            min_height: 10,
            min_pane_width: 10,
            min_pane_height: 3,
        }
    }
}
