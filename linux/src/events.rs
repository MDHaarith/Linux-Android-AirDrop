//! Event bus pushing state changes to the desktop UI (over WebSocket) and the tray.

use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::broadcast;

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<Arc<Event>>,
}

#[derive(Debug)]
pub struct Event {
    pub name: &'static str,
    pub data: Value,
}

impl Event {
    pub fn to_json_string(&self) -> String {
        json!({ "event": self.name, "data": self.data }).to_string()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self {
            tx: broadcast::channel(1024).0,
        }
    }
}

impl EventBus {
    pub fn emit(&self, name: &'static str, data: Value) {
        // No receivers is fine: nobody is watching.
        let _ = self.tx.send(Arc::new(Event { name, data }));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Arc<Event>> {
        self.tx.subscribe()
    }
}
