//! Janitor du temp : lavoir ne garde rien sur le disque.
//!
//! Trois passes, comme prévu au TODO : purge complète au démarrage (résidus
//! d'une session précédente) et à la fermeture, plus un balayage toutes les 60 s
//! des dossiers de téléchargement abandonnés (crash, `.part` orphelins) — sans
//! jamais toucher un téléchargement en cours, reconnu par son identifiant.
//!
//! La purge au démarrage est sûre malgré single-instance : une seconde instance
//! s'arrête dans le `setup` du plugin (`std::process::exit`) avant d'atteindre
//! le nôtre, donc ce code ne tourne que dans l'instance primaire.

use std::collections::HashSet;
use std::time::{Duration, SystemTime};

use tauri::{AppHandle, Manager};

use crate::recuperer::{self, temp_root};

const MAX_AGE: Duration = Duration::from_secs(10 * 60);
const SWEEP_INTERVAL: Duration = Duration::from_secs(60);

/// Efface tout le temp de l'app (cache yt-dlp compris). Best-effort : un fichier
/// encore verrouillé sera repris au balayage suivant ou au prochain démarrage.
pub fn purge_all() {
    let _ = std::fs::remove_dir_all(temp_root());
}

pub fn spawn_periodic(app: AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(SWEEP_INTERVAL);
        let active = app.state::<recuperer::DownloadManager>().active_ids();
        sweep(&active);
    });
}

/// Supprime les dossiers de `dl/` qui ne correspondent à aucun téléchargement
/// actif et qui dépassent 10 minutes. Le cache yt-dlp, lui, n'est purgé qu'au
/// démarrage et à la fermeture : le supprimer en pleine session gênerait un
/// téléchargement en cours pour un gain nul.
fn sweep(active: &HashSet<String>) {
    let Ok(entries) = std::fs::read_dir(temp_root().join("dl")) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let mtime = entry.metadata().and_then(|m| m.modified()).ok();
        if should_remove(&name.to_string_lossy(), mtime, active, now, MAX_AGE) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Décision pure (testable) : on supprime un dossier de téléchargement s'il
/// n'est pas actif et que sa date dépasse `age`. Une date illisible est traitée
/// comme récente (on ne supprime pas dans le doute).
fn should_remove(
    name: &str,
    mtime: Option<SystemTime>,
    active: &HashSet<String>,
    now: SystemTime,
    age: Duration,
) -> bool {
    if active.contains(name) {
        return false;
    }
    mtime
        .and_then(|t| now.duration_since(t).ok())
        .map(|elapsed| elapsed > age)
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spares_active_and_recent_reaps_stale() {
        let active: HashSet<String> = ["dl-7".to_string()].into_iter().collect();
        let now = SystemTime::now();
        let stale = now - Duration::from_secs(20 * 60);
        let recent = now - Duration::from_secs(30);

        // Actif : jamais supprimé, même vieux.
        assert!(!should_remove("dl-7", Some(stale), &active, now, MAX_AGE));
        // Inactif et vieux : supprimé.
        assert!(should_remove("dl-3", Some(stale), &active, now, MAX_AGE));
        // Inactif mais récent : épargné (téléchargement peut-être hors map un instant).
        assert!(!should_remove("dl-3", Some(recent), &active, now, MAX_AGE));
        // Date illisible : épargné.
        assert!(!should_remove("dl-3", None, &active, now, MAX_AGE));
    }
}
