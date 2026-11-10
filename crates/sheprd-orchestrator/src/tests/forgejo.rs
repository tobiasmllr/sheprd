use super::*;
use crate::forgejo::{
    self, Api, Credential, IssueNode, Listed, MAX_OPEN, Page, PullRequestNode, Scheme, list,
    listing_url, status_error, total,
};
use crate::service::retry_wait;
use secrecy::SecretString;
use std::{collections::HashMap, time::Duration};

fn forgejo() -> SourceKey {
    SourceKey {
        provider: Provider::Forgejo,
        host: "codeberg.org".into(),
        repository: "forgejo/app".into(),
    }
}

fn api(instance: &str) -> Api {
    Api {
        instance: instance.into(),
        scheme: Scheme::Https,
    }
}

fn node(number: u64) -> IssueNode {
    serde_json::from_value(serde_json::json!({
        "id": 9000 + number,
        "number": number,
        "title": format!("Issue {number}"),
        "body": "Body",
        "state": "open",
        "html_url": format!("https://codeberg.org/forgejo/app/issues/{number}"),
        "created_at": "2026-11-10T12:03:41+01:00",
        "updated_at": "2026-11-10T12:09:32+01:00",
        "user": {"id": 1, "login": "penso"},
        "labels": [{"id": 3, "name": "bug", "color": "ee0701"}],
        "comments": 2,
        "pull_request": null,
    }))
    .unwrap()
}

fn pull_request(extra: serde_json::Value) -> PullRequestNode {
    let mut value = serde_json::json!({
        "number": 7,
        "title": "Fix it",
        "body": "",
        "state": "open",
        "html_url": "https://codeberg.org/forgejo/app/pulls/7",
        "created_at": "2026-11-10T21:34:21+01:00",
        "updated_at": "2026-11-10T21:40:00+01:00",
        "user": {"login": "penso"},
        "labels": [],
        "comments": 1,
        "additions": 12,
        "deletions": 3,
        "base": {"ref": "main", "sha": "aaa", "repo": {"full_name": "forgejo/app"}},
        "head": {"ref": "fix", "sha": "bbb", "repo": {"full_name": "penso/app"}},
    });
    for (name, field) in extra.as_object().unwrap() {
        value[name] = field.clone();
    }
    serde_json::from_value(value).unwrap()
}

/// A repository's open issues as Forgejo answers the listing: numbered
/// pages, newest first, each with the count of all of them.
struct Repo {
    open: Vec<u64>,
    /// The most one page holds, whatever was asked for.
    page: usize,
    /// Whether the count is sent, as Forgejo does.
    counted: bool,
    asked: usize,
}

impl Repo {
    fn new(count: u64) -> Self {
        Self {
            open: (1..=count).rev().collect(),
            page: 50,
            counted: true,
            asked: 0,
        }
    }

    fn answer(&mut self, page: usize) -> Page<IssueNode> {
        self.asked += 1;
        Page {
            nodes: self
                .open
                .iter()
                .skip((page - 1) * self.page)
                .take(self.page)
                .map(|number| node(*number))
                .collect(),
            total: self.counted.then_some(self.open.len() as u64),
        }
    }
}

fn numbers(items: &[Item]) -> Vec<u64> {
    let mut numbers: Vec<u64> = items
        .iter()
        .map(|item| item.key.native_id.parse().unwrap())
        .collect();
    numbers.sort_unstable();
    numbers
}

#[test]
fn an_issue_reads_in_the_shared_shape() {
    let item = node(42).into_item(&forgejo());
    assert_eq!(item.key.native_id, "42");
    assert_eq!(
        item.key.canonical(),
        "forgejo:codeberg.org:forgejo/app:42",
        "keyed by the repository's number, not Forgejo's global id"
    );
    assert_eq!(item.key.canonical().parse::<ItemKey>().unwrap(), item.key);
    assert_eq!(item.identifier, "#42");
    assert_eq!(item.state, "open");
    assert_eq!(item.author.as_deref(), Some("penso"));
    assert_eq!(item.labels, ["bug"]);
    assert_eq!(item.description.as_deref(), Some("Body"));
    assert_eq!(item.updated_at, Some(at("2026-11-10T11:09:32+00:00")));
    assert_eq!(
        item.activity.and_then(|activity| activity.comments),
        Some(2)
    );
    assert!(item.pull_request.is_none());
}

#[test]
fn an_issue_without_optional_fields_still_decodes() {
    let node: IssueNode = serde_json::from_value(serde_json::json!({
        "number": 1, "title": "Bare", "body": null, "state": "closed",
        "html_url": "https://codeberg.org/forgejo/app/issues/1",
        "created_at": "2026-10-08T23:23:37Z", "updated_at": "2026-10-08T23:23:37Z",
        "user": null, "labels": null,
    }))
    .unwrap();
    let item = node.into_item(&forgejo());
    assert_eq!(item.state, "closed");
    assert!(item.description.is_none() && item.author.is_none() && item.labels.is_empty());
}

#[test]
fn a_pull_request_carries_its_branches_and_diff_size() {
    let item = pull_request(serde_json::json!({})).into_item(&forgejo());
    assert_eq!(item.key.native_id, "pr/7");
    assert_eq!(item.key.canonical().parse::<ItemKey>().unwrap(), item.key);
    assert_eq!(item.identifier, "#7");
    assert_eq!(item.state, "open");
    assert!(item.description.is_none(), "an empty body is none");
    let pr = item.pull_request.unwrap();
    assert_eq!(
        (pr.number, pr.additions, pr.deletions),
        (7, Some(12), Some(3))
    );
    assert_eq!(
        (pr.base_ref.as_str(), pr.base_sha.as_str()),
        ("main", "aaa")
    );
    assert_eq!((pr.head_ref.as_str(), pr.head_sha.as_str()), ("fix", "bbb"));
    assert_eq!(pr.head_repository.as_deref(), Some("penso/app"));
}

#[test]
fn a_pull_request_reads_as_draft_merged_or_closed() {
    let state = |extra| pull_request(extra).into_item(&forgejo()).state;
    assert_eq!(state(serde_json::json!({"draft": true})), "draft");
    assert_eq!(
        state(serde_json::json!({"state": "closed", "merged": true})),
        "merged"
    );
    assert_eq!(
        state(serde_json::json!({"state": "closed", "draft": true})),
        "closed"
    );
}

#[test]
fn a_pull_request_from_a_deleted_fork_still_decodes() {
    let item = pull_request(serde_json::json!({
        "head": {"ref": "fix", "sha": "bbb", "repo": null},
        "additions": null,
        "deletions": null,
    }))
    .into_item(&forgejo());
    let pr = item.pull_request.unwrap();
    assert_eq!(pr.head_ref, "fix");
    assert!(pr.head_repository.is_none() && pr.additions.is_none());
}

#[test]
fn the_listing_address_names_the_owner_and_repository() {
    let url = listing_url::<IssueNode>((&forgejo(), &api("codeberg.org")), 2).unwrap();
    assert_eq!(url.scheme(), "https");
    assert_eq!(url.host_str(), Some("codeberg.org"));
    assert_eq!(url.path(), "/api/v1/repos/forgejo/app/issues");
    let query: HashMap<_, _> = url.query_pairs().into_owned().collect();
    assert_eq!(query["state"], "open");
    assert_eq!(query["type"], "issues", "pull requests are listed apart");
    assert_eq!(query["limit"], "50");
    assert_eq!(query["page"], "2");

    let url = listing_url::<PullRequestNode>((&forgejo(), &api("codeberg.org")), 1).unwrap();
    assert_eq!(url.path(), "/api/v1/repos/forgejo/app/pulls");
    assert!(!url.query_pairs().any(|(name, _)| name == "type"));
}

#[test]
fn an_instance_served_at_a_port_or_a_path_keeps_them() {
    let http = Api {
        instance: "localhost:3000".into(),
        scheme: Scheme::Http,
    };
    let url = listing_url::<IssueNode>((&forgejo(), &http), 1).unwrap();
    assert_eq!((url.scheme(), url.port()), ("http", Some(3000)));
    assert_eq!(url.path(), "/api/v1/repos/forgejo/app/issues");

    let url = listing_url::<IssueNode>((&forgejo(), &api("example.com/forge")), 1).unwrap();
    assert_eq!(url.host_str(), Some("example.com"));
    assert_eq!(url.path(), "/forge/api/v1/repos/forgejo/app/issues");
}

#[test]
fn a_listing_reads_every_page() {
    let mut repo = Repo::new(120);
    let (items, complete) = list(&forgejo(), &mut |page| Ok(repo.answer(page))).unwrap();
    assert!(complete);
    assert_eq!(numbers(&items), (1..=120).collect::<Vec<_>>());
    assert_eq!(repo.asked, 3, "the count ends the listing");
}

#[test]
fn a_server_that_answers_smaller_pages_is_still_read_whole() {
    let mut repo = Repo::new(45);
    repo.page = 20;
    let (items, complete) = list(&forgejo(), &mut |page| Ok(repo.answer(page))).unwrap();
    assert!(complete);
    assert_eq!(items.len(), 45);
}

#[test]
fn a_listing_without_a_count_reads_until_an_empty_page() {
    let mut repo = Repo::new(100);
    repo.counted = false;
    let (items, complete) = list(&forgejo(), &mut |page| Ok(repo.answer(page))).unwrap();
    assert!(complete);
    assert_eq!(items.len(), 100);
    assert_eq!(repo.asked, 3);
}

/// Closing an issue on a page already read pulls the next page's first one
/// back onto it, where it is never read. The count changing shows that, and
/// the listing is read again.
#[test]
fn an_issue_closed_between_pages_never_hides_an_open_one() {
    let mut repo = Repo::new(120);
    let mut closed = false;
    let (items, complete) = list(&forgejo(), &mut |page| {
        if page == 2 && !std::mem::replace(&mut closed, true) {
            repo.open.retain(|number| *number != 120);
        }
        Ok(repo.answer(page))
    })
    .unwrap();
    assert!(complete);
    assert_eq!(numbers(&items), (1..=119).collect::<Vec<_>>());
}

#[test]
fn a_listing_that_keeps_changing_is_shown_but_not_complete() {
    let mut repo = Repo::new(120);
    let (items, complete) = list(&forgejo(), &mut |page| {
        if page == 2 {
            repo.open.remove(0);
        }
        Ok(repo.answer(page))
    })
    .unwrap();
    assert!(
        !complete,
        "an incomplete listing must not replace the cache"
    );
    assert!(!items.is_empty());
}

#[test]
fn a_listing_past_the_cap_is_shown_but_not_complete() {
    let mut repo = Repo::new(MAX_OPEN as u64 + 150);
    let (items, complete) = list(&forgejo(), &mut |page| Ok(repo.answer(page))).unwrap();
    assert!(!complete);
    assert_eq!(items.len(), MAX_OPEN);
}

#[test]
fn a_failed_page_fails_the_listing() {
    let mut repo = Repo::new(120);
    let result = list(&forgejo(), &mut |page| {
        if page == 2 {
            return Err(forgejo::Error::RateLimit.into());
        }
        Ok(repo.answer(page))
    });
    assert!(matches!(
        result,
        Err(Error::Forgejo(forgejo::Error::RateLimit))
    ));
}

#[test]
fn the_count_is_read_from_its_header() {
    let mut headers = ureq::http::HeaderMap::new();
    assert_eq!(total(&headers), None);
    headers.insert("x-total-count", "1623".parse().unwrap());
    assert_eq!(total(&headers), Some(1623));
    headers.insert("x-total-count", "many".parse().unwrap());
    assert_eq!(total(&headers), None);
}

#[test]
fn refusals_say_whether_a_token_is_needed_or_wrong() {
    let token = SecretString::from("abc");
    let sent = Credential::Token(&token);
    for status in [401, 403, 404] {
        assert!(matches!(
            status_error(status, "o/app", Credential::Anonymous),
            forgejo::Error::SignIn { repository } if repository == "o/app"
        ));
        assert!(matches!(
            status_error(status, "o/app", Credential::Withheld),
            forgejo::Error::InsecureRemote { repository } if repository == "o/app"
        ));
    }
    assert!(matches!(
        status_error(401, "o/app", sent),
        forgejo::Error::Authentication
    ));
    assert!(matches!(
        status_error(403, "o/app", sent),
        forgejo::Error::Forbidden { .. }
    ));
    assert!(matches!(
        status_error(404, "o/app", sent),
        forgejo::Error::NotFound { .. }
    ));
    assert!(matches!(
        status_error(429, "o/app", sent),
        forgejo::Error::RateLimit
    ));
    assert!(matches!(
        status_error(502, "o/app", sent),
        forgejo::Error::Status(502)
    ));
    let error = Error::from(status_error(404, "o/app", Credential::Anonymous));
    assert!(error.to_string().contains("fj auth login"));
    let error = Error::from(status_error(404, "o/app", Credential::Withheld));
    assert!(error.to_string().contains("https://"));
}

#[test]
fn forgejo_failures_wait_for_the_user_or_the_rate_limit() {
    let half_hour = Duration::from_secs(1800);
    let hour = Duration::from_secs(3600);
    let sign_in = Error::from(forgejo::Error::SignIn {
        repository: "o/app".into(),
    });
    assert_eq!(retry_wait(&sign_in, None), half_hour);
    let refused = Error::from(forgejo::Error::Authentication);
    assert_eq!(retry_wait(&refused, None), half_hour);
    let limited = Error::from(forgejo::Error::RateLimit);
    assert_eq!(retry_wait(&limited, Some(hour)), hour);
    assert_eq!(retry_wait(&limited, None), Duration::from_secs(300));
}
