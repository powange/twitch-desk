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
        if Self::keyring_available() {
            if let Ok(entry) = keyring::Entry::new(SERVICE, user_id) {
                if entry.set_password(&value).is_ok() {
                    return Ok(());
                }
            }
        }
        let _guard = self.lock.lock().unwrap();
        let mut all = self.read_file();
        all.insert(user_id.to_owned(), value);
        self.write_file(&all)
    }

    pub fn load(&self, user_id: &str) -> Option<Tokens> {
        if Self::keyring_available() {
            if let Ok(value) = keyring::Entry::new(SERVICE, user_id).and_then(|e| e.get_password()) {
                return serde_json::from_str(&value).ok();
            }
        }
        let _guard = self.lock.lock().unwrap();
        self.read_file().get(user_id).and_then(|v| serde_json::from_str(v).ok())
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

    fn write_file(&self, all: &HashMap<String, String>) -> Result<(), String> {
        let text = serde_json::to_string(all).map_err(|e| e.to_string())?;
        std::fs::write(&self.fallback, text).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.fallback, std::fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }
}
