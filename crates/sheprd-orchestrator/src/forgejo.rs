//! A Forgejo repository's open issues and pull requests, read through
//! Forgejo's REST API (`/api/v1/repos/<owner>/<name>/issues` and `/pulls`), on
//! Codeberg or a self-hosted instance. `native_id` is the issue's number, and
//! `pr/<number>` a pull request's, as GitHub's are keyed. Every sync is a
//! complete listing, so it may replace the cache.
//!
//! The token is the Forgejo CLI's own sign-in for the instance (`cli_keys`),
//! sent only to that instance and only over HTTPS. Without one, public
//! repositories still list.
//!
//! A self-hosted instance's name says nothing of what serves it, so a remote
//! is read as Forgejo when the Forgejo CLI is signed in to its host, or has
//! an alias for it: signing in, fj notes the SSH host the instance's
//! repositories are cloned from when it is another, such as `localhost:2222`.

pub mod cli_keys;
mod nodes;

pub use super::gitlab::{Credential, Listing, Scheme};
pub use nodes::{IssueNode, PullRequestNode};

use super::{Checkpoint, Item, Provider, SourceKey, repo::Remote};
use chrono::Utc;
use secrecy::{ExposeSecret, SecretString};
use serde::de::DeserializeOwned;
use std::{collections::HashSet, io::Read, time::Duration, time::SystemTime};
use zeroize::Zeroizing;

/// The most Forgejo answers in one page unless its administrator raised it.
const PAGE: usize = 50;
/// The most open issues, and as many pull requests, one sync reads. A
/// repository with more is shown but not cached, since a capped listing is
/// no full sync.
pub const MAX_OPEN: usize = 2_000;
const TIMEOUT: Duration = Duration::from_secs(30);
/// One page of issues or pull requests, descriptions included.
const LIMIT: u64 = 4 * 1024 * 1024;

/// Why Forgejo could not be read.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Forgejo's address for {0:?} is not valid")]
    Address(String),
    #[error(
        "Forgejo refused the Forgejo CLI's token; sign in again with fj auth login or fj auth add-token"
    )]
    Authentication,
    #[error(
        "Forgejo refused access to {repository}; the token needs to read issues and repositories"
    )]
    Forbidden { repository: String },
    #[error(
        "Forgejo shows {repository} only when signed in, and a token is never sent over plain HTTP: use an https:// remote"
    )]
    InsecureRemote { repository: String },
    #[error("Forgejo's answer is not the expected JSON: {0}")]
    Json(#[source] serde_json::Error),
    #[error("Forgejo could not be reached at {origin}: {source}")]
    Network {
        /// The scheme and host tried, so a wrong one shows.
        origin: String,
        #[source]
        source: Box<ureq::Error>,
    },
    #[error("Forgejo shows no repository {repository} to this token")]
    NotFound { repository: String },
    #[error("Forgejo rate limit reached; it is read again later")]
    RateLimit,
    #[error("Forgejo's answer could not be read: {0}")]
    Read(#[source] std::io::Error),
    #[error(
        "Forgejo shows {repository} only when signed in: sign in to its host with the Forgejo CLI (fj auth login or fj auth add-token)"
    )]
    SignIn { repository: String },
    #[error("Forgejo's answer exceeded the size limit")]
    Size,
    #[error("Forgejo request failed (HTTP {0})")]
    Status(u16),
    #[error("The Forgejo token is not one an HTTP header can carry")]
    Token,
}

impl Error {
    /// A failure only the user can fix, such as a missing or refused token.
    pub fn needs_user(&self) -> bool {
        matches!(
            self,
            Self::SignIn { .. }
                | Self::InsecureRemote { .. }
                | Self::Authentication
                | Self::Forbidden { .. }
                | Self::NotFound { .. }
        )
    }
}

/// `remote` as a Forgejo repository when its host's name alone reads as
/// GitHub but the Forgejo CLI knows that host, as an instance or an alias of
/// one. Only the provider changes; the host and path keep their key. `keys`
/// reads the CLI's sign-ins, only when needed. Returns whether it changed.
pub fn recognize(remote: &mut Remote, keys: impl FnOnce() -> cli_keys::Keys) -> bool {
    if remote.source.provider != Provider::Github || remote.source.host == "github.com" {
        return false;
    }
    let known = keys()
        .instance(names(&remote.source, Some(&remote.url)))
        .is_some();
    if known {
        remote.source.provider = Provider::Forgejo;
    }
    known
}

/// Whether a host's name alone says Forgejo serves it.
pub fn named(host: &str) -> bool {
    host == "codeberg.org" || host.contains("forgejo")
}

/// What the Forgejo CLI may know a source's host as: its remote's host with
/// the port the URL names, which the source key drops for SSH, then the
/// key's own.
fn names<'a>(source: &'a SourceKey, remote: Option<&str>) -> impl Iterator<Item = String> + 'a {
    remote
        .and_then(authority)
        .into_iter()
        .chain(std::iter::once(source.host.clone()))
}

/// A remote URL's host, with its port when it names one.
fn authority(remote: &str) -> Option<String> {
    let url = url::Url::parse(remote).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    Some(
        url.port()
            .map_or(host.clone(), |port| format!("{host}:{port}")),
    )
}

/// Where a repository's API is: the instance the Forgejo CLI names for the
/// remote's host, else that host itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Api {
    /// The instance as the Forgejo CLI names it: a host, with a port or a
    /// path when the instance is served at one. Its token is kept under it.
    pub instance: String,
    pub scheme: Scheme,
}

impl Api {
    /// The API of `source`, whose remote is at `remote` when the repository
    /// shown is the source's, given the Forgejo CLI's sign-ins. HTTP only for
    /// an `http://` remote that is the instance itself; an alias's instance
    /// is read over HTTPS, as every other remote is.
    pub fn new(source: &SourceKey, remote: Option<&str>, keys: &cli_keys::Keys) -> Self {
        let own = remote.map_or(Scheme::Https, Scheme::of_remote);
        match keys.instance(names(source, remote)) {
            Some(known) => Self {
                scheme: if known.alias { Scheme::Https } else { own },
                instance: known.instance,
            },
            None => Self {
                instance: source.host.clone(),
                scheme: own,
            },
        }
    }

    fn origin(&self) -> String {
        format!("{}://{}", self.scheme.as_str(), self.instance)
    }
}

/// Lists `source`'s open issues and pull requests from `api`, reading Forgejo
/// as `token` when given and the scheme keeps it private. `previous` keeps
/// the markers another reader left in the checkpoint.
pub fn sync(
    (source, api): (&SourceKey, &Api),
    token: Option<&SecretString>,
    previous: Option<&Checkpoint>,
    cancelled: &impl Fn() -> bool,
    cooldown: &mut Option<Duration>,
) -> super::Result<Listing> {
    let agent = agent(TIMEOUT);
    let credential = Credential::new(api.scheme, token);
    let started = Utc::now();
    let (mut items, issues) = list::<IssueNode>(source, &mut |page| {
        if cancelled() {
            return Err(super::Error::Cancelled);
        }
        Ok(get_page(&agent, (source, api), credential, page, cooldown)?)
    })?;
    let (pulls, pull_requests) = list::<PullRequestNode>(source, &mut |page| {
        if cancelled() {
            return Err(super::Error::Cancelled);
        }
        Ok(get_page(&agent, (source, api), credential, page, cooldown)?)
    })?;
    items.extend(pulls);
    let previous = previous.cloned().unwrap_or_default();
    Ok(Listing {
        items,
        checkpoint: Checkpoint {
            updated_at: Some(started),
            etag: None,
            last_full_at: Some(Utc::now()),
            pr_details: previous.pr_details,
            pr_cursor: previous.pr_cursor,
        },
        complete: issues && pull_requests,
    })
}

/// A kind of item Forgejo lists a page at a time, from a repository's
/// collection of that name.
pub trait Listed: DeserializeOwned {
    /// The collection's path after `/repos/<owner>/<name>/`.
    const COLLECTION: &'static str;
    /// What narrows the collection to this kind, beyond its being open.
    const FILTER: &'static [(&'static str, &'static str)];

    fn number(&self) -> u64;
    fn into_item(self, source: &SourceKey) -> Item;
}

/// One numbered page, and how many items Forgejo says the whole listing has.
pub struct Page<N> {
    pub nodes: Vec<N>,
    pub total: Option<u64>,
}

/// How one pass over a collection's pages ended.
enum Pass {
    /// Every open item was read.
    Complete,
    /// More than [`MAX_OPEN`] are open.
    Capped,
    /// The collection changed between pages, so an item may have slipped
    /// from an unread page to one already read.
    Moved,
}

/// Every open item of kind `N`, and whether the listing is complete.
///
/// Forgejo's pages are numbered, with no bound to continue from as GitLab
/// has: an item opened or closed while the pages are read shifts the ones
/// after it, so one could be skipped and then dropped from the cache. The
/// total Forgejo sends with each page shows that, and the listing is then
/// read once more; a collection still moving is shown, not stored.
pub fn list<N: Listed>(
    source: &SourceKey,
    fetch: &mut impl FnMut(usize) -> super::Result<Page<N>>,
) -> super::Result<(Vec<Item>, bool)> {
    match pass(source, fetch)? {
        (items, Pass::Complete) => Ok((items, true)),
        (items, Pass::Capped) => Ok((items, false)),
        (_, Pass::Moved) => {
            let (items, pass) = pass(source, fetch)?;
            Ok((items, matches!(pass, Pass::Complete)))
        }
    }
}

fn pass<N: Listed>(
    source: &SourceKey,
    fetch: &mut impl FnMut(usize) -> super::Result<Page<N>>,
) -> super::Result<(Vec<Item>, Pass)> {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    let mut first_total = None;
    for number in 1.. {
        let page = fetch(number)?;
        let steady = *first_total.get_or_insert(page.total) == page.total;
        let read = page.nodes.len();
        items.extend(
            page.nodes
                .into_iter()
                .filter(|node| seen.insert(node.number()))
                .map(|node| node.into_item(source)),
        );
        let count = items.len() as u64;
        // A server may answer fewer than asked for, so only an empty page or
        // the total reached ends a listing.
        if read == 0 || page.total.is_some_and(|total| count >= total) {
            let whole = steady && page.total.is_none_or(|total| count == total);
            return Ok((items, if whole { Pass::Complete } else { Pass::Moved }));
        }
        if !steady {
            return Ok((items, Pass::Moved));
        }
        if items.len() >= MAX_OPEN {
            return Ok((items, Pass::Capped));
        }
    }
    Ok((items, Pass::Capped))
}

/// The API address of page `page` of `source`'s open items of kind `N`.
pub fn listing_url<N: Listed>(
    (source, api): (&SourceKey, &Api),
    page: usize,
) -> Result<url::Url, Error> {
    let invalid = || Error::Address(source.repository.clone());
    let mut url = url::Url::parse(&api.origin()).map_err(|_| invalid())?;
    url.path_segments_mut()
        .map_err(|()| invalid())?
        .pop_if_empty()
        .extend(["api", "v1", "repos"])
        .extend(source.repository.split('/'))
        .push(N::COLLECTION);
    let mut query = url.query_pairs_mut();
    query.append_pair("state", "open");
    for (name, value) in N::FILTER {
        query.append_pair(name, value);
    }
    query
        .append_pair("limit", &PAGE.to_string())
        .append_pair("page", &page.to_string());
    drop(query);
    Ok(url)
}

/// Page `page` of `source`'s collection of kind `N`.
fn get_page<N: Listed>(
    agent: &ureq::Agent,
    (source, api): (&SourceKey, &Api),
    credential: Credential<'_>,
    page: usize,
    cooldown: &mut Option<Duration>,
) -> Result<Page<N>, Error> {
    let url = listing_url::<N>((source, api), page)?;
    let mut request = agent
        .get(url.as_str())
        .header("User-Agent", "Sheprd")
        .header("Accept", "application/json");
    if let Credential::Token(token) = credential {
        request = request.header("Authorization", authorization(token)?);
    }
    let mut reply = request.call().map_err(|error| Error::Network {
        origin: api.origin(),
        source: Box::new(error),
    })?;
    let status = reply.status().as_u16();
    if status != 200 {
        *cooldown = super::gitlab::retry_after(status, reply.headers(), SystemTime::now());
        tracing::warn!(
            category = "orchestrator_forgejo",
            status,
            instance = api.instance.as_str(),
            collection = N::COLLECTION,
            "Forgejo refused a listing"
        );
        return Err(status_error(status, &source.repository, credential));
    }
    let total = total(reply.headers());
    let nodes = serde_json::from_slice(&body(&mut reply, LIMIT)?).map_err(Error::Json)?;
    Ok(Page { nodes, total })
}

/// How many items the whole listing has, which Forgejo sends with each page.
pub fn total(headers: &ureq::http::HeaderMap) -> Option<u64> {
    headers
        .get("x-total-count")?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// An agent for Forgejo's API: bounded by `timeout`, following no redirect,
/// so a token is never carried to another address, and returning refusals
/// as statuses to read.
fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .into()
}

/// A reply's body, refused when it is larger than `limit`.
fn body(reply: &mut ureq::http::Response<ureq::Body>, limit: u64) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    reply
        .body_mut()
        .as_reader()
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(Error::Read)?;
    if bytes.len() as u64 > limit {
        return Err(Error::Size);
    }
    Ok(bytes)
}

fn authorization(token: &SecretString) -> Result<ureq::http::HeaderValue, Error> {
    let text = Zeroizing::new(format!("token {}", token.expose_secret()));
    let mut header = ureq::http::HeaderValue::from_str(&text).map_err(|_| Error::Token)?;
    header.set_sensitive(true);
    Ok(header)
}

/// What a refusal means. Forgejo answers a private repository as missing to
/// a reader without a token, so that asks for one rather than blaming the
/// remote.
pub fn status_error(status: u16, repository: &str, credential: Credential<'_>) -> Error {
    let repository = repository.to_owned();
    match (status, credential) {
        (401 | 403 | 404, Credential::Anonymous) => Error::SignIn { repository },
        (401 | 403 | 404, Credential::Withheld) => Error::InsecureRemote { repository },
        (401, Credential::Token(_)) => Error::Authentication,
        (403, Credential::Token(_)) => Error::Forbidden { repository },
        (404, Credential::Token(_)) => Error::NotFound { repository },
        (429, _) => Error::RateLimit,
        (status, _) => Error::Status(status),
    }
}
