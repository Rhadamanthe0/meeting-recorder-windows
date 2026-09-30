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
use std::process::Command;

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

/// Ouvre une URI (page web, dossier `file://`, `obsidian://`, …) dans
/// l'application par défaut.
///
/// Sur Windows `gio::AppInfo::launch_default_for_uri` échoue silencieusement,
/// donc on passe par `cmd /C start "" <uri>` : le `""` est le titre vide
/// exigé par `start` (sans lui l'URI serait prise pour un titre). Le
/// `CREATE_NO_WINDOW` de [`silent_command`] évite tout flash de console.
/// L'URI est quotée (voir [`quote_for_cmd`]) : sans cela `cmd` couperait
/// l'URI sur `&`/`|`/`^` (`obsidian://open?vault=V&file=F` lancerait `file=F`
/// comme commande) ou étendrait `%var%`. Le `start` détache le navigateur
/// puis termine aussitôt, donc attendre sa fin ne bloque pas l'UI de façon
/// perceptible ; son code de sortie non nul devient `Err` pour que l'appelant
/// puisse toaster l'échec.
/// Sur Unix, simple délégation à gio (comportement amont inchangé).
pub fn open_uri(uri: &str) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let status = silent_command("cmd")
            .args(["/C", "start", ""])
            .raw_arg(quote_for_cmd(uri))
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(std::io::Error::other(format!(
                "`cmd /C start` exited with {status}"
            )))
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        gtk::gio::AppInfo::launch_default_for_uri(uri, None::<&gtk::gio::AppLaunchContext>)
            .map(|_| ())
            .map_err(|e| std::io::Error::other(e.to_string()))
    }
}

/// Quote une URI pour `cmd /C start "" <uri>`.
///
/// `%` est laissé tel quel : à `/C` (contrairement à un fichier batch)
/// `%%` n'est PAS replié en `%`, donc doubler `%` corrompt les URI
/// encodées (`%20`, `%2F`, …) ; revers assumé : `%var%` peut être
/// étendu par `cmd`.
/// `&`/`|`/`^`/`<`/`>` sont littéraux entre guillemets. `"` ne peut pas survivre intact dans une
/// URI entre guillemets `cmd`, il est retiré pour ne pas en sortir.
fn quote_for_cmd(uri: &str) -> String {
    format!("\"{}\"", uri.replace('"', ""))
}

/// Construit un `Command` qui reste invisible sur Windows.
///
/// `ffmpeg`/`ffprobe` (builds Gyan) et `cmd` sont des applications console :
/// sans `CREATE_NO_WINDOW` chaque spawn ouvre un flash de console. Sur Unix,
/// simple passthrough de `Command::new` (args/stdio inchangés aux call sites).
pub fn silent_command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        // CREATE_NO_WINDOW (0x08000000) : pas de console, pas de flash.
        // Constante en littéral pour ne pas ajouter de dépendance.
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_command_keeps_program_and_args() {
        let mut command = silent_command("ffmpeg");
        command.arg("-version");
        let debug = format!("{command:?}");
        assert!(debug.contains("ffmpeg"), "{debug}");
        assert!(debug.contains("-version"), "{debug}");
    }

    #[test]
    fn cmd_quoting_keeps_metachars_and_percent_intact() {
        // `&` doit rester dans l'URI au lieu de séparer deux commandes…
        assert_eq!(
            quote_for_cmd("obsidian://open?vault=V&file=F"),
            "\"obsidian://open?vault=V&file=F\""
        );
        // …`%20` doit survivre tel quel : à `/C` (contrairement à un
        // batch) `%%` n'est PAS replié — `cmd /C echo "a%%20b"` affiche
        // `a%%20b` — donc doubler `%` corrompt les URI encodées…
        assert_eq!(
            quote_for_cmd("obsidian://open?file=a%20b"),
            "\"obsidian://open?file=a%20b\""
        );
        // …et `"` ne doit pas permettre de sortir des guillemets.
        assert_eq!(quote_for_cmd("https://x/a\"|b"), "\"https://x/a|b\"");
    }
}
