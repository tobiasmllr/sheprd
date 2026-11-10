//! The Forgejo CLI's sign-ins (`fj auth login`, `fj auth add-token`), read for
//! which hosts are Forgejo instances and the token each is read with.
//!
//! fj keeps them as JSON in `keys.json` in its data folder: `hosts`, a token
//! for each instance by its name, and `aliases`, the instance another host
//! stands for, which fj fills in at sign-in with the SSH host the instance's
//! repositories are cloned from when that is not the instance's own. An
//! instance's name is its host, with a port or a path when it is served at
//! one. A token fj got through OAuth is used as kept: fj renews it when it
//! next runs.

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    ffi::OsString,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

/// A few instances take a few hundred bytes.
const LIMIT: u64 = 256 * 1024;

/// fj's sign-ins and aliases.
#[derive(Debug, Default, Deserialize)]
pub struct Keys {
    #[serde(default)]
    hosts: BTreeMap<String, Login>,
    #[serde(default)]
    aliases: BTreeMap<String, String>,
}

/// One instance's sign-in, pasted or from OAuth: either kind carries a token.
#[derive(Debug, Deserialize)]
struct Login {
    token: SecretString,
}

/// The instance fj knows a host as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Known {
    /// The instance's name, which fj keeps its token under.
    pub instance: String,
    /// Whether the host is another's alias rather than the instance itself.
    pub alias: bool,
}

impl Keys {
    /// fj's `keys.json` text; `None` when it is not in fj's form.
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str(text).ok()
    }

    /// The instance fj knows the first of `names` it knows as: the one it
    /// is an alias of, else itself when fj is signed in to it.
    pub fn instance(&self, names: impl IntoIterator<Item = String>) -> Option<Known> {
        names.into_iter().find_map(|name| {
            let alias = find(&self.aliases, &name).map(|(_, instance)| Known {
                instance: instance.clone(),
                alias: true,
            });
            alias.or_else(|| {
                find(&self.hosts, &name).map(|(instance, _)| Known {
                    instance: instance.clone(),
                    alias: false,
                })
            })
        })
    }

    /// The token fj is signed in to `instance` with, when it is one an HTTP
    /// header can carry.
    pub fn token(&self, instance: &str) -> Option<&SecretString> {
        find(&self.hosts, instance)
            .map(|(_, login)| &login.token)
            .filter(|token| valid(token.expose_secret()))
    }
}

/// The entry named `name`, whatever its letters' case.
fn find<'a, V>(entries: &'a BTreeMap<String, V>, name: &str) -> Option<(&'a String, &'a V)> {
    entries
        .iter()
        .find(|(entry, _)| entry.eq_ignore_ascii_case(name))
}

/// A token is a short run of visible ASCII; anything else would corrupt the
/// header or is not a token at all.
fn valid(token: &str) -> bool {
    !token.is_empty() && token.len() <= 1024 && token.bytes().all(|byte| byte.is_ascii_graphic())
}

/// Where fj may keep `keys.json`, in the order it looks: its data folder,
/// then the one releases before its rename used. On Linux both are
/// `forgejo-cli` in `XDG_DATA_HOME`.
pub fn paths(variable: impl Fn(&str) -> Option<OsString>, home: Option<&Path>) -> Vec<PathBuf> {
    let set = |name| variable(name).filter(|value| !value.is_empty());
    let folders: Vec<PathBuf> = if cfg!(target_os = "macos") {
        let Some(support) = home.map(|home| home.join("Library/Application Support")) else {
            return Vec::new();
        };
        vec![
            support.join("forgejo-cli.forgejo-cli"),
            support.join("Cyborus.forgejo-cli"),
        ]
    } else if cfg!(windows) {
        let Some(roaming) = set("APPDATA").map(PathBuf::from) else {
            return Vec::new();
        };
        vec![
            roaming.join("forgejo-cli").join("forgejo-cli").join("data"),
            roaming.join("Cyborus").join("forgejo-cli").join("data"),
        ]
    } else {
        let data = set("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| home.map(|home| home.join(".local/share")));
        data.map(|data| data.join("forgejo-cli"))
            .into_iter()
            .collect()
    };
    folders
        .into_iter()
        .map(|folder| folder.join("keys.json"))
        .collect()
}

/// fj's sign-ins on this machine, or none when it has no readable file. The
/// variables come from the login shell, so a launch from a desktop entry
/// finds the same file as a terminal's `fj`. Reads a file and may probe the
/// shell: call it from a worker.
pub fn read() -> Keys {
    let home = sheprd_client::login_env::var("HOME")
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
        .or_else(sheprd_client::local_path::home_dir);
    paths(sheprd_client::login_env::var, home.as_deref())
        .iter()
        .find_map(|path| read_file(path))
        .unwrap_or_default()
}

/// The sign-ins in the file at `path`; `None` when there is no such file, so
/// the next place is tried.
fn read_file(path: &Path) -> Option<Keys> {
    let mut text = zeroize::Zeroizing::new(String::new());
    let read = File::open(path).and_then(|file| file.take(LIMIT + 1).read_to_string(&mut text));
    match read {
        Ok(size) if size as u64 <= LIMIT => Some(Keys::parse(&text).unwrap_or_default()),
        Ok(_) => Some(Keys::default()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            tracing::debug!(
                category = "orchestrator_forgejo",
                %error,
                "The Forgejo CLI's sign-ins could not be read"
            );
            Some(Keys::default())
        }
    }
}
