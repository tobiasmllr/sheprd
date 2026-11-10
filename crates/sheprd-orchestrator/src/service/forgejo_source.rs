//! Reading a Forgejo source as the Forgejo CLI is signed in to its instance.
//! The CLI's sign-ins are read on every sync, so a new `fj auth login` is
//! picked up without reopening the view.

use super::{Result, SourceKey, Worker, forgejo};
use crate::Checkpoint;
use std::sync::atomic::Ordering;

impl Worker {
    /// Lists Forgejo source `key`, continuing from `previous`.
    pub(super) fn forgejo_listing(
        &mut self,
        key: &SourceKey,
        previous: Option<&Checkpoint>,
    ) -> Result<forgejo::Listing> {
        // The app reads the API itself, so the sign-ins are this computer's
        // even when the checkout is on another.
        let keys = forgejo::cli_keys::read();
        let remote = self
            .snapshot
            .repo
            .as_ref()
            .and_then(|info| info.remote.as_ref())
            .filter(|remote| remote.source == *key)
            .map(|remote| remote.url.as_str());
        let api = forgejo::Api::new(key, remote, &keys);
        let cancelled = &self.cancelled;
        forgejo::sync(
            (key, &api),
            keys.token(&api.instance),
            previous,
            &|| cancelled.load(Ordering::Relaxed),
            &mut self.cooldown,
        )
    }
}
