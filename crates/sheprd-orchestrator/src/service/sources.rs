//! Which sources a repository has, which this build reads, the GitLab and
//! Forgejo CLIs' say in which forge a remote is on, and where a full listing
//! is kept.

use super::{
    Access, Item, Provider, RepoInfo, Result, SourceKey, SourceStatus, SyncState, Worker, azure,
    forgejo, gitlab,
};
use chrono::Utc;
use std::collections::{HashMap, HashSet};

impl SyncState {
    /// How many items the source's last sync reported, kept while it syncs
    /// again; `None` before one has, or after one failed.
    pub fn count(&self) -> Option<usize> {
        match self {
            Self::Synced { count, .. } | Self::Capped { count } => Some(*count),
            Self::Syncing { count } => *count,
            Self::SignIn | Self::Unsupported | Self::Failed(_) => None,
        }
    }
}

impl Worker {
    /// Marks `key` syncing, keeping the count its last sync reported.
    pub(super) fn set_syncing(&mut self, key: &SourceKey) {
        if let Some(status) = self.snapshot.sources.iter_mut().find(|s| s.key == *key) {
            status.state = SyncState::Syncing {
                count: status.state.count(),
            };
        }
    }

    /// Brings the store to `info`'s Beads source, then reads it again. `info`
    /// must be what was just discovered: a cached one can be older than what
    /// another checkout of the repository synced, and settling on it would
    /// forget those tasks. Failing leaves the old rows, which
    /// [`keep_current_beads`] still hides.
    pub(super) fn settle_beads(&mut self, info: &RepoInfo) -> Result<()> {
        let Some(store) = self
            .store
            .as_mut()
            .filter(|store| matches!(store.access(), Access::ReadWrite))
        else {
            return Ok(());
        };
        if let Err(error) = store.settle_beads(info.beads_source().as_ref()) {
            tracing::warn!(
                category = "orchestrator",
                %error,
                "Cannot bring the stored beads to the project's source"
            );
        }
        self.reload()
    }
}

impl Worker {
    /// Stores a complete `listing` of `key` when the database is `writable`;
    /// a capped one, or any the database cannot take, is shown from memory.
    pub(super) fn keep_listing(
        &mut self,
        key: &SourceKey,
        listing: gitlab::Listing,
        writable: bool,
    ) -> Result<SyncState> {
        let Some(store) = self.store.as_mut() else {
            return Ok(SyncState::Unsupported);
        };
        let count = listing.items.len();
        if !listing.complete || !writable {
            self.uncached.insert(key.clone(), listing.items);
            return Ok(if listing.complete {
                SyncState::Synced {
                    at: Utc::now(),
                    count,
                }
            } else {
                SyncState::Capped { count }
            });
        }
        store.replace_items(key, &listing.items, &listing.checkpoint)?;
        self.uncached.remove(key);
        Ok(SyncState::Synced {
            at: Utc::now(),
            count,
        })
    }

    /// Reads the shown remote as GitLab once glab knows its host, such as
    /// through an `ssh_host` added after the repository was found, or as
    /// Forgejo once fj does.
    pub(super) fn recognize(
        &mut self,
        cli: impl FnOnce() -> Vec<gitlab::cli_hosts::CliHost>,
        keys: impl FnOnce() -> forgejo::cli_keys::Keys,
    ) {
        let Some(mut info) = self.snapshot.repo.clone() else {
            return;
        };
        if recognize(&mut info, cli, keys) {
            self.snapshot.sources = statuses(&info);
            self.snapshot.repo = Some(info);
        }
    }
}

/// Makes `info`'s remote a GitLab source when glab knows its host, else a
/// Forgejo one when fj does; returns whether it changed. `cli` reads glab's
/// config and `keys` fj's sign-ins, each only when needed.
pub(super) fn recognize(
    info: &mut RepoInfo,
    cli: impl FnOnce() -> Vec<gitlab::cli_hosts::CliHost>,
    keys: impl FnOnce() -> forgejo::cli_keys::Keys,
) -> bool {
    info.remote.as_mut().is_some_and(|remote| {
        gitlab::recognize(&mut remote.source, cli) || forgejo::recognize(remote, keys)
    })
}

/// Drops beads of any source but `info`'s. A database this build cannot
/// write still holds them after the source moves, as a `.beads/redirect`
/// added later moves it, and they would list beside the ones read now.
/// Before the repository is known, what was last listed stays.
pub(super) fn keep_current_beads(items: &mut Vec<Item>, info: Option<&RepoInfo>) {
    let Some(info) = info else {
        return;
    };
    let current = info.beads_source();
    items.retain(|item| {
        item.key.source.provider != Provider::Beads || Some(&item.key.source) == current.as_ref()
    });
}

/// Each source of `info`, syncing when this build can read it.
pub(super) fn statuses(info: &RepoInfo) -> Vec<SourceStatus> {
    sources(info)
        .into_iter()
        .map(|(key, supported)| SourceStatus {
            key,
            state: if supported {
                SyncState::Syncing { count: None }
            } else {
                SyncState::Unsupported
            },
        })
        .collect()
}

/// The sources a repository has, and whether this build can read each.
pub fn sources(info: &RepoInfo) -> Vec<(SourceKey, bool)> {
    let remote = info.remote.as_ref().map(|remote| {
        let supported = match remote.source.provider {
            Provider::Azure | Provider::Forgejo => true,
            Provider::Github => remote.source.host == "github.com",
            Provider::Gitlab => true,
            Provider::Beads => false,
        };
        (remote.source.clone(), supported)
    });
    remote
        .into_iter()
        .chain(info.beads_source().map(|key| (key, true)))
        .collect()
}

/// Every source whose stored items a listing shown but not stored replaces:
/// its own, and for Azure DevOps its project's too, which its work items are
/// keyed by. A listing whose project now has no open work item still hides
/// the stored ones.
pub(super) fn replaced_by(uncached: &HashMap<SourceKey, Vec<Item>>) -> HashSet<SourceKey> {
    uncached
        .keys()
        .flat_map(|key| {
            let project = (key.provider == Provider::Azure)
                .then(|| {
                    azure::Repository::of(key)
                        .ok()
                        .map(|repository| repository.project_source())
                })
                .flatten();
            std::iter::once(key.clone()).chain(project)
        })
        .collect()
}
