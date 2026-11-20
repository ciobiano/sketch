//! Live mentions from the local comment composer. Each bridge keeps its own cursor so a
//! second listener or a tool call between notifications cannot consume another client's event.
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, time::Duration};
use tokio::sync::watch;

pub const WAIT_METHOD: &str = "sketch/waitForCommentMention";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mention {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(rename = "commentThreadId")]
    pub thread_id: String,
    pub message_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct Batch {
    pub cursor: String,
    pub mentions: Vec<Mention>,
}

struct Recent {
    initial_cursor: String,
    events: VecDeque<Mention>,
}

impl Recent {
    fn latest(&self) -> &str {
        self.events
            .back()
            .map_or(&self.initial_cursor, |m| &m.message_id)
    }

    fn after(&self, cursor: Option<&str>) -> Batch {
        // A new listener starts at the tip. An existing listener that fell behind can
        // still receive every event retained since it lost its cursor.
        let start = match cursor {
            None => self.events.len(),
            Some(id) => self
                .events
                .iter()
                .position(|m| m.message_id == id)
                .map_or(0, |i| i + 1),
        };
        Batch {
            cursor: self.latest().to_owned(),
            mentions: self.events.iter().skip(start).cloned().collect(),
        }
    }
}

#[derive(Clone)]
pub struct Mentions(watch::Sender<Recent>);

impl Default for Mentions {
    fn default() -> Self {
        Self(
            watch::channel(Recent {
                initial_cursor: crate::project::new_id(),
                events: VecDeque::new(),
            })
            .0,
        )
    }
}

impl Mentions {
    pub fn publish(&self, mention: Mention) {
        self.0.send_modify(|recent| {
            if recent.events.len() == 64 {
                recent.events.pop_front();
            }
            recent.events.push_back(mention);
        });
    }

    pub(super) async fn wait(&self, cursor: Option<&str>) -> Batch {
        let mut rx = self.0.subscribe();
        let _ = tokio::time::timeout(
            Duration::from_secs(20),
            rx.wait_for(|s| Some(s.latest()) != cursor),
        )
        .await;
        rx.borrow().after(cursor)
    }
}
