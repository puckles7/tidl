use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("git operation failed: {0}")]
    Git(#[from] git2::Error),
    #[error("no credentials available for git source")]
    NoCredentials,
    #[error("PAT file not found or unreadable: {0}")]
    PatFileRead(String),
    #[error("decryption failed")]
    Decrypt(String),
    #[error("push failed: {0}")]
    PushFailed(String),
    #[error("could not detect default branch: {0}")]
    DetectFailed(String),
    #[error("local clone has uncommitted changes; refusing to overwrite them")]
    DirtyWorktree,
    #[error("local clone has diverged from the remote and cannot be fast-forwarded")]
    NotFastForward,
}

/// Info needed for git operations on a single source.
pub struct GitSourceConfig {
    /// HTTPS remote URL — passed separately to clone/pull/push fns; stored for introspection
    #[allow(dead_code)]
    pub url: String,
    /// Branch to track (e.g. "main") — passed separately to clone/pull/push fns; stored for introspection
    #[allow(dead_code)]
    pub branch: String,
    /// Path to a plaintext PAT file (flake-configured sources)
    pub pat_file: Option<String>,
    /// base64(nonce || ciphertext) for ad-hoc sources
    pub pat_encrypted: Option<String>,
    /// 32-byte AES-256-GCM key for decrypting pat_encrypted (from AppState)
    pub enc_key: Option<Vec<u8>>,
}

impl GitSourceConfig {
    pub fn resolve_pat(&self) -> Result<String, GitError> {
        if let Some(ref path) = self.pat_file {
            let pat = std::fs::read_to_string(path)
                .map_err(|e| GitError::PatFileRead(format!("{path}: {e}")))?;
            return Ok(pat.trim().to_string());
        }
        if let Some(ref encrypted) = self.pat_encrypted {
            if let Some(ref key) = self.enc_key {
                return crate::crypto::decrypt_pat(encrypted, key)
                    .map_err(|e| GitError::Decrypt(e.to_string()));
            }
        }
        Err(GitError::NoCredentials)
    }
}

/// Clone `url` into `local_path` and check out `branch`. `local_path` must not already exist.
pub fn clone_repo(url: &str, local_path: &Path, branch: &str, pat: &str) -> Result<(), GitError> {
    let pat_owned = pat.to_owned();
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
        git2::Cred::userpass_plaintext("oauth2", &pat_owned)
    });

    let mut fo = git2::FetchOptions::new();
    fo.remote_callbacks(callbacks);
    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fo);
    let repo = builder.clone(url, local_path)?;
    checkout_configured_branch(&repo, branch)?;
    Ok(())
}

/// Ensure local HEAD tracks the source's configured branch, not whatever the
/// remote happened to check out by default.
///
/// `RepoBuilder::clone` always checks out the remote's default branch (its
/// HEAD symref). That frequently isn't the branch a source is configured
/// for — e.g. a Forgejo repo's default branch is "master" while the source
/// is set to sync "main", a real branch that just isn't the remote's HEAD.
/// Left alone, every later `commit_and_push` would push
/// `refs/heads/{branch}` against a local repo that has no such ref,
/// failing with "src refspec ... does not match any existing object" even
/// though the remote repo is not empty.
fn checkout_configured_branch(repo: &git2::Repository, branch: &str) -> Result<(), GitError> {
    let local_ref = format!("refs/heads/{branch}");

    let already_correct = repo
        .head()
        .ok()
        .and_then(|h| h.name().map(String::from))
        .as_deref()
        == Some(local_ref.as_str());
    if already_correct {
        return Ok(());
    }

    let remote_ref = format!("refs/remotes/origin/{branch}");
    if let Ok(r) = repo.find_reference(&remote_ref) {
        // The configured branch exists on the remote under a different name
        // than HEAD's default — create a local branch tracking it and
        // switch over.
        let commit = r.peel_to_commit()?;
        repo.branch(branch, &commit, false)?;
        repo.set_head(&local_ref)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
        return Ok(());
    }

    if repo.is_empty()? {
        // Genuinely fresh remote with no branches at all yet — point HEAD at
        // the configured branch as unborn so the first local commit creates
        // it there.
        repo.set_head(&local_ref)?;
        return Ok(());
    }

    Err(GitError::PushFailed(format!(
        "remote has no branch '{branch}' and is not empty — check the source's branch setting"
    )))
}

/// Connect to `url` and return the name of the remote's default branch
/// (its `HEAD` symref), without cloning anything.
///
/// Used to pre-fill a git source's branch field with the value that will
/// actually be checked out, instead of guessing "main" — Forgejo instances
/// commonly still default new repos to "master".
pub fn detect_default_branch(url: &str, pat: &str) -> Result<String, GitError> {
    let pat_owned = pat.to_owned();
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
        git2::Cred::userpass_plaintext("oauth2", &pat_owned)
    });

    let mut remote = git2::Remote::create_detached(url)?;
    let connection = remote
        .connect_auth(git2::Direction::Fetch, Some(callbacks), None)
        .map_err(|e| GitError::DetectFailed(e.message().to_string()))?;
    let head = connection
        .default_branch()
        .map_err(|e| GitError::DetectFailed(e.message().to_string()))?;
    let head_str = head.as_str().ok_or_else(|| {
        GitError::DetectFailed("default branch name is not valid UTF-8".to_string())
    })?;

    Ok(head_str
        .strip_prefix("refs/heads/")
        .unwrap_or(head_str)
        .to_string())
}

/// Fast-forward pull: fetch origin + reset HEAD to origin/<branch>.
pub fn pull_repo(local_path: &Path, branch: &str, pat: &str) -> Result<(), GitError> {
    let repo = git2::Repository::open(local_path)?;

    let pat_owned = pat.to_owned();
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
        git2::Cred::userpass_plaintext("oauth2", &pat_owned)
    });

    let mut remote = repo.find_remote("origin")?;
    let mut fo = git2::FetchOptions::new();
    fo.remote_callbacks(callbacks);
    remote.fetch(&[branch], Some(&mut fo), None)?;

    let fetch_head = repo.find_reference("FETCH_HEAD")?;
    let fetch_commit = repo.reference_to_annotated_commit(&fetch_head)?;

    let (analysis, _) = repo.merge_analysis(&[&fetch_commit])?;
    if analysis.is_up_to_date() {
        return Ok(());
    }
    if analysis.is_fast_forward() {
        if worktree_is_dirty(&repo)? {
            return Err(GitError::DirtyWorktree);
        }
        let refname = format!("refs/heads/{branch}");
        let mut reference = repo.find_reference(&refname)?;
        reference.set_target(fetch_commit.id(), "Fast-forward")?;
        repo.set_head(&refname)?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::default().force()))?;
        return Ok(());
    }
    Err(GitError::NotFastForward)
}

/// Repo-relative paths with staged, unstaged, or untracked changes.
///
/// Used by the explicit "Commit & push server copy" action. Ignored files are
/// excluded, as are common editor scratch files so an open VSCodium session
/// cannot accidentally commit its swap files.
pub fn dirty_paths(local_path: &Path) -> Result<Vec<String>, GitError> {
    let repo = git2::Repository::open(local_path)?;
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(true)
        .recurse_untracked_dirs(true)
        .include_ignored(false);
    let statuses = repo.statuses(Some(&mut opts))?;
    let mut out = Vec::new();
    for entry in statuses.iter() {
        if entry.status().is_empty() {
            continue;
        }
        let Some(path) = entry.path() else {
            continue;
        };
        if is_editor_scratch_path(path) {
            continue;
        }
        out.push(path.to_string());
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn is_editor_scratch_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    name == ".DS_Store"
        || name.ends_with('~')
        || name.ends_with(".swp")
        || name.ends_with(".swo")
        || name.ends_with(".tmp")
}

/// Whether the clone has staged or unstaged changes to tracked files.
/// Untracked and ignored files are deliberately ignored: they don't block a
/// fast-forward checkout and shouldn't turn every scheduled sync into an error.
fn worktree_is_dirty(repo: &git2::Repository) -> Result<bool, GitError> {
    let mut opts = git2::StatusOptions::new();
    opts.include_untracked(false)
        .include_ignored(false)
        .include_unmodified(false);
    let statuses = repo.statuses(Some(&mut opts))?;
    Ok(statuses.iter().any(|entry| !entry.status().is_empty()))
}

/// Stage `rel_path`, commit, and push to origin/<branch>.
#[allow(dead_code)] // single-path convenience wrapper; the pusher closure uses the multi-path form
pub fn commit_and_push(
    local_path: &Path,
    rel_path: &str,
    branch: &str,
    pat: &str,
) -> Result<(), GitError> {
    commit_and_push_paths(local_path, &[rel_path.to_string()], branch, pat)
}

/// Stage every path in `rel_paths`, commit, and push to origin/<branch>.
///
/// A path that exists on disk is added/updated; one that no longer does is
/// staged for deletion — used when consolidating a note into a markdown file
/// with image sidecars (several writes, one commit) or deleting a file after
/// pulling it into a note.
pub fn commit_and_push_paths(
    local_path: &Path,
    rel_paths: &[String],
    branch: &str,
    pat: &str,
) -> Result<(), GitError> {
    let repo = git2::Repository::open(local_path)?;

    // Cloning an empty remote repo always leaves local HEAD as an unborn
    // branch on refs/heads/master — git has no ref to negotiate the default
    // branch name against, so it ignores init.defaultBranch and falls back to
    // its hardcoded default. Realign HEAD to the configured branch before the
    // first commit, otherwise the commit lands on `master` and the push below
    // (which always targets refs/heads/{branch}) fails with "src refspec
    // ... does not match any existing object".
    if repo.is_empty()? {
        repo.set_head(&format!("refs/heads/{branch}"))?;
    }

    // Stage each path — add/update when present, remove when deleted.
    let mut index = repo.index()?;
    for rel_path in rel_paths {
        let p = Path::new(rel_path);
        if local_path.join(p).exists() {
            index.add_path(p)?;
        } else {
            // Ignore a path that was never tracked (nothing to remove).
            let _ = index.remove_path(p);
        }
    }
    index.write()?;
    let tree_oid = index.write_tree()?;
    let tree = repo.find_tree(tree_oid)?;

    // Build commit.
    let sig = repo
        .signature()
        .or_else(|_| git2::Signature::now("tidl", "tidl@localhost"))?;
    let parent_commit = if repo.is_empty()? {
        None
    } else {
        repo.head().ok().and_then(|h| h.peel_to_commit().ok())
    };
    let parents: Vec<&git2::Commit> = parent_commit.iter().collect();
    let message = match rel_paths {
        [single] => {
            if local_path.join(single).exists() {
                format!("tidl: edit {single}")
            } else {
                format!("tidl: delete {single}")
            }
        }
        many => format!("tidl: edit {} files", many.len()),
    };
    repo.commit(Some("HEAD"), &sig, &sig, &message, &tree, &parents)?;

    // Push.
    let mut remote = repo.find_remote("origin")?;
    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    let mut po = git2::PushOptions::new();
    let mut callbacks = git2::RemoteCallbacks::new();
    let pat_owned = pat.to_owned();
    callbacks.credentials(move |_url, _username, _allowed| {
        git2::Cred::userpass_plaintext("oauth2", &pat_owned)
    });
    callbacks.push_update_reference(|refname, status| {
        if let Some(msg) = status {
            return Err(git2::Error::from_str(&format!(
                "push rejected for {refname}: {msg}"
            )));
        }
        Ok(())
    });
    po.remote_callbacks(callbacks);
    remote
        .push(&[refspec.as_str()], Some(&mut po))
        .map_err(|e| GitError::PushFailed(e.message().to_string()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(label: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "tidl_git_sync_test_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        TempDir(path)
    }

    /// Regression test for: cloning an empty repo (as when tidl points at a
    /// freshly created, unpushed Forgejo repo) always lands local HEAD on
    /// refs/heads/master, so a naive `push refs/heads/main:refs/heads/main`
    /// fails with "src refspec 'refs/heads/main' does not match any existing
    /// object". commit_and_push must realign HEAD to the configured branch
    /// before the first commit.
    #[test]
    fn commit_and_push_realigns_branch_on_empty_repo() {
        let remote_dir = temp_dir("remote");
        let clone_dir = temp_dir("clone");

        git2::Repository::init_bare(&remote_dir.0).expect("init bare remote");

        let remote_url = remote_dir.0.to_str().unwrap();
        clone_repo(remote_url, &clone_dir.0, "main", "unused").expect("clone empty repo");

        std::fs::write(clone_dir.0.join("notes.md"), b"# hello\n").expect("write file");

        commit_and_push(&clone_dir.0, "notes.md", "main", "unused")
            .expect("commit_and_push should succeed against the configured branch");

        let remote_repo = git2::Repository::open_bare(&remote_dir.0).expect("open remote");
        let main_ref = remote_repo
            .find_reference("refs/heads/main")
            .expect("refs/heads/main must exist on the remote after push");
        let commit = main_ref
            .peel_to_commit()
            .expect("main must point at a commit");
        assert_eq!(commit.message(), Some("tidl: edit notes.md"));

        assert!(
            remote_repo.find_reference("refs/heads/master").is_err(),
            "push must not have landed on refs/heads/master"
        );
    }

    fn seed_commit(
        repo: &git2::Repository,
        branch_ref: &str,
        filename: &str,
        content: &[u8],
        message: &str,
    ) {
        let sig = git2::Signature::now("seed", "seed@test").unwrap();
        let blob = repo.blob(content).unwrap();
        let mut tb = repo.treebuilder(None).unwrap();
        tb.insert(filename, blob, 0o100644).unwrap();
        let tree_id = tb.write().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        repo.commit(Some(branch_ref), &sig, &sig, message, &tree, &[])
            .unwrap();
    }

    /// Regression test for the non-empty-repo case: the remote already has
    /// content, but its default (HEAD) branch differs from the branch the
    /// source is configured for — the exact shape of a real Forgejo repo
    /// whose default branch is "master"/"trunk" while the source syncs
    /// "main". `clone_repo` must follow the configured branch, not whatever
    /// the remote checks out by default.
    #[test]
    fn clone_and_push_use_configured_branch_when_it_differs_from_remote_default() {
        let remote_dir = temp_dir("remote_nondefault");
        let clone_dir = temp_dir("clone_nondefault");

        let remote_repo = git2::Repository::init_bare(&remote_dir.0).expect("init bare remote");
        seed_commit(
            &remote_repo,
            "refs/heads/trunk",
            "trunk.md",
            b"trunk content\n",
            "seed trunk",
        );
        remote_repo.set_head("refs/heads/trunk").unwrap();
        seed_commit(
            &remote_repo,
            "refs/heads/main",
            "main.md",
            b"main content\n",
            "seed main",
        );

        let remote_url = remote_dir.0.to_str().unwrap();
        clone_repo(remote_url, &clone_dir.0, "main", "unused")
            .expect("clone should follow the configured branch");

        assert!(
            clone_dir.0.join("main.md").exists(),
            "clone should check out the configured branch's content, not the remote default's"
        );
        assert!(!clone_dir.0.join("trunk.md").exists());

        std::fs::write(clone_dir.0.join("notes.md"), b"# hello\n").expect("write file");
        commit_and_push(&clone_dir.0, "notes.md", "main", "unused")
            .expect("push to the configured branch should succeed");

        let main_ref = remote_repo.find_reference("refs/heads/main").unwrap();
        assert_eq!(
            main_ref.peel_to_commit().unwrap().message(),
            Some("tidl: edit notes.md")
        );

        // trunk (the remote's actual default branch) must be untouched.
        let trunk_ref = remote_repo.find_reference("refs/heads/trunk").unwrap();
        assert_eq!(
            trunk_ref.peel_to_commit().unwrap().message(),
            Some("seed trunk")
        );
    }

    fn commit_local(repo_path: &Path, filename: &str, content: &[u8], message: &str) {
        let repo = git2::Repository::open(repo_path).unwrap();
        std::fs::write(repo_path.join(filename), content).unwrap();
        let mut index = repo.index().unwrap();
        index.add_path(Path::new(filename)).unwrap();
        index.write().unwrap();
        let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
        let sig = git2::Signature::now("local", "local@test").unwrap();
        let parent = repo.head().unwrap().peel_to_commit().unwrap();
        repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])
            .unwrap();
    }

    fn push_change(remote_url: &str, content: &[u8]) {
        let dir = temp_dir("push_change");
        clone_repo(remote_url, &dir.0, "main", "unused").unwrap();
        std::fs::write(dir.0.join("notes.md"), content).unwrap();
        commit_and_push(&dir.0, "notes.md", "main", "unused").unwrap();
    }

    #[test]
    fn dirty_paths_reports_changes_but_skips_editor_scratch_files() {
        let remote_dir = temp_dir("remote_dirty_paths");
        let clone_dir = temp_dir("clone_dirty_paths");
        let remote_repo = git2::Repository::init_bare(&remote_dir.0).expect("init bare remote");
        seed_commit(
            &remote_repo,
            "refs/heads/main",
            "notes.md",
            b"one\n",
            "seed",
        );
        remote_repo.set_head("refs/heads/main").unwrap();

        let remote_url = remote_dir.0.to_str().unwrap();
        clone_repo(remote_url, &clone_dir.0, "main", "unused").expect("clone");
        std::fs::write(clone_dir.0.join("notes.md"), b"changed\n").unwrap();
        std::fs::write(clone_dir.0.join("new.md"), b"new\n").unwrap();
        std::fs::write(clone_dir.0.join(".notes.md.swp"), b"scratch").unwrap();
        std::fs::write(clone_dir.0.join(".DS_Store"), b"scratch").unwrap();

        let paths = dirty_paths(&clone_dir.0).unwrap();
        assert_eq!(paths, vec!["new.md".to_string(), "notes.md".to_string()]);
    }

    #[test]
    fn pull_repo_refuses_to_overwrite_a_dirty_worktree() {
        let remote_dir = temp_dir("remote_dirty");
        let clone_dir = temp_dir("clone_dirty");
        let remote_repo = git2::Repository::init_bare(&remote_dir.0).expect("init bare remote");
        seed_commit(
            &remote_repo,
            "refs/heads/main",
            "notes.md",
            b"one\n",
            "seed",
        );
        remote_repo.set_head("refs/heads/main").unwrap();

        let remote_url = remote_dir.0.to_str().unwrap();
        clone_repo(remote_url, &clone_dir.0, "main", "unused").expect("clone");
        push_change(remote_url, b"two\n");

        std::fs::write(clone_dir.0.join("notes.md"), b"local edit\n").unwrap();
        let err = pull_repo(&clone_dir.0, "main", "unused").unwrap_err();
        assert!(matches!(err, GitError::DirtyWorktree), "got {err:?}");
    }

    #[test]
    fn pull_repo_reports_divergence_instead_of_silently_doing_nothing() {
        let remote_dir = temp_dir("remote_diverge");
        let clone_dir = temp_dir("clone_diverge");
        let remote_repo = git2::Repository::init_bare(&remote_dir.0).expect("init bare remote");
        seed_commit(
            &remote_repo,
            "refs/heads/main",
            "notes.md",
            b"one\n",
            "seed",
        );
        remote_repo.set_head("refs/heads/main").unwrap();

        let remote_url = remote_dir.0.to_str().unwrap();
        clone_repo(remote_url, &clone_dir.0, "main", "unused").expect("clone");
        commit_local(&clone_dir.0, "notes.md", b"local\n", "local change");
        push_change(remote_url, b"remote\n");

        let err = pull_repo(&clone_dir.0, "main", "unused").unwrap_err();
        assert!(matches!(err, GitError::NotFastForward), "got {err:?}");
    }

    #[test]
    fn pull_repo_allows_fast_forward_with_only_untracked_files() {
        let remote_dir = temp_dir("remote_untracked");
        let clone_dir = temp_dir("clone_untracked");
        let remote_repo = git2::Repository::init_bare(&remote_dir.0).expect("init bare remote");
        seed_commit(
            &remote_repo,
            "refs/heads/main",
            "notes.md",
            b"one\n",
            "seed",
        );
        remote_repo.set_head("refs/heads/main").unwrap();

        let remote_url = remote_dir.0.to_str().unwrap();
        clone_repo(remote_url, &clone_dir.0, "main", "unused").expect("clone");
        std::fs::write(clone_dir.0.join("scratch.txt"), b"untracked\n").unwrap();
        push_change(remote_url, b"two\n");

        pull_repo(&clone_dir.0, "main", "unused").expect("untracked files should not block");
        assert_eq!(
            std::fs::read_to_string(clone_dir.0.join("notes.md")).unwrap(),
            "two\n"
        );
    }

    /// `detect_default_branch` must report the remote's real HEAD branch name
    /// (e.g. "master" on most self-hosted Forgejo instances) rather than
    /// tidl's UI-side "main" placeholder.
    #[test]
    fn detect_default_branch_reports_remote_head() {
        let remote_dir = temp_dir("remote_detect");
        let remote_repo = git2::Repository::init_bare(&remote_dir.0).expect("init bare remote");
        seed_commit(
            &remote_repo,
            "refs/heads/master",
            "readme.md",
            b"hi\n",
            "seed master",
        );
        remote_repo.set_head("refs/heads/master").unwrap();

        let remote_url = remote_dir.0.to_str().unwrap();
        let branch = detect_default_branch(remote_url, "unused").expect("detect should succeed");
        assert_eq!(branch, "master");
    }
}
