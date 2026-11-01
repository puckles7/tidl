//! Background loop that applies each source's "archive completed items older
//! than N days" retention rule automatically (backlog #135).
//!
//! The Done settings tab already applies a scope's rule immediately on "Save &
//! apply"; this loop keeps applying it as time passes, so a completed item
//! leaves its source once it is old enough without the user re-saving. Each
//! tick delegates to `tidl_web::api::sweep_due_done_items`, which reconciles the
//! owner's index and reuses the same strip+archive engine as the manual purge.

use std::path::PathBuf;
use std::time::Duration;

use sqlx::SqlitePool;
use tracing::{error, info};

use tidl_web::ssr_types::{GitCommitPusher, MarkdownSyncTrigger};

/// How often to look for scopes whose retention age has elapsed. Retention is
/// expressed in whole days, so an hourly tick is comfortably responsive.
const TICK_SECS: u64 = 60 * 60;

pub async fn run_done_sweep_loop(
    pool: SqlitePool,
    data_dir: PathBuf,
    git_pusher: GitCommitPusher,
    md_trigger: MarkdownSyncTrigger,
) {
    loop {
        match tidl_web::api::sweep_due_done_items(&pool, &data_dir, &git_pusher, &md_trigger).await
        {
            Ok(0) => {}
            Ok(n) => info!(
                archived = n,
                "done_sweep: archived completed items past retention"
            ),
            Err(e) => error!("done_sweep: sweep failed: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(TICK_SECS)).await;
    }
}
