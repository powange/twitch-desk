//! Jetons Twitch : dans le Gestionnaire d'identification Windows (ou le Secret
//! Service sous Linux). Si le trousseau du système est indisponible (WSL, Linux
//! sans Secret Service), repli sur un fichier lisible par l'utilisateur seul ; sous
//! Windows, chaque valeur y est en plus chiffrée pour la session (DPAPI).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use crate::twitch::auth::Tokens;

const SERVICE: &str = "twitch-desk";
/// Valeur chiffrée dans le fichier de secours (préfixe, puis base64)
const PROTECTED: &str = "dpapi:";

/// Chiffrement des valeurs du fichier de secours, lié à la session Windows (DPAPI) : le
/// fichier copié ailleurs ou lu par un autre utilisateur est inutilisable
#[cfg(windows)]
mod protect {
    use base64::Engine;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

    pub const AVAILABLE: bool = true;

    fn take(blob: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        // Mémoire allouée par Windows : copiée puis rendue
        let bytes = unsafe { std::slice::from_raw_parts(blob.pbData, blob.cbData as usize) }.to_vec();
        unsafe {
            let _ = LocalFree(Some(HLOCAL(blob.pbData as _)));
        }
        bytes
    }

    pub fn seal(value: &str) -> Result<String, String> {
        let input = CRYPT_INTEGER_BLOB { cbData: value.len() as u32, pbData: value.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe { CryptProtectData(&input, windows::core::PCWSTR::null(), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }
            .map_err(|e| e.to_string())?;
        Ok(base64::engine::general_purpose::STANDARD.encode(take(out)))
    }

    pub fn open(sealed: &str) -> Result<String, String> {
        let mut data = base64::engine::general_purpose::STANDARD.decode(sealed).map_err(|e| e.to_string())?;
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_mut_ptr() };
        let mut out = CRYPT_INTEGER_BLOB::default();
        unsafe { CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out) }.map_err(|e| e.to_string())?;
        String::from_utf8(take(out)).map_err(|e| e.to_string())
    }
}

/// Ailleurs que sous Windows : fichier lisible par l'utilisateur seul (0600), sans chiffrement
#[cfg(not(windows))]
mod protect {
    pub const AVAILABLE: bool = false;

    pub fn seal(value: &str) -> Result<String, String> {
        Ok(value.to_owned())
    }

    pub fn open(_sealed: &str) -> Result<String, String> {
        Err("valeur chiffrée sous Windows, illisible ici".into())
    }
}

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
        let stored: HashMap<String, String> =
            std::fs::read_to_string(&self.fallback).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let mut plain = false;
        let all: HashMap<String, String> = stored
            .into_iter()
            .filter_map(|(k, v)| match v.strip_prefix(PROTECTED) {
                Some(sealed) => match protect::open(sealed) {
                    Ok(value) => Some((k, value)),
                    Err(e) => {
                        tracing::warn!("secret {k} illisible ({e})");
                        None
                    }
                },
                None => {
                    plain = true;
                    Some((k, v))
                }
            })
            .collect();
        // Valeurs écrites en clair par une version précédente : chiffrées tout de suite
        if plain && protect::AVAILABLE {
            if let Err(e) = self.write_file(&all) {
                tracing::warn!("secrets : chiffrement du fichier de secours impossible ({e})");
            }
        }
        all
    }

    /// Écriture atomique (fichier temporaire puis renommage), lisible par l'utilisateur seul
    /// dès sa création
    fn write_file(&self, all: &HashMap<String, String>) -> Result<(), String> {
        use std::io::Write;
        let stored: HashMap<&String, String> = all
            .iter()
            .map(|(k, v)| Ok((k, if protect::AVAILABLE { format!("{PROTECTED}{}", protect::seal(v)?) } else { v.clone() })))
            .collect::<Result<_, String>>()?;
        let text = serde_json::to_string(&stored).map_err(|e| e.to_string())?;
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
