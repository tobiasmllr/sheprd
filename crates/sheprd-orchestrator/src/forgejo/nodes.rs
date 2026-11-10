//! Forgejo's issues and pull requests in the shared item shape, so the lists
//! show them beside GitHub's.

use super::Listed;
use crate::{Activity, Item, ItemKey, PullRequest, SourceKey};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Deserialize)]
struct User {
    login: String,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

fn labels(labels: Option<Vec<Label>>) -> Vec<String> {
    labels
        .unwrap_or_default()
        .into_iter()
        .map(|label| label.name)
        .collect()
}

/// One entry of `GET /repos/<owner>/<name>/issues`.
#[derive(Deserialize)]
pub struct IssueNode {
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    html_url: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    user: Option<User>,
    labels: Option<Vec<Label>>,
    comments: Option<u64>,
}

impl Listed for IssueNode {
    const COLLECTION: &'static str = "issues";
    /// The collection holds pull requests too, which `pulls` lists in full.
    const FILTER: &'static [(&'static str, &'static str)] = &[("type", "issues")];

    fn number(&self) -> u64 {
        self.number
    }

    fn into_item(self, source: &SourceKey) -> Item {
        Item {
            key: ItemKey {
                source: source.clone(),
                native_id: self.number.to_string(),
            },
            identifier: format!("#{}", self.number),
            title: self.title,
            description: self.body.filter(|body| !body.is_empty()),
            state: self.state.to_ascii_lowercase(),
            url: Some(self.html_url),
            author: self.user.map(|user| user.login),
            labels: labels(self.labels),
            parent_id: None,
            blocked_by: Vec::new(),
            assignee: None,
            acceptance: None,
            priority: None,
            created_at: Some(self.created_at),
            updated_at: Some(self.updated_at),
            pull_request: None,
            activity: Some(Activity {
                comments: self.comments,
                ..Activity::default()
            }),
        }
    }
}

/// One entry of `GET /repos/<owner>/<name>/pulls`.
#[derive(Deserialize)]
pub struct PullRequestNode {
    number: u64,
    title: String,
    body: Option<String>,
    state: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    merged: bool,
    html_url: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    user: Option<User>,
    labels: Option<Vec<Label>>,
    comments: Option<u64>,
    additions: Option<u64>,
    deletions: Option<u64>,
    base: Option<Branch>,
    head: Option<Branch>,
}

/// One side of a pull request.
#[derive(Default, Deserialize)]
struct Branch {
    #[serde(rename = "ref", default)]
    name: String,
    #[serde(default)]
    sha: String,
    /// None once a fork is deleted.
    repo: Option<Repository>,
}

#[derive(Deserialize)]
struct Repository {
    full_name: String,
}

impl PullRequestNode {
    /// The state word the list filters and colors by.
    fn state(&self) -> &'static str {
        match self.state.as_str() {
            _ if self.merged => "merged",
            "closed" => "closed",
            _ if self.draft => "draft",
            _ => "open",
        }
    }
}

impl Listed for PullRequestNode {
    const COLLECTION: &'static str = "pulls";
    const FILTER: &'static [(&'static str, &'static str)] = &[];

    fn number(&self) -> u64 {
        self.number
    }

    fn into_item(self, source: &SourceKey) -> Item {
        let state = self.state();
        let base = self.base.unwrap_or_default();
        let head = self.head.unwrap_or_default();
        Item {
            key: ItemKey {
                source: source.clone(),
                native_id: format!("pr/{}", self.number),
            },
            identifier: format!("#{}", self.number),
            title: self.title,
            description: self.body.filter(|body| !body.is_empty()),
            state: state.to_owned(),
            url: Some(self.html_url),
            author: self.user.map(|user| user.login),
            labels: labels(self.labels),
            parent_id: None,
            blocked_by: Vec::new(),
            assignee: None,
            acceptance: None,
            priority: None,
            created_at: Some(self.created_at),
            updated_at: Some(self.updated_at),
            pull_request: Some(PullRequest {
                number: self.number,
                additions: self.additions,
                deletions: self.deletions,
                base_ref: base.name,
                head_ref: head.name,
                base_sha: base.sha,
                head_sha: head.sha,
                head_repository: head.repo.map(|repository| repository.full_name),
            }),
            activity: Some(Activity {
                comments: self.comments,
                ..Activity::default()
            }),
        }
    }
}
