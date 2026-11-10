use super::*;
use sheprd_orchestrator::forgejo;

#[test]
fn every_forgejo_variant_has_a_message() {
    let repository = || "owner/app".to_owned();
    assert_localized(&[
        forgejo::Error::Address("owner app".into()),
        forgejo::Error::Authentication,
        forgejo::Error::Forbidden {
            repository: repository(),
        },
        forgejo::Error::InsecureRemote {
            repository: repository(),
        },
        forgejo::Error::Json(serde_json::from_str::<u8>("x").unwrap_err()),
        forgejo::Error::Network {
            origin: "https://codeberg.org".into(),
            source: Box::new(ureq::Error::HostNotFound),
        },
        forgejo::Error::NotFound {
            repository: repository(),
        },
        forgejo::Error::RateLimit,
        forgejo::Error::Read(std::io::Error::other("pipe closed")),
        forgejo::Error::SignIn {
            repository: repository(),
        },
        forgejo::Error::Size,
        forgejo::Error::Status(500),
        forgejo::Error::Token,
    ]);
}
