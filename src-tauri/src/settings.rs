//! Préférences persistées, dans `%APPDATA%/app.lavoir.desktop/settings.json`.
//!
//! Règle no-trace : ce fichier ne contient que des préférences — jamais d'URL,
//! de nom de fichier téléchargé ni d'historique.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Dossier de destination des téléchargements. `None` → `Téléchargements/lavoir`.
    pub destination: Option<String>,
    /// Navigateur dont réutiliser les cookies (`firefox`/`chrome`/`edge`), ou `None`.
    pub cookies_from_browser: Option<String>,
    /// Vérifier la mise à jour de yt-dlp au démarrage (désactivé par défaut).
    pub check_ytdlp_on_launch: bool,
}

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    Some(dir.join("settings.json"))
}

pub fn load(app: &AppHandle) -> Settings {
    settings_path(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = settings_path(app).ok_or("dossier de configuration introuvable")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("création du dossier de configuration : {e}"))?;
    }
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("écriture des réglages : {e}"))
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
    load(&app)
}

#[tauri::command]
pub fn set_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
    save(&app, &settings)
}
