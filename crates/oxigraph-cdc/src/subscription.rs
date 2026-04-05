use dashmap::DashMap;
use std::time::Instant;

/// Channel type for a subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ChannelType {
    WebSocketChannel2023,
    StreamingHTTPChannel2023,
}

/// An active notification subscription.
#[derive(Debug, Clone)]
pub struct Subscription {
    pub id: String,
    pub topic: String,
    pub channel_type: ChannelType,
    pub created: Instant,
    pub last_event_id: Option<u64>,
}

/// Thread-safe registry of active subscriptions.
pub struct SubscriptionRegistry {
    subscriptions: DashMap<String, Subscription>,
}

impl SubscriptionRegistry {
    pub fn new() -> Self {
        Self {
            subscriptions: DashMap::new(),
        }
    }

    pub fn insert(&self, sub: Subscription) {
        self.subscriptions.insert(sub.id.clone(), sub);
    }

    pub fn get(&self, id: &str) -> Option<Subscription> {
        self.subscriptions.get(id).map(|s| s.clone())
    }

    pub fn remove(&self, id: &str) -> Option<Subscription> {
        self.subscriptions.remove(id).map(|(_, s)| s)
    }

    pub fn count(&self) -> usize {
        self.subscriptions.len()
    }
}
