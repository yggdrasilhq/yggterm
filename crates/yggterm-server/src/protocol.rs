use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const YGG_PROTOCOL_SCHEMA_VERSION: &str = "2026-03-26";
pub const YGG_LOADING_NOTIFICATION_AFTER_MS: u64 = 3_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YggOperationPriority {
    Interactive,
    Background,
    Passive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YggCachePolicy {
    RequireFresh,
    PreferFresh,
    PreferStaleThenRefresh,
    CacheOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YggSurface {
    App,
    Sidebar,
    Preview,
    Terminal,
    MetadataRail,
    Search,
    Notifications,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum YggTarget {
    App,
    ActiveSession,
    Session {
        session_path: String,
    },
    Terminal {
        session_path: String,
    },
    Preview {
        session_path: String,
    },
    RemoteMachine {
        machine_key: String,
    },
    Search {
        query: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YggRequestMeta {
    pub request_id: String,
    pub operation: String,
    pub target: YggTarget,
    pub surface: YggSurface,
    pub priority: YggOperationPriority,
    pub cache_policy: YggCachePolicy,
    #[serde(default)]
    pub stale_ok_after_ms: Option<u64>,
    #[serde(default = "default_loading_notice_after_ms")]
    pub notify_loading_after_ms: u64,
}

fn default_loading_notice_after_ms() -> u64 {
    YGG_LOADING_NOTIFICATION_AFTER_MS
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YggProgress {
    pub step: String,
    #[serde(default)]
    pub current: Option<u64>,
    #[serde(default)]
    pub total: Option<u64>,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum YggEventKind {
    Accepted,
    Loading,
    Progress,
    Result,
    Error,
    Invalidated,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct YggEventEnvelope {
    pub protocol_version: String,
    pub meta: YggRequestMeta,
    pub event: YggEventKind,
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    #[serde(default)]
    pub progress: Option<YggProgress>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub data: Option<Value>,
}

impl YggEventEnvelope {
    pub fn new(meta: YggRequestMeta, event: YggEventKind) -> Self {
        Self {
            protocol_version: YGG_PROTOCOL_SCHEMA_VERSION.to_string(),
            meta,
            event,
            elapsed_ms: None,
            progress: None,
            message: None,
            data: None,
        }
    }

    pub fn with_elapsed_ms(mut self, elapsed_ms: u64) -> Self {
        self.elapsed_ms = Some(elapsed_ms);
        self
    }

    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }

    pub fn with_progress(mut self, progress: YggProgress) -> Self {
        self.progress = Some(progress);
        self
    }

    pub fn with_data(mut self, data: Value) -> Self {
        self.data = Some(data);
        self
    }
}

impl YggRequestMeta {
    pub fn interactive(
        request_id: impl Into<String>,
        operation: impl Into<String>,
        surface: YggSurface,
        target: YggTarget,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            operation: operation.into(),
            target,
            surface,
            priority: YggOperationPriority::Interactive,
            cache_policy: YggCachePolicy::PreferStaleThenRefresh,
            stale_ok_after_ms: Some(500),
            notify_loading_after_ms: YGG_LOADING_NOTIFICATION_AFTER_MS,
        }
    }
}
