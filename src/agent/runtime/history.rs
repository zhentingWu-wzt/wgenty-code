//! Shared history store implementations.

use super::ports::HistoryStore;
use crate::api::ChatMessage;
use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::Mutex;

/// History backed by the same `Arc<Mutex<Vec<ChatMessage>>>` the TUI already uses.
#[derive(Clone)]
pub struct MutexHistoryStore {
    inner: Arc<Mutex<Vec<ChatMessage>>>,
}

impl MutexHistoryStore {
    pub fn new(inner: Arc<Mutex<Vec<ChatMessage>>>) -> Self {
        Self { inner }
    }

    pub fn handle(&self) -> Arc<Mutex<Vec<ChatMessage>>> {
        self.inner.clone()
    }
}

#[async_trait]
impl HistoryStore for MutexHistoryStore {
    async fn get(&self) -> Vec<ChatMessage> {
        self.inner.lock().await.clone()
    }

    async fn replace(&self, messages: Vec<ChatMessage>) {
        *self.inner.lock().await = messages;
    }

    async fn push(&self, message: ChatMessage) {
        self.inner.lock().await.push(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mutex_history_round_trip() {
        let store = MutexHistoryStore::new(Arc::new(Mutex::new(vec![ChatMessage::system("s")])));
        store.push(ChatMessage::user("hi")).await;
        let msgs = store.get().await;
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].role, "user");
        store.replace(vec![ChatMessage::system("reset")]).await;
        assert_eq!(store.get().await.len(), 1);
    }
}
