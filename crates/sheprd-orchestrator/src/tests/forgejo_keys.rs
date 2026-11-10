//! The Forgejo CLI's sign-ins: which hosts they make Forgejo instances, and
//! the token each is read with.
use super::*;
use crate::forgejo::{
    self, Api, Scheme,
    cli_keys::{Keys, Known, paths},
};
use crate::repo::{Remote, parse_remote_url};
use secrecy::ExposeSecret;

const KEYS: &str = r#"{
  "hosts": {
    "git.example.com": {"type": "Application", "token": "abc123"},
    "Forge.example.org:8443": {
      "type": "OAuth",
      "token": "oauth456",
      "refresh_token": "refresh",
      "expires_at": [2026, 320, 12, 0, 0, 0, 0, 0, 0]
    },
    "broken.example.com": {"type": "Application", "token": "two words"}
  },
  "aliases": {"localhost:2222": "git.example.com"},
  "default_ssh": []
}"#;

fn keys() -> Keys {
    Keys::parse(KEYS).unwrap()
}

fn remote(url: &str) -> Remote {
    Remote {
        name: "origin".into(),
        url: url.into(),
        source: parse_remote_url(url).unwrap(),
    }
}

fn known(names: &[&str]) -> Option<Known> {
    keys().instance(names.iter().map(|name| (*name).to_owned()))
}

#[test]
fn a_sign_in_of_either_kind_gives_its_token() {
    let keys = keys();
    let token = |instance| keys.token(instance).map(|token| token.expose_secret());
    assert_eq!(token("git.example.com"), Some("abc123"));
    assert_eq!(token("forge.example.org:8443"), Some("oauth456"));
    assert_eq!(token("other.example.com"), None);
    assert_eq!(
        token("broken.example.com"),
        None,
        "not one an HTTP header can carry"
    );
    assert!(!format!("{keys:?}").contains("abc123"));
}

#[test]
fn a_file_in_another_form_holds_no_sign_in() {
    assert!(Keys::parse("hosts:\n  git.example.com: {}\n").is_none());
    let empty = Keys::parse("{\"hosts\": {}}").unwrap();
    assert_eq!(empty.instance(["git.example.com".to_owned()]), None);
}

#[test]
fn an_alias_names_the_instance_it_stands_for() {
    assert_eq!(
        known(&["localhost:2222", "localhost"]),
        Some(Known {
            instance: "git.example.com".into(),
            alias: true,
        })
    );
    assert_eq!(
        known(&["GIT.example.com"]),
        Some(Known {
            instance: "git.example.com".into(),
            alias: false,
        })
    );
    assert_eq!(known(&["localhost"]), None, "the alias names its port");
}

#[test]
fn a_remote_fj_knows_is_recognized_as_forgejo() {
    for url in [
        "git@git.example.com:team/app.git",
        "https://git.example.com/team/app",
        "ssh://git@localhost:2222/team/app.git",
    ] {
        let mut remote = remote(url);
        let key = remote.source.clone();
        assert_eq!(key.provider, Provider::Github, "{url}");
        assert!(forgejo::recognize(&mut remote, keys), "{url}");
        assert_eq!(remote.source.provider, Provider::Forgejo, "{url}");
        assert_eq!(
            (remote.source.host, remote.source.repository),
            (key.host, key.repository),
            "the host and path keep their key"
        );
    }
}

#[test]
fn only_a_remote_read_as_github_elsewhere_asks_fj() {
    let mut unknown = remote("git@code.example.com:team/app.git");
    assert!(!forgejo::recognize(&mut unknown, keys));
    assert_eq!(unknown.source.provider, Provider::Github);
    for url in [
        "git@github.com:team/app.git",
        "git@gitlab.com:team/app.git",
        "https://codeberg.org/team/app",
    ] {
        let mut remote = remote(url);
        let provider = remote.source.provider;
        assert!(!forgejo::recognize(&mut remote, || unreachable!()), "{url}");
        assert_eq!(remote.source.provider, provider, "{url}");
    }
}

#[test]
fn a_host_named_for_forgejo_needs_no_sign_in() {
    for (url, host) in [
        ("https://codeberg.org/team/app.git", "codeberg.org"),
        (
            "git@forgejo.example.com:team/app.git",
            "forgejo.example.com",
        ),
    ] {
        let key = parse_remote_url(url).unwrap();
        assert_eq!((key.provider, key.host.as_str()), (Provider::Forgejo, host));
    }
    assert!(!forgejo::named("git.example.com"));
}

#[test]
fn the_api_is_the_instance_fj_names_for_the_remote() {
    let api = |url: &str| Api::new(&parse_remote_url(url).unwrap(), Some(url), &keys());
    let aliased = api("ssh://git@localhost:2222/team/app.git");
    assert_eq!(
        (aliased.instance.as_str(), aliased.scheme),
        ("git.example.com", Scheme::Https)
    );
    let own = api("git@git.example.com:team/app.git");
    assert_eq!(
        (own.instance.as_str(), own.scheme),
        ("git.example.com", Scheme::Https)
    );
    let ported = api("https://forge.example.org:8443/team/app");
    assert_eq!(
        ported.instance, "Forge.example.org:8443",
        "as fj names it, so its token is found"
    );
    assert!(keys().token(&ported.instance).is_some());
    let plain = api("http://forgejo.lan:3000/team/app");
    assert_eq!(
        (plain.instance.as_str(), plain.scheme),
        ("forgejo.lan:3000", Scheme::Http)
    );
}

#[test]
fn a_source_that_is_not_the_shown_remote_is_read_at_its_own_host() {
    let source = parse_remote_url("git@git.example.com:team/app.git").unwrap();
    let api = Api::new(&source, None, &Keys::default());
    assert_eq!(
        (api.instance.as_str(), api.scheme),
        ("git.example.com", Scheme::Https)
    );
}

#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn the_sign_ins_are_looked_for_in_fjs_data_folder() {
    use std::{ffi::OsString, path::Path, path::PathBuf};
    let home = Some(Path::new("/home/me"));
    let variable = |value: &'static str| {
        move |name: &str| (name == "XDG_DATA_HOME").then(|| OsString::from(value))
    };
    assert_eq!(
        paths(|_| None, home),
        [PathBuf::from("/home/me/.local/share/forgejo-cli/keys.json")]
    );
    assert_eq!(
        paths(variable("/data"), home),
        [PathBuf::from("/data/forgejo-cli/keys.json")]
    );
    assert_eq!(
        paths(variable("relative"), home),
        [PathBuf::from("/home/me/.local/share/forgejo-cli/keys.json")],
        "a relative XDG folder is not one"
    );
    assert!(paths(|_| None, None).is_empty());
}
