//! Jetons Twitch : dans le Gestionnaire d'identification Windows (ou le Secret
//! Service sous Linux). Si le trousseau du système est indisponible (WSL, Linux
//! sans Secret Service), repli sur un fichier lisible par l'utilisateur seul.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::twitch::auth::Tokens;

const SERVICE: &str = "twitch-desk";

pub struct Secrets {
    fallback: PathBuf,
    lock: Mutex<()>,
}

impl Secrets {
    pub fn new(data_dir: PathBuf) -> Self {
        Self { fallback: data_dir.join("secrets.json"), lock: Mutex::new(()) }
    }

    fn keyring_available() -> bool {
        keyring::Entry::store_status().is_ok()
    }

    pub fn save(&self, user_id: &str, tokens: &Tokens) -> Result<(), String> {
        let value = serde_json::to_string(tokens).map_err(|e| e.to_string())?;
        self.save_raw(user_id, &value)
    }

    pub fn load(&self, user_id: &str) -> Option<Tokens> {
        self.load_raw(user_id).and_then(|v| serde_json::from_str(&v).ok())
    }

    /// Jeton StreamElements d'un compte Twitch
    pub fn se_key(user_id: &str) -> String {
        format!("se:{user_id}")
    }

    pub fn save_raw(&self, key: &str, value: &str) -> Result<(), String> {
        if Self::keyring_available() {
            match keyring::Entry::new(SERVICE, key) {
                Ok(entry) => match entry.set_password(value) {
                    Ok(()) => return Ok(()),
                    Err(e) => {
                        tracing::warn!("trousseau du système refusé pour {key} ({e}) : repli sur le fichier");
                        // L'ancienne valeur du trousseau serait relue avant celle du fichier
                        let _ = entry.delete_credential();
                    }
                },
                Err(e) => tracing::warn!("trousseau du système indisponible pour {key} ({e}) : repli sur le fichier"),
            }
        }
        let _guard = self.lock.lock().unwrap();
        let mut all = self.read_file();
        all.insert(key.to_owned(), value.to_owned());
        self.write_file(&all)
    }

    pub fn load_raw(&self, key: &str) -> Option<String> {
        if Self::keyring_available() {
            if let Ok(value) = keyring::Entry::new(SERVICE, key).and_then(|e| e.get_password()) {
                return Some(value);
            }
        }
        let _guard = self.lock.lock().unwrap();
        self.read_file().get(key).cloned()
    }

    pub fn delete(&self, user_id: &str) {
        if Self::keyring_available() {
            if let Ok(entry) = keyring::Entry::new(SERVICE, user_id) {
                let _ = entry.delete_credential();
            }
        }
        let _guard = self.lock.lock().unwrap();
        let mut all = self.read_file();
        if all.remove(user_id).is_some() {
            let _ = self.write_file(&all);
        }
    }

    fn read_file(&self) -> HashMap<String, String> {
        std::fs::read_to_string(&self.fallback)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Écriture atomique (fichier temporaire puis renommage), lisible par l'utilisateur seul
    /// dès sa création
    fn write_file(&self, all: &HashMap<String, String>) -> Result<(), String> {
        use std::io::Write;
        let text = serde_json::to_string(all).map_err(|e| e.to_string())?;
        let tmp = self.fallback.with_extension("json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let written = options.open(&tmp).and_then(|mut f| {
            // Fichier temporaire laissé par une version précédente : droits remis à 0o600
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            }
            f.write_all(text.as_bytes())?;
            f.sync_all()
        });
        if let Err(e) = written.and_then(|_| std::fs::rename(&tmp, &self.fallback)) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("twitch-desk-secrets-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let secrets = Secrets::new(dir.clone());
        let mut all = HashMap::new();
        all.insert("k".to_owned(), "v".to_owned());
        secrets.write_file(&all).unwrap();
        assert_eq!(secrets.read_file().get("k").map(String::as_str), Some("v"));
        assert!(!dir.join("secrets.json.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join("secrets.json")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
