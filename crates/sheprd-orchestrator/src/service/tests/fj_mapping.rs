//! A remote the Forgejo CLI comes to know after the repository was found,
//! through a sign-in or an alias added later, is read as Forgejo from the
//! next sync on, without reopening the view.
use super::*;

#[test]
fn a_sign_in_added_after_opening_makes_the_remote_a_forgejo_source() {
    let dir = tempfile::tempdir().unwrap();
    let (_service, mut worker, _host) = opened(dir.path());
    worker.recognize(Vec::new, Default::default);
    let source = |worker: &Worker| worker.snapshot.sources[0].clone();
    assert_eq!(source(&worker).key.provider, Provider::Github);
    assert!(matches!(source(&worker).state, SyncState::Unsupported));

    let keys = r#"{"hosts": {"github.acme.com": {"type": "Application", "token": "abc"}}}"#;
    worker.recognize(Vec::new, || forgejo::cli_keys::Keys::parse(keys).unwrap());
    let status = source(&worker);
    let expected = SourceKey {
        provider: Provider::Forgejo,
        host: "github.acme.com".into(),
        repository: "group/app".into(),
    };
    assert_eq!(status.key, expected, "the host and path keep their key");
    assert!(matches!(status.state, SyncState::Syncing { .. }));

    // Once Forgejo, neither CLI's files are read again.
    worker.recognize(|| unreachable!(), || unreachable!());
    assert_eq!(source(&worker).key, expected);
}
