//! Push an SSE change notification to a calendar's owner *and* every one of
//! its grantees, not just whoever's action triggered the change.
//!
//! `tidl-web`'s `#[server]` functions have their own copy of this
//! (`notify_calendar_watchers` in `api/events.rs`) because the compilation
//! boundary (see CLAUDE.md §8) means this crate can't call into it. Mutation
//! paths that live entirely in `tidl-server` — background sync loops and
//! direct CalDAV writes — used to `change_tx.send` only the acting/owning
//! user, so a calendar's other grantees never got pushed a live update and
//! needed a manual refresh (backlog #59).

use std::sync::Arc;

use tidl_caldav::change::ChangeKind;
use tokio::sync::broadcast;

use crate::storage::CalendarStore;

pub async fn notify_calendar_watchers(
    storage: &Arc<dyn CalendarStore + Send + Sync>,
    change_tx: &broadcast::Sender<String>,
    owner_username: &str,
    calendar_name: &str,
    kind: ChangeKind,
) {
    let _ = change_tx.send(kind.to_msg(owner_username));
    if let Ok(grants) = storage
        .list_shares_for_calendar(owner_username, calendar_name)
        .await
    {
        for grant in grants {
            let _ = change_tx.send(kind.to_msg(&grant.grantee_username));
        }
    }
}
