use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
};

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use serde::{Deserialize, Serialize};

use crate::auth::{error::AuthError, keys::random_string};

/// A machine allowed to ask for tokens. Its secret is only ever stored hashed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Client {
    pub client_id: String,
    /// Argon2id hash in PHC string form.
    pub secret_hash: String,
    pub scopes: Vec<String>,
}

/// The registered clients, kept in a JSON file.
#[derive(Debug, Clone, Default)]
pub struct ClientRegistry {
    clients: Vec<Client>,
}

impl ClientRegistry {
    pub fn load(path: &Path) -> Result<Self, AuthError> {
        let invalid = |reason: String| AuthError::ClientsFile {
            path: path.display().to_string(),
            reason,
        };

        let text = fs::read_to_string(path).map_err(|error| io_error(path, error))?;
        let clients: Vec<Client> =
            serde_json::from_str(&text).map_err(|error| invalid(error.to_string()))?;

        let mut registry = Self::default();
        for client in clients {
            validate_client_id(&client.client_id).map_err(|e| invalid(e.to_string()))?;
            validate_scopes(&client.scopes).map_err(|e| invalid(e.to_string()))?;
            PasswordHash::new(&client.secret_hash).map_err(|_| {
                invalid(format!(
                    "{}: secret_hash is not an Argon2 hash",
                    client.client_id
                ))
            })?;
            if registry.get(&client.client_id).is_some() {
                return Err(invalid(format!("{} is listed twice", client.client_id)));
            }
            registry.clients.push(client);
        }

        Ok(registry)
    }

    /// Writes the registry, readable by the owner only. It goes to a temporary file that is
    /// then renamed over the real one, so a crash cannot leave a half-written registry.
    pub fn save(&self, path: &Path) -> Result<(), AuthError> {
        let json = serde_json::to_string_pretty(&self.clients).expect("clients serialize") + "\n";
        let mut temporary = path.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);

        let _ = fs::remove_file(&temporary);
        write_private(&temporary, json.as_bytes())?;
        fs::rename(&temporary, path).map_err(|error| io_error(path, error))
    }

    /// Registers a client and returns its secret, which cannot be recovered afterwards.
    pub fn create(&mut self, client_id: &str, scopes: &[String]) -> Result<String, AuthError> {
        validate_client_id(client_id)?;
        validate_scopes(scopes)?;
        if self.get(client_id).is_some() {
            return Err(AuthError::DuplicateClient(client_id.to_owned()));
        }

        let secret = random_string(32)?;
        self.clients.push(Client {
            client_id: client_id.to_owned(),
            secret_hash: hash_secret(&secret)?,
            scopes: scopes.to_vec(),
        });

        Ok(secret)
    }

    pub fn get(&self, client_id: &str) -> Option<&Client> {
        self.clients
            .iter()
            .find(|client| client.client_id == client_id)
    }

    pub fn clients(&self) -> &[Client] {
        &self.clients
    }

    /// The client, if the id and secret match. An unknown id costs the same hash check as a
    /// wrong secret, so response time does not reveal which ids exist.
    pub fn authenticate(&self, client_id: &str, secret: &str) -> Option<&Client> {
        let client = self.get(client_id);
        let hash: &str = match client {
            Some(client) => &client.secret_hash,
            None => decoy_hash(),
        };

        let matches = PasswordHash::new(hash).is_ok_and(|hash| {
            Argon2::default()
                .verify_password(secret.as_bytes(), &hash)
                .is_ok()
        });

        client.filter(|_| matches)
    }
}

fn hash_secret(secret: &str) -> Result<String, AuthError> {
    Argon2::default()
        .hash_password(secret.as_bytes())
        .map(|hash| hash.to_string())
        .map_err(|_| AuthError::Hash)
}

/// A hash no secret matches, verified against when the client id is unknown.
fn decoy_hash() -> &'static str {
    static DECOY: OnceLock<String> = OnceLock::new();

    DECOY.get_or_init(|| {
        let secret = random_string(32).unwrap_or_default();
        hash_secret(&secret).unwrap_or_default()
    })
}

pub(crate) fn validate_client_id(client_id: &str) -> Result<(), AuthError> {
    let valid = (1..=64).contains(&client_id.len())
        && client_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));

    valid
        .then_some(())
        .ok_or_else(|| AuthError::InvalidClientId(client_id.to_owned()))
}

pub(crate) fn validate_scopes(scopes: &[String]) -> Result<(), AuthError> {
    if scopes.is_empty() {
        return Err(AuthError::NoScopes);
    }
    for scope in scopes {
        let valid = !scope.is_empty()
            && scope
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || ":._-".contains(c));
        if !valid {
            return Err(AuthError::InvalidScope(scope.clone()));
        }
    }

    Ok(())
}

pub(crate) fn io_error(path: &Path, error: std::io::Error) -> AuthError {
    AuthError::Io {
        path: path.display().to_string(),
        reason: error.to_string(),
    }
}

/// Writes `bytes` to a new file at `path`, readable by the owner only. It refuses to replace
/// a file that already exists.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), AuthError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);

    let mut file = options.open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            AuthError::AlreadyExists(path.display().to_string())
        } else {
            io_error(path, error)
        }
    })?;

    file.write_all(bytes).map_err(|error| io_error(path, error))
}

/// Registers `client_id` in the clients file, creating the file if need be, and returns the
/// new client's secret. The secret is shown once: only its hash is stored.
pub fn create_client(path: &Path, client_id: &str, scopes: &[String]) -> Result<String, AuthError> {
    let mut registry = if path.exists() {
        ClientRegistry::load(path)?
    } else {
        ClientRegistry::default()
    };
    let secret = registry.create(client_id, scopes)?;
    registry.save(path)?;

    Ok(secret)
}

pub fn list_clients(path: &Path) -> Result<Vec<Client>, AuthError> {
    Ok(ClientRegistry::load(path)?.clients().to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scopes(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn registry_with_one() -> (ClientRegistry, String) {
        let mut registry = ClientRegistry::default();
        let secret = registry
            .create("assistant", &scopes(&["woods:write"]))
            .unwrap();

        (registry, secret)
    }

    #[test]
    fn a_new_client_gets_a_long_random_secret_stored_only_as_an_argon2id_hash() {
        let (registry, secret) = registry_with_one();

        let client = registry.get("assistant").unwrap();

        assert_eq!(secret.len(), 43);
        assert!(client.secret_hash.starts_with("$argon2id$"));
        assert!(!client.secret_hash.contains(&secret));
        assert_eq!(client.scopes, ["woods:write"]);
        assert_eq!(registry.clients().len(), 1);
    }

    #[test]
    fn only_the_right_id_and_secret_authenticate() {
        let (registry, secret) = registry_with_one();

        assert_eq!(
            registry
                .authenticate("assistant", &secret)
                .unwrap()
                .client_id,
            "assistant"
        );
        assert!(registry.authenticate("assistant", "wrong").is_none());
        assert!(registry.authenticate("assistant", "").is_none());
        assert!(
            registry
                .authenticate("assistant", &secret.to_uppercase())
                .is_none()
        );
        assert!(registry.authenticate("someone-else", &secret).is_none());
        assert!(registry.authenticate("", "").is_none());
    }

    #[test]
    fn a_secret_is_not_valid_for_another_client() {
        let mut registry = ClientRegistry::default();
        let first = registry.create("first", &scopes(&["woods:write"])).unwrap();
        let second = registry
            .create("second", &scopes(&["woods:write"]))
            .unwrap();

        assert!(registry.authenticate("first", &first).is_some());
        assert!(registry.authenticate("first", &second).is_none());
        assert!(registry.authenticate("second", &first).is_none());
        assert_ne!(first, second);
    }

    #[test]
    fn bad_client_ids_scopes_and_duplicates_are_refused() {
        let mut registry = ClientRegistry::default();
        let long = "a".repeat(65);
        for id in [
            "",
            "has space",
            "slash/ed",
            "naïve",
            "semi;colon",
            long.as_str(),
        ] {
            let error = registry.create(id, &scopes(&["woods:write"])).unwrap_err();
            assert!(matches!(error, AuthError::InvalidClientId(_)), "{id:?}");
        }
        assert!(
            registry
                .create(&"a".repeat(64), &scopes(&["woods:write"]))
                .is_ok()
        );
        assert!(
            registry
                .create("Dots.and_dashes-1", &scopes(&["a"]))
                .is_ok()
        );

        assert!(matches!(
            registry.create("x", &[]).unwrap_err(),
            AuthError::NoScopes
        ));
        for scope in ["", "Woods:write", "has space", "woods/write", "écrire"] {
            let error = registry.create("y", &scopes(&[scope])).unwrap_err();
            assert!(matches!(error, AuthError::InvalidScope(_)), "{scope:?}");
        }
        let error = registry
            .create("Dots.and_dashes-1", &scopes(&["a"]))
            .unwrap_err();
        assert!(matches!(error, AuthError::DuplicateClient(id) if id == "Dots.and_dashes-1"));
    }

    #[test]
    fn a_saved_registry_loads_back_identically_and_is_private() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clients.json");
        let (registry, secret) = registry_with_one();

        registry.save(&path).unwrap();
        let loaded = ClientRegistry::load(&path).unwrap();

        assert_eq!(loaded.clients(), registry.clients());
        assert!(loaded.authenticate("assistant", &secret).is_some());
        assert!(!std::fs::read_to_string(&path).unwrap().contains(&secret));
        assert!(
            !dir.path().join("clients.json.tmp").exists(),
            "no temporary file is left behind"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn saving_replaces_the_old_file_and_survives_a_stale_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clients.json");
        std::fs::write(
            dir.path().join("clients.json.tmp"),
            "left over from a crash",
        )
        .unwrap();
        let (mut registry, _) = registry_with_one();
        registry.save(&path).unwrap();

        registry
            .create("second", &scopes(&["woods:write"]))
            .unwrap();
        registry.save(&path).unwrap();

        assert_eq!(ClientRegistry::load(&path).unwrap().clients().len(), 2);
    }

    #[test]
    fn saving_to_a_missing_directory_reports_the_path() {
        let (registry, _) = registry_with_one();

        let error = registry
            .save(Path::new("/no/such/directory/clients.json"))
            .unwrap_err();

        assert!(matches!(&error, AuthError::Io { path, .. } if path.contains("clients.json")));
    }

    #[test]
    fn a_broken_clients_file_is_refused_with_its_path_and_the_reason() {
        let dir = tempfile::tempdir().unwrap();
        let hash = registry_with_one().0.clients()[0].secret_hash.clone();
        let entry = |id: &str, hash: &str, scope: &str| {
            format!(r#"{{"client_id":"{id}","secret_hash":"{hash}","scopes":["{scope}"]}}"#)
        };
        let cases = [
            ("not json".to_owned(), "expected"),
            ("{}".to_owned(), "invalid type"),
            (
                format!("[{}]", entry("bad id", &hash, "a")),
                "invalid client id",
            ),
            (format!("[{}]", entry("ok", &hash, "BAD")), "invalid scope"),
            (
                format!("[{}]", entry("ok", "plaintext-secret", "a")),
                "not an Argon2 hash",
            ),
            (
                format!("[{0},{0}]", entry("twice", &hash, "a")),
                "listed twice",
            ),
            (
                r#"[{"client_id":"ok","secret_hash":"x","scopes":[]}]"#.to_owned(),
                "at least one scope",
            ),
        ];

        for (index, (text, reason)) in cases.into_iter().enumerate() {
            let path = dir.path().join(format!("clients-{index}.json"));
            std::fs::write(&path, text).unwrap();

            let error = ClientRegistry::load(&path).unwrap_err();

            assert!(matches!(&error, AuthError::ClientsFile { .. }), "{error}");
            assert!(
                error.to_string().contains(&path.display().to_string()),
                "{error}"
            );
            assert!(error.to_string().contains(reason), "{reason}: {error}");
        }
    }

    #[test]
    fn a_missing_clients_file_is_an_io_error() {
        let error = ClientRegistry::load(Path::new("/no/such/clients.json")).unwrap_err();

        assert!(matches!(error, AuthError::Io { .. }));
    }

    #[test]
    fn create_client_starts_the_file_then_adds_to_it_and_list_shows_who_is_registered() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("clients.json");

        let first = create_client(&path, "assistant", &scopes(&["woods:write"])).unwrap();
        let second =
            create_client(&path, "importer", &scopes(&["woods:write", "woods:read"])).unwrap();

        let listed = list_clients(&path).unwrap();
        assert_eq!(
            listed
                .iter()
                .map(|c| c.client_id.as_str())
                .collect::<Vec<_>>(),
            ["assistant", "importer"]
        );
        assert_eq!(listed[1].scopes, ["woods:write", "woods:read"]);
        let registry = ClientRegistry::load(&path).unwrap();
        assert!(registry.authenticate("assistant", &first).is_some());
        assert!(registry.authenticate("importer", &second).is_some());
        assert!(matches!(
            create_client(&path, "assistant", &scopes(&["woods:write"])).unwrap_err(),
            AuthError::DuplicateClient(_)
        ));
    }

    #[test]
    fn the_hash_check_runs_for_unknown_clients_too() {
        // The decoy hash is a valid Argon2 hash, so an unknown id costs a real verification.
        assert!(PasswordHash::new(decoy_hash()).is_ok());
        assert_eq!(decoy_hash(), decoy_hash());
    }
}
