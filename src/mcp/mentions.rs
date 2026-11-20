//! Live mentions from the local comment composer. Each bridge keeps its own cursor so a
//! second listener or a tool call between notifications cannot consume another client's event.
use crate::comments::{ANY_AGENT, Author};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};
use tokio::sync::watch;

pub const WAIT_METHOD: &str = "sketch/waitForCommentMention";

pub fn valid_listener_name(s: &str) -> bool {
    s.trim() == s
        && s.len() <= 128
        && s.chars().any(char::is_alphanumeric)
        && !s.chars().any(char::is_control)
        && !s.eq_ignore_ascii_case(ANY_AGENT)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Mention {
    #[serde(skip)]
    pub recipients: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    #[serde(rename = "commentThreadId")]
    pub thread_id: String,
    pub message_id: String,
    /// Comments the person sent along with this one, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<MentionRef>,
}

/// Another comment in the same send.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MentionRef {
    #[serde(rename = "commentThreadId")]
    pub thread_id: String,
    pub message_id: String,
}

#[derive(Serialize, Deserialize)]
pub struct Batch {
    #[serde(default)]
    pub recipient: Option<String>,
    pub cursor: String,
    pub mentions: Vec<Mention>,
}

struct Recent {
    initial_cursor: String,
    events: VecDeque<Mention>,
    listeners: BTreeMap<String, (Author, Instant)>,
}

impl Recent {
    fn latest(&self) -> &str {
        self.events
            .back()
            .map_or(&self.initial_cursor, |m| &m.message_id)
    }

    fn after(&self, cursor: Option<&str>, recipient: &str) -> Batch {
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
            recipient: Some(recipient.to_owned()),
            cursor: self.latest().to_owned(),
            mentions: self
                .events
                .iter()
                .skip(start)
                .filter(|m| {
                    m.recipients
                        .iter()
                        .any(|id| id == ANY_AGENT || id == recipient)
                })
                .cloned()
                .collect(),
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
                listeners: BTreeMap::new(),
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

    pub fn connected_agents(&self) -> Vec<Author> {
        let now = Instant::now();
        self.0
            .borrow()
            .listeners
            .values()
            .filter(|(_, until)| *until > now)
            .map(|(a, _)| a.clone())
            .collect()
    }

    pub(super) async fn wait(&self, cursor: Option<&str>, agent: Author) -> Batch {
        let id = agent.id.clone();
        let now = Instant::now();
        // The bridge renews its name with each 20-second poll. Crashed listeners expire.
        // Presence changes do not advance the comment cursor.
        let mut baseline = None;
        self.0.send_if_modified(|r| {
            r.listeners.retain(|_, (_, until)| *until > now);
            r.listeners
                .insert(id.clone(), (agent, now + Duration::from_secs(60)));
            if cursor.is_none() {
                baseline = Some(r.after(None, &id));
            }
            false
        });
        if let Some(batch) = baseline {
            return batch;
        }
        let mut rx = self.0.subscribe();
        let _ = tokio::time::timeout(
            Duration::from_secs(20),
            rx.wait_for(|s| Some(s.latest()) != cursor),
        )
        .await;
        rx.borrow().after(cursor, &id)
    }
}
