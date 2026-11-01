/// Which part of the data model changed, used to route SSE notifications to
/// only the resources that need to refetch.
///
/// Replaces the implicit `"<kind>:<username>"` string protocol that was
/// previously constructed and parsed independently in `api.rs`, `main.rs`,
/// and the sync loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    /// Only calendar metadata changed (name, colour, sync URL, …).
    Cals,
    /// Only calendar events / todos changed.
    Events,
    /// Both calendar metadata and events changed (e.g. calendar deleted).
    All,
}

impl ChangeKind {
    /// Format as a `"kind:username"` broadcast message.
    pub fn to_msg(self, username: &str) -> String {
        format!("{}:{}", self.as_str(), username)
    }

    /// Parse a `"kind:username"` broadcast message.
    ///
    /// Returns `(kind, username)`. Bare messages with no colon (legacy
    /// compatibility) are treated as `(All, whole_message)`.
    pub fn parse_msg(msg: &str) -> (Self, &str) {
        if let Some(pos) = msg.find(':') {
            let kind = match &msg[..pos] {
                "cals" => ChangeKind::Cals,
                "events" => ChangeKind::Events,
                _ => ChangeKind::All,
            };
            (kind, &msg[pos + 1..])
        } else {
            (ChangeKind::All, msg)
        }
    }

    /// The label sent to the browser as SSE event data.
    pub fn as_str(self) -> &'static str {
        match self {
            ChangeKind::Cals => "cals",
            ChangeKind::Events => "events",
            ChangeKind::All => "all",
        }
    }
}
