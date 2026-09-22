//! Récupérer : téléchargement d'un média public via yt-dlp.
//!
//! Le fichier arrive dans notre temp, y est lavé (réutilise le pipeline des
//! phases 2-3), et seule la copie propre est déplacée vers la destination — le
//! fichier brut ne sort jamais du temp. Deux téléchargements simultanés au plus
//! (sémaphore) ; annulation par kill de l'arbre de process (yt-dlp + ffmpeg).
//!
//! No-trace : `--cache-dir` et le fichier en cours vivent sous `temp_root()`
//! (purgé par le janitor de la phase 5) ; aucune URL n'est jamais persistée.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

use crate::{doctor, laver, settings};

/// Racine des fichiers temporaires de l'app. Le janitor de la phase 5 en héritera.
pub fn temp_root() -> PathBuf {
    std::env::temp_dir().join("lavoir")
}

fn cache_dir() -> PathBuf {
    temp_root().join("yt-dlp-cache")
}

// --- Sonde -------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    title: String,
    source: String,
    uploader: Option<String>,
    duration_seconds: Option<f64>,
    max_height: Option<u64>,
    is_live: bool,
    /// "image" | "video" — une story-photo (Snap/Insta) n'a ni durée ni flux
    /// vidéo ; on la télécharge alors comme image, sans remux (sinon yt-dlp
    /// l'emballe dans un mp4 de 0 s).
    kind: &'static str,
    webpage_url: String,
    /// Rang `--playlist-items` quand ce « média isolé » est en fait l'unique
    /// snap d'un lien de partage revenu en playlist de représentations
    /// dupliquées (cf. `dedupe_representations`) : le téléchargement vise ce
    /// rang plutôt que de relancer `--no-playlist`, qui ramènerait aussi les
    /// leurres. `None` pour une vraie vidéo isolée (YouTube…).
    playlist_item: Option<u32>,
}

/// Une entrée d'un profil/liste (une story parmi d'autres). `index` est le
/// `playlist_index` yt-dlp : c'est lui qu'on redonnera au téléchargement
/// (`--playlist-items`) pour ré-extraire un lien frais, jamais le lien média
/// capté ici qui, lui, expire.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    index: u32,
    title: String,
    kind: &'static str,
    duration_seconds: Option<f64>,
    max_height: Option<u64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    title: Option<String>,
    source: String,
    entries: Vec<Entry>,
}

/// Ce que la sonde renvoie : un média isolé, ou une liste à choisir. Le champ
/// `mode` (`"single"` / `"list"`) discrimine côté frontend.
#[derive(Serialize)]
#[serde(tag = "mode", rename_all = "camelCase")]
pub enum ProbeResult {
    Single(Probe),
    List(Listing),
}

/// Plafond d'entrées extraites d'un profil : les stories tiennent largement
/// dessous, et ça borne le temps d'une URL de chaîne collée par mégarde (chaque
/// entrée est extraite en entier pour connaître son type photo/vidéo).
const LIST_LIMIT: &str = "100";

/// Suit le process de la sonde en cours pour pouvoir l'interrompre : extraire un
/// profil de 100 entrées peut prendre du temps, et l'utilisateur doit pouvoir
/// annuler (ou quitter la vue) sans laisser un yt-dlp tourner en fond.
#[derive(Default)]
pub struct ProbeManager {
    pid: AtomicU32,
}

#[tauri::command]
pub fn cancel_probe(probe: State<ProbeManager>) {
    kill_tree(probe.pid.swap(0, Ordering::Relaxed));
}

#[tauri::command]
pub fn probe_url(
    probe: State<ProbeManager>,
    url: String,
    cookies_from_browser: Option<String>,
) -> Result<ProbeResult, String> {
    let yt = doctor::tool_path("yt-dlp");
    let mut cmd = doctor::command(&yt);
    // `--no-playlist` ne réduit que les URLs « vidéo + playlist » (un lien
    // YouTube `watch?v=…&list=…` ramène bien la seule vidéo) ; un profil ou une
    // page de stories, qui n'a pas de vidéo isolée, reste une liste. On obtient
    // donc les deux comportements voulus avec un seul appel.
    cmd.args([
        "--dump-single-json",
        "--no-playlist",
        "--no-warnings",
        "--no-color",
        "--ignore-config",
        "--socket-timeout",
        "20",
        "--playlist-end",
        LIST_LIMIT,
    ]);
    cmd.arg("--cache-dir").arg(cache_dir());
    if let Some(browser) = cookies_from_browser.as_deref() {
        if !browser.is_empty() {
            cmd.arg("--cookies-from-browser").arg(browser);
        }
    }
    cmd.arg("--").arg(&url);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Spawn plutôt que `output()` : on garde le PID pour que `cancel_probe`
    // puisse tuer l'arbre. Le PID est remis à 0 dès la fin (évite qu'une
    // annulation tardive ne frappe un process sans rapport après réemploi du PID).
    let child = cmd
        .spawn()
        .map_err(|e| format!("yt-dlp ne démarre pas : {e}"))?;
    probe.pid.store(child.id(), Ordering::Relaxed);
    let waited = child.wait_with_output();
    probe.pid.store(0, Ordering::Relaxed);
    let out = waited.map_err(|e| format!("yt-dlp : {e}"))?;

    if !out.status.success() {
        return Err(humanize_error(&String::from_utf8_lossy(&out.stderr)));
    }

    let v: Value =
        serde_json::from_slice(&out.stdout).map_err(|_| "réponse illisible de yt-dlp".to_string())?;
    let obj = v.as_object().ok_or("réponse illisible de yt-dlp")?;

    if obj.get("_type").and_then(|t| t.as_str()) == Some("playlist") {
        let listing = parse_listing(obj);
        // Un lien de partage d'un snap isolé revient en « playlist » de
        // représentations dupliquées : une fois dédoublonné il n'en reste
        // qu'une, qu'on présente comme un média isolé plutôt qu'une galerie
        // d'une seule story (fini le « plusieurs options, seul le dernier marche »).
        if let [only] = listing.entries.as_slice() {
            return Ok(ProbeResult::Single(single_from_entry(only, &listing, &url)));
        }
        return Ok(ProbeResult::List(listing));
    }
    Ok(ProbeResult::Single(parse_media(obj, &url)))
}

/// Hauteur vidéo maximale, lue des formats ou du champ direct.
fn video_height(obj: &serde_json::Map<String, Value>) -> Option<u64> {
    obj.get("formats")
        .and_then(|f| f.as_array())
        .and_then(|arr| {
            arr.iter()
                .filter_map(|f| f.get("height").and_then(|h| h.as_u64()))
                .max()
        })
        .or_else(|| obj.get("height").and_then(|h| h.as_u64()))
}

fn parse_media(obj: &serde_json::Map<String, Value>, url: &str) -> Probe {
    let str_field = |key: &str| obj.get(key).and_then(|x| x.as_str());
    let height = video_height(obj);
    let duration = obj.get("duration").and_then(|x| x.as_f64());
    Probe {
        title: str_field("title").unwrap_or("Sans titre").to_string(),
        source: str_field("extractor_key")
            .or_else(|| str_field("extractor"))
            .unwrap_or("")
            .to_string(),
        uploader: str_field("uploader")
            .or_else(|| str_field("channel"))
            .or_else(|| str_field("uploader_id"))
            .map(str::to_string),
        duration_seconds: duration,
        max_height: height,
        is_live: obj.get("is_live").and_then(|x| x.as_bool()).unwrap_or(false)
            || str_field("live_status") == Some("is_live"),
        kind: media_kind(str_field("ext"), duration, height, str_field("vcodec")),
        webpage_url: str_field("webpage_url").unwrap_or(url).to_string(),
        playlist_item: None,
    }
}

fn parse_listing(obj: &serde_json::Map<String, Value>) -> Listing {
    let str_field = |key: &str| obj.get(key).and_then(|x| x.as_str());
    let entries = dedupe_representations(obj.get("entries").and_then(|e| e.as_array()));
    Listing {
        title: str_field("title")
            .or_else(|| str_field("uploader"))
            .map(str::to_string),
        source: str_field("extractor_key")
            .or_else(|| str_field("extractor"))
            .unwrap_or("")
            .to_string(),
        entries,
    }
}

/// Regroupe les entrées d'une playlist par snap et n'en garde qu'une par snap.
///
/// L'extracteur générique (Snapchat notamment) renvoie un lien de partage comme
/// une « playlist » de plusieurs représentations du MÊME snap : deux leurres
/// dont l'extension est le jeton Snap (« IRZXSOY », que yt-dlp refuse à
/// l'extraction) et un vrai `mp4`. Sans regroupement, l'app les affiche comme
/// autant de stories à cocher dont une seule se télécharge — le bug « plusieurs
/// options, seul le dernier marche ». La clé de snap est `webpage_url_basename`,
/// identique pour toutes les représentations ; l'`id`, lui, porte un suffixe
/// `-1/-2/-3` par embed et des paramètres d'URL volatils. On conserve, par snap,
/// la représentation la plus téléchargeable (extension média réelle plutôt que
/// jeton), avec son rang d'origine pour `--playlist-items`.
fn dedupe_representations(entries: Option<&Vec<Value>>) -> Vec<Entry> {
    let Some(arr) = entries else {
        return Vec::new();
    };
    let mut order: Vec<String> = Vec::new();
    let mut best: HashMap<String, (u8, Entry)> = HashMap::new();
    for (i, e) in arr.iter().enumerate() {
        let Some(eo) = e.as_object() else { continue };
        let Some(entry) = parse_entry(eo, i) else { continue };
        let key = snap_key(eo, i);
        let rank = ext_rank(eo.get("ext").and_then(Value::as_str));
        let replace = match best.get(&key) {
            Some((seen, _)) => rank > *seen,
            None => {
                order.push(key.clone());
                true
            }
        };
        if replace {
            best.insert(key, (rank, entry));
        }
    }
    order
        .into_iter()
        .filter_map(|k| best.remove(&k).map(|(_, entry)| entry))
        .collect()
}

/// Identifiant stable d'un snap, indépendant de la représentation. À défaut
/// (vrais extracteurs qui ne l'exposent pas), une clé unique par position — pour
/// ne jamais fusionner par erreur deux médias distincts.
fn snap_key(eo: &serde_json::Map<String, Value>, position: usize) -> String {
    match eo.get("webpage_url_basename").and_then(Value::as_str) {
        Some(b) if !b.is_empty() => b.to_string(),
        _ => format!("#{position}"),
    }
}

/// Classe une extension par « téléchargeabilité » : un conteneur vidéo réel
/// prime une image, qui prime une extension inconnue (le jeton Snap), qui prime
/// l'absence d'extension.
fn ext_rank(ext: Option<&str>) -> u8 {
    match ext.map(|e| e.to_ascii_lowercase()) {
        Some(e) if is_video_container(&e) => 3,
        Some(e) if IMAGE_EXTS.contains(&e.as_str()) => 2,
        Some(_) => 1,
        None => 0,
    }
}

/// Construit un média isolé à partir de l'unique snap restant après
/// dédoublonnage : le téléchargement visera `playlist_item` (le rang de la vraie
/// représentation) au lieu de relancer l'extraction complète.
fn single_from_entry(entry: &Entry, listing: &Listing, url: &str) -> Probe {
    Probe {
        title: entry.title.clone(),
        source: listing.source.clone(),
        uploader: listing.title.clone(),
        duration_seconds: entry.duration_seconds,
        max_height: entry.max_height,
        is_live: false,
        kind: entry.kind,
        webpage_url: url.to_string(),
        playlist_item: Some(entry.index),
    }
}

fn parse_entry(eo: &serde_json::Map<String, Value>, position: usize) -> Option<Entry> {
    let str_field = |key: &str| eo.get(key).and_then(|x| x.as_str());
    let index = eo
        .get("playlist_index")
        .and_then(|x| x.as_u64())
        .map(|n| n as u32)
        .unwrap_or(position as u32 + 1);
    let duration = eo.get("duration").and_then(|x| x.as_f64());
    let height = video_height(eo);
    // Une entrée seulement esquissée (extraction paresseuse : ni formats, ni
    // codec, ni durée) ne porte aucun signal fiable — on la suppose vidéo plutôt
    // que de risquer le faux positif « image » qui casserait le remux.
    let extracted = eo.contains_key("formats") || eo.contains_key("vcodec") || duration.is_some();
    let kind = if extracted {
        media_kind(str_field("ext"), duration, height, str_field("vcodec"))
    } else {
        "video"
    };
    let title = str_field("title")
        .filter(|s| !s.is_empty())
        .unwrap_or("Sans titre")
        .to_string();
    Some(Entry {
        index,
        title,
        kind,
        duration_seconds: duration,
        max_height: height,
    })
}

/// Extensions reconnues comme image, partagées par la détection de type et le
/// classement des représentations d'une playlist (`ext_rank`).
const IMAGE_EXTS: [&str; 7] = ["jpg", "jpeg", "png", "webp", "gif", "bmp", "heic"];

/// Distingue une image d'une vidéo. L'extension tranche en premier : une
/// extension image donne « image », un conteneur vidéo donne « vidéo ». Sans
/// extension exploitable (le jeton Snap, ou rien), on retombe sur les
/// métadonnées — mais « image » exige alors un signal *positif* d'absence de
/// flux vidéo (`vcodec == "none"`). L'absence pure de codec, elle, est la
/// signature de l'extracteur générique : une vraie vidéo Snap arrive sans durée,
/// sans hauteur, sans codec — la prendre pour une photo était le bug. Le doute
/// restant part en vidéo ; `recover_still_image` rattrape après téléchargement
/// une image qui aurait été mal rangée.
fn media_kind(
    ext: Option<&str>,
    duration: Option<f64>,
    height: Option<u64>,
    vcodec: Option<&str>,
) -> &'static str {
    if let Some(e) = ext.map(|e| e.to_ascii_lowercase()) {
        if IMAGE_EXTS.contains(&e.as_str()) {
            return "image";
        }
        if is_video_container(&e) {
            return "video";
        }
    }
    let has_duration = duration.map(|d| d > 0.0).unwrap_or(false);
    let no_video = vcodec == Some("none");
    if !has_duration && height.is_none() && no_video {
        "image"
    } else {
        "video"
    }
}

// --- File d'attente & état ---------------------------------------------------

/// Sémaphore comptant maison (std n'en fournit pas) : borne le nombre de
/// téléchargements simultanés.
struct Semaphore {
    count: Mutex<usize>,
    cv: Condvar,
}

impl Semaphore {
    fn new(n: usize) -> Self {
        Self {
            count: Mutex::new(n),
            cv: Condvar::new(),
        }
    }
    fn acquire(&self) {
        let mut c = self.count.lock().unwrap();
        while *c == 0 {
            c = self.cv.wait(c).unwrap();
        }
        *c -= 1;
    }
    fn release(&self) {
        *self.count.lock().unwrap() += 1;
        self.cv.notify_one();
    }
}

/// Rend une place au sémaphore quoi qu'il arrive (retour anticipé, panique).
struct SemGuard<'a>(&'a Semaphore);
impl Drop for SemGuard<'_> {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct JobHandle {
    cancelled: Arc<AtomicBool>,
    /// PID du process yt-dlp, 0 tant qu'il n'est pas lancé.
    pid: Arc<AtomicU32>,
}

struct Inner {
    sem: Semaphore,
    jobs: Mutex<HashMap<String, JobHandle>>,
    seq: AtomicU64,
}

pub struct DownloadManager {
    inner: Arc<Inner>,
}

impl Default for DownloadManager {
    fn default() -> Self {
        Self {
            inner: Arc::new(Inner {
                sem: Semaphore::new(2),
                jobs: Mutex::new(HashMap::new()),
                seq: AtomicU64::new(0),
            }),
        }
    }
}

impl DownloadManager {
    /// Identifiants des téléchargements vivants (en file ou en cours) — le
    /// janitor s'en sert pour épargner leurs dossiers temporaires.
    pub fn active_ids(&self) -> HashSet<String> {
        self.inner.jobs.lock().unwrap().keys().cloned().collect()
    }

    /// Coupe tous les téléchargements (à la fermeture) : lève le drapeau et tue
    /// chaque arbre de process pour qu'aucun yt-dlp/ffmpeg ne survive à l'app.
    pub fn cancel_all(&self) {
        let pids: Vec<u32> = {
            let jobs = self.inner.jobs.lock().unwrap();
            jobs.values()
                .map(|job| {
                    job.cancelled.store(true, Ordering::Relaxed);
                    job.pid.load(Ordering::Relaxed)
                })
                .collect()
        };
        for pid in pids {
            kill_tree(pid);
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRequest {
    url: String,
    /// "best" | "p1080" | "p720" | "audio"
    quality: String,
    wash: bool,
    destination: String,
    cookies_from_browser: Option<String>,
    /// Clé d'extracteur yt-dlp issue de la sonde (« Snapchat », « YouTube »…).
    /// Pilote le nom aléatoire des réseaux sociaux. Absente si l'utilisateur a
    /// lancé sans sonder.
    source: Option<String>,
    /// "image" | "video" issu de la sonde ; absent → traité en vidéo.
    kind: Option<String>,
    /// Numéro d'item à extraire du profil (`--playlist-items`). Présent quand on
    /// télécharge une story choisie dans une liste : `url` est alors l'URL du
    /// profil, et yt-dlp ré-extrait un lien frais — le lien média capté à la
    /// sonde aurait expiré.
    playlist_item: Option<u32>,
}

#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum DownloadEvent {
    Queued,
    Started,
    Progress {
        downloaded: Option<u64>,
        total: Option<u64>,
        speed: Option<f64>,
        eta: Option<f64>,
    },
    Postprocess {
        stage: String,
    },
    Washing,
    Completed {
        path: String,
        name: String,
    },
    Failed {
        message: String,
    },
    Cancelled,
}

#[tauri::command]
pub fn start_download(
    app: AppHandle,
    manager: State<DownloadManager>,
    req: DownloadRequest,
    on_event: Channel<DownloadEvent>,
) -> String {
    let inner = manager.inner.clone();
    let id = format!("dl-{}", inner.seq.fetch_add(1, Ordering::Relaxed));

    let handle = JobHandle {
        cancelled: Arc::new(AtomicBool::new(false)),
        pid: Arc::new(AtomicU32::new(0)),
    };
    let cancelled = handle.cancelled.clone();
    let pid = handle.pid.clone();
    inner.jobs.lock().unwrap().insert(id.clone(), handle);

    let inner_for_job = inner.clone();
    let id_for_job = id.clone();
    thread::spawn(move || {
        run_download(app, &inner_for_job, &id_for_job, req, &on_event, &cancelled, &pid);
        inner.jobs.lock().unwrap().remove(&id_for_job);
    });

    id
}

#[tauri::command]
pub fn cancel_download(manager: State<DownloadManager>, job_id: String) {
    let pid = {
        let jobs = manager.inner.jobs.lock().unwrap();
        match jobs.get(&job_id) {
            Some(job) => {
                job.cancelled.store(true, Ordering::Relaxed);
                job.pid.load(Ordering::Relaxed)
            }
            None => return,
        }
    };
    kill_tree(pid);
}

fn run_download(
    app: AppHandle,
    inner: &Inner,
    id: &str,
    req: DownloadRequest,
    ch: &Channel<DownloadEvent>,
    cancelled: &AtomicBool,
    pid: &AtomicU32,
) {
    let _ = ch.send(DownloadEvent::Queued);
    inner.sem.acquire();
    let _slot = SemGuard(&inner.sem);

    if cancelled.load(Ordering::Relaxed) {
        let _ = ch.send(DownloadEvent::Cancelled);
        return;
    }

    let job_dir = temp_root().join("dl").join(id);
    if let Err(e) = std::fs::create_dir_all(&job_dir) {
        let _ = ch.send(fail(format!("dossier temporaire : {e}")));
        return;
    }

    let raw = match download_raw(ch, cancelled, pid, &job_dir, &req) {
        Outcome::File(p) => p,
        Outcome::Cancelled => {
            let _ = std::fs::remove_dir_all(&job_dir);
            let _ = ch.send(DownloadEvent::Cancelled);
            return;
        }
        Outcome::Failed(msg) => {
            let _ = std::fs::remove_dir_all(&job_dir);
            let _ = ch.send(DownloadEvent::Failed { message: msg });
            return;
        }
    };

    // Une « vidéo » qui n'est en réalité qu'une image fixe (story-photo servie
    // avec une durée d'affichage, ou emballée en conteneur d'une frame par le
    // remux) est ramenée à sa vraie image avant lavage et déplacement.
    let raw = {
        let ffprobe = doctor::tool_path("ffprobe");
        let ffmpeg = doctor::tool_path("ffmpeg");
        recover_still_image(&ffprobe, &ffmpeg, &raw).unwrap_or(raw)
    };

    // Nom final = nom du fichier brut (sans le suffixe de lavage), lavé ou non.
    let final_name = raw
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "media".into());

    let to_move = if req.wash {
        let _ = ch.send(DownloadEvent::Washing);
        let ffmpeg = doctor::tool_path("ffmpeg");
        let exif = app.state::<laver::ExifState>();
        match laver::wash_download(&exif, &ffmpeg, &raw) {
            Ok(clean) => clean,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&job_dir);
                let _ = ch.send(fail(format!("lavage : {e}")));
                return;
            }
        }
    } else {
        raw.clone()
    };

    let dest_dir = PathBuf::from(&req.destination);
    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        let _ = std::fs::remove_dir_all(&job_dir);
        let _ = ch.send(fail(format!("dossier de destination : {e}")));
        return;
    }
    let dest = dedup(dest_dir.join(&final_name));
    if let Err(e) = move_file(&to_move, &dest) {
        let _ = std::fs::remove_dir_all(&job_dir);
        let _ = ch.send(fail(format!("déplacement : {e}")));
        return;
    }

    let _ = std::fs::remove_dir_all(&job_dir);
    let name = dest
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(final_name);
    let _ = ch.send(DownloadEvent::Completed {
        path: dest.to_string_lossy().into_owned(),
        name,
    });
}

enum Outcome {
    File(PathBuf),
    Cancelled,
    Failed(String),
}

/// Lance yt-dlp, streame la progression, et rend le fichier brut téléchargé.
fn download_raw(
    ch: &Channel<DownloadEvent>,
    cancelled: &AtomicBool,
    pid: &AtomicU32,
    job_dir: &Path,
    req: &DownloadRequest,
) -> Outcome {
    let _ = ch.send(DownloadEvent::Started);

    let yt = doctor::tool_path("yt-dlp");
    let ffmpeg_dir = doctor::tool_path("ffmpeg")
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    // Réseaux sociaux : le « titre » n'est qu'une légende (« View this Snap
    // from X… ») qui fuiterait dans le dossier de destination — no-trace. On lui
    // substitue un jeton aléatoire. Ailleurs (YouTube…), le vrai titre est utile.
    let out_template = if randomize_name(req.source.as_deref(), &req.url) {
        job_dir.join(format!("{}.%(ext)s", random_stem()))
    } else {
        job_dir.join("%(title).200B.%(ext)s")
    };
    let is_image = req.kind.as_deref() == Some("image");

    let mut cmd = doctor::command(&yt);
    cmd.args([
        "--no-color",
        "--newline",
        "--ignore-config",
        "--no-mtime",
        "--socket-timeout",
        "30",
        "--retries",
        "3",
        "--windows-filenames",
        "--trim-filenames",
        "200",
    ]);
    // Story choisie dans une liste : on cible l'item par son rang sur l'URL du
    // profil (lien frais). Sinon, une seule vidéo : on ignore tout contexte de
    // playlist que l'URL pourrait porter.
    match req.playlist_item {
        Some(n) => {
            cmd.arg("--playlist-items").arg(n.to_string());
        }
        None => {
            cmd.arg("--no-playlist");
        }
    }
    cmd.arg("--cache-dir").arg(cache_dir());
    cmd.arg("--ffmpeg-location").arg(&ffmpeg_dir);
    cmd.arg("-o").arg(&out_template);
    cmd.arg("--progress-template").arg(
        "download:LAVOIR\t%(progress.status)s\t%(progress.downloaded_bytes)s\t\
         %(progress.total_bytes)s\t%(progress.total_bytes_estimate)s\t\
         %(progress.speed)s\t%(progress.eta)s",
    );
    cmd.arg("--progress-template")
        .arg("postprocess:LAVOIR_PP\t%(progress.status)s\t%(progress.postprocessor)s");
    for a in download_args(&req.quality, is_image) {
        cmd.arg(a);
    }
    if let Some(browser) = req.cookies_from_browser.as_deref() {
        if !browser.is_empty() {
            cmd.arg("--cookies-from-browser").arg(browser);
        }
    }
    cmd.arg("--").arg(&req.url);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return Outcome::Failed(format!("yt-dlp ne démarre pas : {e}")),
    };
    pid.store(child.id(), Ordering::Relaxed);
    // Annulation arrivée entre le spawn et le store du PID.
    if cancelled.load(Ordering::Relaxed) {
        kill_tree(child.id());
    }

    // stdout et stderr fusionnés dans un seul flux de lignes : la progression
    // (préfixée) peut sortir sur l'un ou l'autre selon les versions de yt-dlp.
    let (tx, rx) = std::sync::mpsc::channel::<String>();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let tx2 = tx.clone();
    let h_out = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx2.send(line).is_err() {
                break;
            }
        }
    });
    let h_err = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });

    // Dernières lignes non-progression, pour humaniser une erreur éventuelle.
    let mut log: VecDeque<String> = VecDeque::with_capacity(80);
    // Extension source d'un éventuel remux : sert à effacer le jeton parasite
    // que yt-dlp laisse dans le nom quand l'URL n'a pas d'extension propre.
    let mut remux_src_ext: Option<String> = None;
    for line in rx {
        if let Some(rest) = line.strip_prefix("LAVOIR\t") {
            if let Some(ev) = parse_progress(rest) {
                let _ = ch.send(ev);
            }
        } else if let Some(rest) = line.strip_prefix("LAVOIR_PP\t") {
            let mut it = rest.split('\t');
            let status = it.next().unwrap_or("");
            let name = it.next().unwrap_or("");
            if status != "finished" {
                let _ = ch.send(DownloadEvent::Postprocess {
                    stage: pp_label(name),
                });
            }
        } else {
            if remux_src_ext.is_none() {
                remux_src_ext = parse_remux_source(&line);
            }
            if log.len() == 80 {
                log.pop_front();
            }
            log.push_back(line);
        }
    }
    let _ = h_out.join();
    let _ = h_err.join();
    let status = child.wait();

    if cancelled.load(Ordering::Relaxed) {
        return Outcome::Cancelled;
    }
    if !matches!(status, Ok(s) if s.success()) {
        let joined = log.iter().cloned().collect::<Vec<_>>().join("\n");
        return Outcome::Failed(humanize_error(&joined));
    }

    match find_media(job_dir) {
        Some(p) => {
            let p = remux_src_ext
                .as_deref()
                .and_then(|junk| strip_redundant_ext(&p, junk))
                .unwrap_or(p);
            Outcome::File(p)
        }
        None => Outcome::Failed("fichier téléchargé introuvable".into()),
    }
}

/// « [VideoRemuxer] Remuxing video from irzxsoy to mp4 » → « irzxsoy ».
fn parse_remux_source(line: &str) -> Option<String> {
    let src = line
        .split("Remuxing video from ")
        .nth(1)?
        .split(" to ")
        .next()?
        .trim();
    (!src.is_empty()).then(|| src.to_string())
}

/// Quand yt-dlp remuxe depuis une extension inhabituelle (URL Snapchat & co, qui
/// exposent un jeton aléatoire à la place d'une extension), il le conserve dans
/// le nom : « clip.IRZXSOY.mp4 ». On l'efface pour retomber sur « clip.mp4 ».
/// Ne se déclenche que si l'avant-dernier segment est bien ce jeton — un titre
/// contenant un point (« Godzilla vs. Kong.mp4 ») n'est jamais touché, faute de
/// remux. Renvoie le nouveau chemin après renommage.
fn strip_redundant_ext(path: &Path, junk: &str) -> Option<PathBuf> {
    let final_ext = path.extension()?.to_str()?;
    let stem = path.file_stem().and_then(|s| s.to_str())?;
    let inner_ext = Path::new(stem).extension()?.to_str()?;
    if !inner_ext.eq_ignore_ascii_case(junk) {
        return None;
    }
    let base = Path::new(stem).file_stem()?.to_str()?;
    let cleaned = dedup(path.with_file_name(format!("{base}.{final_ext}")));
    std::fs::rename(path, &cleaned).ok()?;
    Some(cleaned)
}

// --- Récupération d'une image servie en conteneur vidéo -----------------------

/// Conteneurs vidéo dont on sait ré-extraire une image fixe.
fn is_video_container(ext: &str) -> bool {
    matches!(ext, "mp4" | "mov" | "m4v" | "mkv" | "webm")
}

struct StillPlan {
    /// Extension de l'image récupérée.
    ext: &'static str,
    /// Le flux est déjà une image (JPEG/PNG) : on le copie sans réencoder.
    copy: bool,
}

/// Décide si un média téléchargé est en réalité une image fixe, à ne jamais
/// confondre avec une vraie vidéo : il faut d'abord un flux vidéo unique et
/// aucun audio.
///
/// Le signal décisif est le **codec**. Une plateforme qui sert une photo avec
/// une durée d'affichage (Snap, Insta) l'emballe en un flux Motion-JPEG/PNG :
/// le conteneur peut alors annoncer plusieurs secondes et un `nb_frames` absent
/// ou fantaisiste (flux fragmenté), mais le codec, lui, ne ment pas — c'est une
/// image, qu'on copie sans réencoder. Pour un vrai codec vidéo (h264…), on
/// n'accepte que la frame unique prouvée.
fn still_plan(
    video_streams: usize,
    audio_streams: usize,
    vcodec: &str,
    nb_frames: Option<u64>,
    duration: Option<f64>,
) -> Option<StillPlan> {
    if video_streams != 1 || audio_streams != 0 {
        return None;
    }
    match vcodec {
        "mjpeg" => return Some(StillPlan { ext: "jpg", copy: true }),
        "png" => return Some(StillPlan { ext: "png", copy: true }),
        _ => {}
    }
    // Le signal fiable est `nb_frames == 1`. Le repli sur la durée ne vaut que
    // quand `nb_frames` manque : on le borne serré (< 0,1 s ≈ une frame même à
    // très haut framerate) pour ne pas prendre un vrai clip muet court — un GIF
    // sans son de quelques dixièmes de seconde — pour une image fixe.
    let one_frame = nb_frames == Some(1)
        || (nb_frames.is_none() && duration.map(|d| d > 0.0 && d < 0.1).unwrap_or(false));
    one_frame.then_some(StillPlan { ext: "jpg", copy: false })
}

/// Ce que ffprobe apprend d'un fichier, pour trancher image fixe vs vidéo.
struct MediaProbe {
    video_streams: usize,
    audio_streams: usize,
    /// Codec du premier flux vidéo.
    vcodec: String,
    nb_frames: Option<u64>,
    duration: Option<f64>,
}

fn probe_streams(ffprobe: &Path, path: &Path) -> Option<MediaProbe> {
    let out = doctor::command(ffprobe)
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,codec_name,nb_frames",
            "-show_entries",
            "format=duration",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    let mut video = 0usize;
    let mut audio = 0usize;
    let mut vcodec = String::new();
    let mut nb_frames = None;
    for s in v.get("streams")?.as_array()? {
        match s.get("codec_type").and_then(Value::as_str) {
            Some("video") => {
                if video == 0 {
                    vcodec = s
                        .get("codec_name")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    nb_frames = s
                        .get("nb_frames")
                        .and_then(Value::as_str)
                        .and_then(|n| n.parse().ok());
                }
                video += 1;
            }
            Some("audio") => audio += 1,
            _ => {}
        }
    }
    let duration = v
        .get("format")
        .and_then(|f| f.get("duration"))
        .and_then(Value::as_str)
        .and_then(|d| d.parse().ok());
    Some(MediaProbe {
        video_streams: video,
        audio_streams: audio,
        vcodec,
        nb_frames,
        duration,
    })
}

/// Si le fichier téléchargé est en fait une image fixe, la ré-extrait en vraie
/// image — copie sans perte quand le flux est déjà du JPEG/PNG — et rend son
/// chemin ; le conteneur d'origine est supprimé. `None` sinon (vraie vidéo, ou
/// fichier déjà image).
fn recover_still_image(ffprobe: &Path, ffmpeg: &Path, path: &Path) -> Option<PathBuf> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !is_video_container(&ext) {
        return None;
    }
    let p = probe_streams(ffprobe, path)?;
    let plan = still_plan(
        p.video_streams,
        p.audio_streams,
        &p.vcodec,
        p.nb_frames,
        p.duration,
    )?;
    let out = dedup(path.with_extension(plan.ext));

    let mut cmd = doctor::command(ffmpeg);
    cmd.args(["-y", "-hide_banner", "-loglevel", "error"])
        .arg("-i")
        .arg(path)
        .args(["-map", "0:v:0", "-frames:v", "1"]);
    if plan.copy {
        cmd.args(["-c", "copy"]);
    }
    cmd.arg(&out);

    let ok = cmd.output().map(|o| o.status.success()).unwrap_or(false);
    if !ok || !out.exists() {
        let _ = std::fs::remove_file(&out);
        return None;
    }
    let _ = std::fs::remove_file(path);
    Some(out)
}

fn fail(message: String) -> DownloadEvent {
    DownloadEvent::Failed { message }
}

/// Une story-photo se télécharge en un seul flux, sans jamais de remux : c'est
/// le `--remux-video mp4` du chemin vidéo qui, appliqué à une image, la réencode
/// en mp4 muet de 0 s. On garde donc l'image dans son conteneur natif.
fn download_args(quality: &str, is_image: bool) -> Vec<&'static str> {
    if is_image {
        vec!["-f", "b"]
    } else {
        format_args(quality)
    }
}

/// Réseaux sociaux dont le « titre » est une légende sans valeur (et souvent
/// indiscrète) : on remplace le nom de fichier par un jeton aléatoire.
fn randomize_name(source: Option<&str>, url: &str) -> bool {
    const SOCIAL: [&str; 7] = [
        "snapchat",
        "instagram",
        "tiktok",
        "facebook",
        "twitter",
        "reddit",
        "threads",
    ];
    let has_kw = |s: &str| {
        let s = s.to_ascii_lowercase();
        SOCIAL.iter().any(|k| s.contains(k))
    };
    // La clé d'extracteur suffit d'ordinaire. Mais un lien de partage social
    // passe par l'extracteur générique (« Generic »/« HTML5MediaEmbed »), qui ne
    // trahit pas la plateforme — l'hôte de l'URL, si. Sans ce repli, la légende
    // du snap fuiterait dans le nom de fichier (entorse au no-trace).
    if source.map(&has_kw).unwrap_or(false) {
        return true;
    }
    match host_of(url) {
        Some(h) => has_kw(&h) || h == "x.com" || h.ends_with(".x.com"),
        None => false,
    }
}

/// Hôte (minuscule) d'une URL, sans dépendance — assez pour reconnaître une
/// plateforme sociale dans un lien collé.
fn host_of(url: &str) -> Option<String> {
    let after = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host = after.split(['/', '?', '#']).next()?;
    let host = host.rsplit('@').next()?; // userinfo éventuel
    let host = host.split(':').next()?; // port éventuel
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// Jeton aléatoire pour un nom de fichier, sans dépendance : `RandomState` est
/// réensemencé par l'OS à chaque appel, et l'horodatage en nanosecondes garantit
/// deux jetons distincts même en rafale.
fn random_stem() -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(nanos);
    let mut n = h.finish();
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut stem = String::with_capacity(13);
    while n > 0 {
        stem.push(ALPHABET[(n % 36) as usize] as char);
        n /= 36;
    }
    // n=0 (improbable) donnerait une chaîne vide : garde une longueur minimale.
    while stem.len() < 8 {
        stem.push('0');
    }
    stem
}

fn format_args(quality: &str) -> Vec<&'static str> {
    match quality {
        "audio" => vec!["-x", "--audio-format", "m4a", "--audio-quality", "0"],
        // `--remux-video mp4` garantit un conteneur mp4 même pour un flux unique
        // (sans fusion). Il débloque aussi les sources dont l'URL du média n'a
        // pas d'extension exploitable — Snapchat, notamment, expose un jeton
        // aléatoire que yt-dlp prend pour une extension « inhabituelle » et
        // refuse ; planifier un remux rend l'extension finale prévisible et lève
        // ce garde-fou. Sur un fichier déjà mp4, le remux est un no-op.
        // Le repli final `/b` sur les sélecteurs de hauteur évite « format non
        // disponible » quand la source est mono-format sans hauteur connue (un
        // média Snap générique) : on prend alors la seule qualité offerte.
        "p720" => vec![
            "-f",
            "bv*[height<=720]+ba/b[height<=720]/b",
            "--merge-output-format",
            "mp4",
            "--remux-video",
            "mp4",
        ],
        "p1080" => vec![
            "-f",
            "bv*[height<=1080]+ba/b[height<=1080]/b",
            "--merge-output-format",
            "mp4",
            "--remux-video",
            "mp4",
        ],
        _ => vec![
            "-f",
            "bv*+ba/b",
            "--merge-output-format",
            "mp4",
            "--remux-video",
            "mp4",
        ],
    }
}

fn parse_progress(rest: &str) -> Option<DownloadEvent> {
    let mut it = rest.split('\t');
    let status = it.next()?;
    let downloaded = num_u64(it.next().unwrap_or(""));
    let total = num_u64(it.next().unwrap_or(""));
    let total_est = num_u64(it.next().unwrap_or(""));
    let speed = num_f64(it.next().unwrap_or(""));
    let eta = num_f64(it.next().unwrap_or(""));

    match status {
        "downloading" | "finished" => Some(DownloadEvent::Progress {
            downloaded,
            total: total.or(total_est),
            speed,
            eta,
        }),
        _ => None,
    }
}

fn num_u64(s: &str) -> Option<u64> {
    if s == "NA" || s.is_empty() {
        None
    } else {
        s.parse::<f64>().ok().map(|f| f as u64)
    }
}

fn num_f64(s: &str) -> Option<f64> {
    if s == "NA" || s.is_empty() {
        None
    } else {
        s.parse().ok()
    }
}

fn pp_label(name: &str) -> String {
    match name {
        "Merger" => "Fusion",
        "ExtractAudio" => "Extraction audio",
        "VideoRemuxer" | "VideoConvertor" => "Conversion",
        "Metadata" | "FFmpegMetadata" => "Métadonnées",
        "" => "Finalisation",
        other => other,
    }
    .to_string()
}

/// Traduit le charabia stderr de yt-dlp en une phrase française actionnable.
/// Jamais d'échec silencieux : à défaut de motif connu, on renvoie la dernière
/// ligne d'erreur nettoyée.
fn humanize_error(text: &str) -> String {
    let low = text.to_lowercase();
    let has = |p: &str| low.contains(p);

    if has("private video") || has("this video is private") || has("is private") {
        return "Vidéo privée — accessible seulement à son propriétaire.".into();
    }
    if has("confirm your age") || has("age-restricted") || has("inappropriate for some users") {
        return "Contenu soumis à vérification d'âge — connexion requise.".into();
    }
    if has("sign in") || has("log in") || has("login required") || has("requires authentication")
        || has("this content isn't available") && has("account")
    {
        return "Connexion requise — active « utiliser ma session navigateur » dans les réglages."
            .into();
    }
    if has("not available in your country")
        || has("not available from your location")
        || (has("geo") && has("restrict"))
    {
        return "Contenu géobloqué dans ta région.".into();
    }
    if has("video unavailable")
        || has("has been removed")
        || has("no longer available")
        || has("been terminated")
        || has("does not exist")
    {
        return "Contenu supprimé ou indisponible.".into();
    }
    if has("unsupported url") || has("no suitable") || has("is not a valid url") {
        return "Lien non pris en charge.".into();
    }
    if has("http error 404") || has("404: not found") {
        return "Introuvable (404).".into();
    }
    if has("unable to download webpage")
        || has("failed to resolve")
        || has("timed out")
        || (has("connection") && (has("refused") || has("reset")))
    {
        return "Problème de connexion — réessaie.".into();
    }

    let last = text
        .lines()
        .rev()
        .find(|l| l.to_lowercase().contains("error"))
        .unwrap_or("")
        .trim();
    let cleaned = last
        .trim_start_matches("ERROR:")
        .split(';')
        .next()
        .unwrap_or(last)
        .trim();
    if cleaned.is_empty() {
        "Le téléchargement a échoué.".into()
    } else {
        format!("Échec : {cleaned}")
    }
}

fn find_media(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if matches!(ext.as_str(), "part" | "ytdl" | "temp" | "tmp") {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if best.as_ref().map(|(s, _)| size > *s).unwrap_or(true) {
            best = Some((size, path));
        }
    }
    best.map(|(_, p)| p)
}

/// `fs::rename` échoue entre volumes (temp sur C:, destination sur D:) : on
/// bascule alors sur copie puis suppression.
fn move_file(src: &Path, dst: &Path) -> Result<(), String> {
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dst).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(src);
    Ok(())
}

fn dedup(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned());
    let mut n = 2;
    loop {
        let name = match &ext {
            Some(e) => format!("{stem} ({n}).{e}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

fn kill_tree(pid: u32) {
    if pid == 0 {
        return;
    }
    #[cfg(windows)]
    {
        let _ = doctor::command(Path::new("taskkill"))
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
    }
}

// --- Destination -------------------------------------------------------------

#[tauri::command]
pub fn default_destination(app: AppHandle) -> String {
    let base = app
        .path()
        .download_dir()
        .unwrap_or_else(|_| PathBuf::from("."));
    base.join("lavoir").to_string_lossy().into_owned()
}

// --- Mise à jour de yt-dlp ----------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UpdateResult {
    updated: bool,
    version: Option<String>,
    message: String,
}

#[tauri::command]
pub fn update_ytdlp() -> UpdateResult {
    run_update()
}

pub fn run_update() -> UpdateResult {
    let yt = doctor::tool_path("yt-dlp");
    let out = match doctor::command(&yt).args(["-U", "--no-color"]).output() {
        Ok(o) => o,
        Err(e) => {
            return UpdateResult {
                updated: false,
                version: None,
                message: format!("yt-dlp ne démarre pas : {e}"),
            }
        }
    };

    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let low = text.to_lowercase();
    let updated = low.contains("updated yt-dlp") || low.contains("updated to");
    let up_to_date = low.contains("up to date") || low.contains("is up-to-date");

    let message = if updated {
        "yt-dlp mis à jour.".into()
    } else if up_to_date {
        "yt-dlp est déjà à jour.".into()
    } else if !out.status.success() {
        if low.contains("permission") || low.contains("access is denied") || low.contains("errno 13")
        {
            "Mise à jour impossible : droits insuffisants (installe l'app hors de Program Files, \
             ou relance-la en administrateur)."
                .into()
        } else {
            let l = text
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim();
            if l.is_empty() {
                "La mise à jour a échoué.".into()
            } else {
                l.to_string()
            }
        }
    } else {
        "Vérification terminée.".into()
    };

    UpdateResult {
        updated,
        version: current_version(&yt),
        message,
    }
}

fn current_version(yt: &Path) -> Option<String> {
    let out = doctor::command(yt).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string(),
    )
}

/// Résultat de la vérification faite au démarrage (si l'utilisateur l'a activée),
/// que la page Réglages affiche à son ouverture.
#[derive(Default)]
pub struct LaunchUpdate(pub Mutex<Option<UpdateResult>>);

#[tauri::command]
pub fn launch_update_status(state: State<LaunchUpdate>) -> Option<UpdateResult> {
    state.0.lock().unwrap().clone()
}

/// Appelé au `setup()` : si l'option est active, vérifie/met à jour yt-dlp en
/// fond et range le résultat pour la page Réglages.
pub fn spawn_launch_check(app: &AppHandle) {
    if !settings::load(app).check_ytdlp_on_launch {
        return;
    }
    let app = app.clone();
    thread::spawn(move || {
        let result = run_update();
        *app.state::<LaunchUpdate>().0.lock().unwrap() = Some(result);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_parsing_handles_missing_fields() {
        // status, downloaded, total(NA), total_estimate, speed, eta
        let ev = parse_progress("downloading\t1048576\tNA\t5242880\t524288.0\t8").unwrap();
        match ev {
            DownloadEvent::Progress {
                downloaded,
                total,
                speed,
                eta,
            } => {
                assert_eq!(downloaded, Some(1_048_576));
                assert_eq!(total, Some(5_242_880)); // repli sur l'estimation
                assert_eq!(speed, Some(524_288.0));
                assert_eq!(eta, Some(8.0));
            }
            _ => panic!("attendu Progress"),
        }
        assert!(parse_progress("").is_none());
        // Une ligne « finished » sans octets connus reste un Progress valide.
        assert!(matches!(
            parse_progress("finished\tNA\tNA\tNA\tNA\tNA"),
            Some(DownloadEvent::Progress { .. })
        ));
    }

    #[test]
    fn humanize_maps_known_failures() {
        assert!(humanize_error("ERROR: Private video. Sign in if you've been granted access")
            .contains("privée"));
        assert!(humanize_error("ERROR: Unsupported URL: https://example.com").contains("pris en charge"));
        assert!(humanize_error("ERROR: Video unavailable. This video has been removed")
            .contains("supprimé"));
        assert!(
            humanize_error("ERROR: [geo] This video is not available in your country")
                .contains("géobloqué")
        );
        // Motif inconnu : on garde la substance sans planter.
        let fallback = humanize_error("ERROR: something very specific broke");
        assert!(fallback.starts_with("Échec"));
        assert!(fallback.contains("something very specific broke"));
    }

    #[test]
    fn parse_remux_source_reads_the_original_ext() {
        assert_eq!(
            parse_remux_source(
                "[VideoRemuxer] Remuxing video from irzxsoy to mp4; Destination: clip.IRZXSOY.mp4"
            )
            .as_deref(),
            Some("irzxsoy")
        );
        assert!(parse_remux_source("[download] 100% of 2MiB").is_none());
        // « Not remuxing … » (déjà au bon format) ne doit rien capturer.
        assert!(parse_remux_source("[VideoRemuxer] Not remuxing media file clip.mp4").is_none());
    }

    #[test]
    fn strip_redundant_ext_drops_only_the_junk_token() {
        let dir = std::env::temp_dir().join("lavoir-strip-test");
        std::fs::create_dir_all(&dir).unwrap();

        // Cas Snapchat : le jeton parasite disparaît.
        let junk = dir.join("Ma story.IRZXSOY.mp4");
        std::fs::write(&junk, b"x").unwrap();
        let cleaned = strip_redundant_ext(&junk, "irzxsoy").unwrap();
        assert_eq!(cleaned.file_name().unwrap(), "Ma story.mp4");
        assert!(cleaned.exists() && !junk.exists());

        // Titre à point légitime : intouché (l'avant-dernier segment ≠ jeton).
        let titled = dir.join("Godzilla vs. Kong.mp4");
        std::fs::write(&titled, b"x").unwrap();
        assert!(strip_redundant_ext(&titled, "irzxsoy").is_none());
        assert!(titled.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn obj(v: Value) -> serde_json::Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn probe_result_carries_mode_tag() {
        // Le frontend discrimine sur `mode` : il doit accompagner les champs.
        let single = serde_json::to_value(ProbeResult::Single(parse_media(
            &obj(serde_json::json!({"title": "X", "duration": 3.0, "vcodec": "h264"})),
            "u",
        )))
        .unwrap();
        assert_eq!(single["mode"], "single");
        assert_eq!(single["title"], "X");
        assert_eq!(single["kind"], "video");

        let list = serde_json::to_value(ProbeResult::List(parse_listing(&obj(
            serde_json::json!({"_type": "playlist", "entries": []}),
        ))))
        .unwrap();
        assert_eq!(list["mode"], "list");
        assert!(list["entries"].is_array());
    }

    #[test]
    fn parse_media_reads_a_single_video() {
        let m = parse_media(
            &obj(serde_json::json!({
                "title": "Une vidéo", "extractor_key": "YouTube",
                "duration": 12.0, "vcodec": "vp9",
                "formats": [{"height": 720}, {"height": 1080}],
                "webpage_url": "https://y/x"
            })),
            "https://y/x",
        );
        assert_eq!(m.title, "Une vidéo");
        assert_eq!(m.kind, "video");
        assert_eq!(m.max_height, Some(1080));
    }

    #[test]
    fn parse_listing_splits_photos_and_videos() {
        let listing = parse_listing(&obj(serde_json::json!({
            "_type": "playlist",
            "title": "Stories de X",
            "extractor_key": "InstagramStory",
            "entries": [
                {"playlist_index": 1, "title": "s1", "vcodec": "none", "ext": "jpg"},
                {"playlist_index": 2, "title": "s2", "duration": 8.0, "vcodec": "h264",
                 "formats": [{"height": 1080}]}
            ]
        })));
        assert_eq!(listing.source, "InstagramStory");
        assert_eq!(listing.entries.len(), 2);
        assert_eq!(listing.entries[0].index, 1);
        assert_eq!(listing.entries[0].kind, "image");
        assert_eq!(listing.entries[1].kind, "video");
        assert_eq!(listing.entries[1].duration_seconds, Some(8.0));
    }

    #[test]
    fn parse_listing_collapses_snap_share_representations() {
        // Cas RÉEL d'un lien de partage Snapchat : le générique renvoie trois
        // représentations du même snap (même `webpage_url_basename`) — deux
        // leurres à extension jeton « IRZXSOY » et le vrai mp4. On ne garde que
        // le mp4, avec son rang d'origine (3) pour `--playlist-items`.
        let listing = parse_listing(&obj(serde_json::json!({
            "_type": "playlist",
            "title": "ZK",
            "extractor_key": "Generic",
            "entries": [
                {"playlist_index": 1, "title": "View this Snap", "ext": "IRZXSOY",
                 "vcodec": "none", "webpage_url_basename": "SNAPID", "formats": [{"ext": "IRZXSOY"}]},
                {"playlist_index": 2, "title": "View this Snap", "ext": "IRZXSOY",
                 "vcodec": "none", "webpage_url_basename": "SNAPID", "formats": [{"ext": "IRZXSOY"}]},
                {"playlist_index": 3, "title": "View this Snap", "ext": "mp4",
                 "vcodec": "none", "webpage_url_basename": "SNAPID", "formats": [{"ext": "mp4"}]}
            ]
        })));
        assert_eq!(listing.entries.len(), 1);
        assert_eq!(listing.entries[0].index, 3);

        // Et présenté comme un média isolé qui vise ce rang au téléchargement.
        let single = single_from_entry(&listing.entries[0], &listing, "https://snapchat.com/t/xyz");
        assert_eq!(single.playlist_item, Some(3));
        assert_eq!(single.source, "Generic");
    }

    #[test]
    fn dedupe_keeps_distinct_snaps_apart() {
        // Deux snaps distincts (basenames différents) ne fusionnent jamais.
        let listing = parse_listing(&obj(serde_json::json!({
            "_type": "playlist",
            "entries": [
                {"playlist_index": 1, "ext": "mp4", "webpage_url_basename": "A", "formats": [{}]},
                {"playlist_index": 2, "ext": "mp4", "webpage_url_basename": "B", "formats": [{}]}
            ]
        })));
        assert_eq!(listing.entries.len(), 2);
    }

    #[test]
    fn parse_entry_defaults_thin_entries_to_video() {
        // Entrée à peine esquissée (extraction paresseuse) : pas de faux « image ».
        let e = parse_entry(&obj(serde_json::json!({"title": "brouillon"})), 4).unwrap();
        assert_eq!(e.kind, "video");
        assert_eq!(e.index, 5); // position 4 → rang 5 quand playlist_index manque
    }

    #[test]
    fn media_kind_spots_images() {
        // Extension image explicite → image, quelles que soient les métadonnées.
        assert_eq!(media_kind(Some("jpg"), None, None, Some("none")), "image");
        assert_eq!(media_kind(Some("webp"), Some(0.0), None, None), "image");
        // Sans extension mais flux vidéo explicitement absent → image (story-photo).
        assert_eq!(media_kind(None, None, None, Some("none")), "image");
        // RÉGRESSION Snapchat : une vidéo servie par le générique arrive en mp4
        // sans durée, sans hauteur, sans codec — c'est une VIDÉO, pas une photo.
        assert_eq!(media_kind(Some("mp4"), None, None, None), "video");
        // Aucun signal du tout (ni extension, ni métadonnées) → défaut prudent : vidéo.
        assert_eq!(media_kind(None, None, None, None), "video");
        // Vraie vidéo : durée présente.
        assert_eq!(media_kind(Some("mp4"), Some(7.0), Some(1080), Some("h264")), "video");
        // Vidéo sans durée mais avec hauteur → reste vidéo.
        assert_eq!(media_kind(Some("mp4"), None, Some(720), Some("vp9")), "video");
    }

    #[test]
    fn quality_selectors_fall_back_to_best_available() {
        // Sur un média mono-format sans hauteur (Snap générique), le filtre de
        // hauteur ne matche rien ; le repli `/b` évite « format non disponible ».
        for q in ["p720", "p1080"] {
            let fmt = format_args(q);
            let sel = fmt[fmt.iter().position(|a| *a == "-f").unwrap() + 1];
            assert!(sel.ends_with("/b"), "{q} devrait retomber sur /b : {sel}");
        }
    }

    #[test]
    fn download_args_skips_remux_for_images() {
        let img = download_args("best", true);
        assert!(!img.iter().any(|a| a.contains("remux")));
        assert!(!img.iter().any(|a| a.contains("merge")));
        // Le chemin vidéo garde bien son remux.
        assert!(download_args("best", false).contains(&"--remux-video"));
    }

    #[test]
    fn randomize_name_targets_socials_only() {
        let neutral = "https://example.com/x";
        assert!(randomize_name(Some("Snapchat"), neutral));
        assert!(randomize_name(Some("InstagramStory"), neutral));
        assert!(randomize_name(Some("TikTok"), neutral));
        assert!(!randomize_name(Some("YouTube"), "https://youtube.com/watch?v=x"));
        assert!(!randomize_name(Some("Vimeo"), neutral));
        assert!(!randomize_name(None, neutral));
        // Repli sur l'URL : un partage Snap passe par l'extracteur générique,
        // dont la clé (« Generic »/« HTML5MediaEmbed ») ne dit rien de Snapchat.
        assert!(randomize_name(Some("Generic"), "https://snapchat.com/t/T8LP5mek"));
        assert!(randomize_name(
            Some("HTML5MediaEmbed"),
            "https://www.snapchat.com/@u/spotlight/xyz"
        ));
        assert!(randomize_name(None, "https://x.com/user/status/1"));
        // Pas de faux positif : « x.com » ne doit pas capturer « netflix.com ».
        assert!(!randomize_name(Some("Generic"), "https://www.netflix.com/watch/123"));
    }

    #[test]
    fn random_stem_is_unique_and_clean() {
        let a = random_stem();
        let b = random_stem();
        assert_ne!(a, b);
        assert!(a.len() >= 8);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn dedup_avoids_clobbering() {
        let dir = std::env::temp_dir().join("lavoir-dedup-test");
        std::fs::create_dir_all(&dir).unwrap();
        let base = dir.join("clip.mp4");
        // Chemin libre → inchangé.
        assert_eq!(dedup(base.clone()), base);
        std::fs::write(&base, b"x").unwrap();
        assert_eq!(dedup(base.clone()), dir.join("clip (2).mp4"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn still_plan_spots_wrapped_photos() {
        let plan = |v, a, c, nf, d| still_plan(v, a, c, nf, d).map(|p| (p.ext, p.copy));
        // Story-photo Snap : flux mjpeg sans audio → JPEG copié sans perte.
        assert_eq!(plan(1, 0, "mjpeg", Some(1), Some(0.04)), Some(("jpg", true)));
        // Cas Snap RÉEL : flux mjpeg fragmenté, nb_frames absent, durée
        // d'affichage de plusieurs secondes → reconnu image par le codec.
        assert_eq!(plan(1, 0, "mjpeg", None, Some(5.0)), Some(("jpg", true)));
        // PNG fixe servi avec une durée → copié en .png.
        assert_eq!(plan(1, 0, "png", None, Some(3.0)), Some(("png", true)));
        // Frame unique dans un vrai codec vidéo → JPEG réencodé (pas de copie).
        assert_eq!(plan(1, 0, "h264", Some(1), Some(0.04)), Some(("jpg", false)));
        // Repli sur une durée d'une frame quand nb_frames manque (codec vidéo).
        assert!(still_plan(1, 0, "h264", None, Some(0.03)).is_some());
    }

    #[test]
    fn still_plan_leaves_real_videos_alone() {
        // Vraie vidéo : plusieurs frames + audio.
        assert!(still_plan(1, 1, "h264", Some(69), Some(2.3)).is_none());
        // Codec vidéo muet mais long (nb_frames > 1) → pas une image.
        assert!(still_plan(1, 0, "h264", Some(300), Some(12.0)).is_none());
        // Codec vidéo, durée longue sans nb_frames → pas une image.
        assert!(still_plan(1, 0, "h264", None, Some(12.0)).is_none());
        // Clip muet court (GIF sans son, ~0,4 s) sans nb_frames → vraie vidéo,
        // pas une image fixe : on ne le convertit pas en JPEG.
        assert!(still_plan(1, 0, "h264", None, Some(0.4)).is_none());
        // Deux flux vidéo → on ne touche pas.
        assert!(still_plan(2, 0, "mjpeg", None, Some(5.0)).is_none());
        // mjpeg mais AVEC une piste audio → c'est une vidéo : on ne touche pas.
        assert!(still_plan(1, 1, "mjpeg", None, Some(5.0)).is_none());
    }
}
