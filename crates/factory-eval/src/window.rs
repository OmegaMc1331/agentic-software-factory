use chrono::{DateTime, Duration, Utc};

/// The supported evaluation time windows. Deliberately a compact fixed set —
/// not a general analytics query language.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceWindow {
    #[default]
    AllTime,
    Last30Days,
    Last7Days,
}

impl PerformanceWindow {
    pub fn as_str(self) -> &'static str {
        match self {
            PerformanceWindow::AllTime => "all_time",
            PerformanceWindow::Last30Days => "last_30_days",
            PerformanceWindow::Last7Days => "last_7_days",
        }
    }

    /// Human label, e.g. for a filter dropdown.
    pub fn label(self) -> &'static str {
        match self {
            PerformanceWindow::AllTime => "All time",
            PerformanceWindow::Last30Days => "Last 30 days",
            PerformanceWindow::Last7Days => "Last 7 days",
        }
    }

    /// Parses the API form: `all`, `30d`, `7d` (case-insensitive).
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "all" | "all_time" => Some(PerformanceWindow::AllTime),
            "30d" | "last_30_days" => Some(PerformanceWindow::Last30Days),
            "7d" | "last_7_days" => Some(PerformanceWindow::Last7Days),
            _ => None,
        }
    }

    /// The inclusive lower bound of the window, or `None` for all time.
    pub fn since(self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            PerformanceWindow::AllTime => None,
            PerformanceWindow::Last30Days => Some(now - Duration::days(30)),
            PerformanceWindow::Last7Days => Some(now - Duration::days(7)),
        }
    }
}
