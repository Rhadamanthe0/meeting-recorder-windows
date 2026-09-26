//! Cross-platform paths.
//!
//! SEULE source de chemins pour la suite du portage Windows : tout nouveau
//! code (audio, player, ipc, agent, ui) doit passer par ces fonctions au lieu
//! de lire `XDG_*` ou `glib::*_dir()` directement.
//!
//! Sur Linux, comportement XDG/glib identique à l'amont :
//! config `~/.config`, data `~/.local/share`, state `~/.local/state`,
//! cache/runtime via glib, meetings `~/Documents/Meetings`.
//! Sur Windows, dossiers connus via la crate `dirs`
//! (AppData Roaming/Local, Documents) avec repli glib.

use std::path::PathBuf;

use crate::APP_NAME;

#[cfg(target_os = "windows")]
fn dirs_or(home_fallback: &str) -> PathBuf {
    glib_fallback_home().join(home_fallback)
}

#[cfg(target_os = "windows")]
fn glib_fallback_home() -> PathBuf {
    gtk::glib::home_dir()
}

/// Dossier de configuration (`config.toml`).
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        dirs::config_dir().unwrap_or_else(|| dirs_or(".config"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| gtk::glib::home_dir().join(".config"))
    }
}

/// Dossier de données partagées.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        dirs::data_dir().unwrap_or_else(|| dirs_or(".local/share"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| gtk::glib::home_dir().join(".local/share"))
    }
}

/// Dossier d'état (`settings.json`, thèmes).
pub fn state_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        dirs::state_dir()
            .or_else(dirs::data_dir)
            .unwrap_or_else(|| dirs_or(".local/state"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| gtk::glib::home_dir().join(".local/state"))
    }
}

/// Dossier de cache (staging d'enregistrement).
pub fn cache_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        dirs::cache_dir().unwrap_or_else(std::env::temp_dir)
    }
    #[cfg(not(target_os = "windows"))]
    {
        gtk::glib::user_cache_dir()
    }
}

/// Dossier des réunions exportées (`Documents/Meetings`).
pub fn meetings_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        dirs::document_dir()
            .unwrap_or_else(|| dirs_or("Documents"))
            .join("Meetings")
    }
    #[cfg(not(target_os = "windows"))]
    {
        gtk::glib::home_dir().join("Documents/Meetings")
    }
}

/// Dossier des modèles téléchargés (`.../meeting-recorder/models`).
/// Reprend `transcribe::models_dir` de l'amont.
pub fn models_dir() -> PathBuf {
    data_dir().join(APP_NAME).join("models")
}

/// Dossier runtime (socket Unix sur Linux ; sur Windows l'IPC passe par un
/// named pipe, ce dossier n'est qu'un repli pour fichiers temporaires).
pub fn runtime_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::temp_dir().join(APP_NAME)
    }
    #[cfg(not(target_os = "windows"))]
    {
        gtk::glib::user_runtime_dir()
    }
}
