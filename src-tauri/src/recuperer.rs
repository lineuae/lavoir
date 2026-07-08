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
    webpage_url: String,
}

#[tauri::command]
pub fn probe_url(url: String) -> Result<Probe, String> {
    let yt = doctor::tool_path("yt-dlp");
    let out = doctor::command(&yt)
        .args([
            "--dump-single-json",
            "--no-playlist",
            "--no-warnings",
            "--no-color",
            "--ignore-config",
            "--socket-timeout",
            "20",
        ])
        .arg("--cache-dir")
        .arg(cache_dir())
        .arg("--")
        .arg(&url)
        .output()
        .map_err(|e| format!("yt-dlp ne démarre pas : {e}"))?;

    if !out.status.success() {
        return Err(humanize_error(&String::from_utf8_lossy(&out.stderr)));
    }

    let v: Value =
        serde_json::from_slice(&out.stdout).map_err(|_| "réponse illisible de yt-dlp".to_string())?;
    let obj = v.as_object().ok_or("réponse illisible de yt-dlp")?;

    if obj.get("_type").and_then(|t| t.as_str()) == Some("playlist") {
        return Err("Les playlists ne sont pas gérées — colle le lien d'une seule vidéo.".into());
    }

    let str_field = |key: &str| obj.get(key).and_then(|x| x.as_str());
    let max_height = obj
        .get("formats")
        .and_then(|f| f.as_array())
        .and_then(|arr| {
            arr.iter()
                .filter_map(|f| f.get("height").and_then(|h| h.as_u64()))
                .max()
        })
        .or_else(|| obj.get("height").and_then(|h| h.as_u64()));

    Ok(Probe {
        title: str_field("title").unwrap_or("Sans titre").to_string(),
        source: str_field("extractor_key")
            .or_else(|| str_field("extractor"))
            .unwrap_or("")
            .to_string(),
        uploader: str_field("uploader")
            .or_else(|| str_field("channel"))
            .or_else(|| str_field("uploader_id"))
            .map(str::to_string),
        duration_seconds: obj.get("duration").and_then(|x| x.as_f64()),
        max_height,
        is_live: obj.get("is_live").and_then(|x| x.as_bool()).unwrap_or(false)
            || str_field("live_status") == Some("is_live"),
        webpage_url: str_field("webpage_url").unwrap_or(&url).to_string(),
    })
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
    let out_template = job_dir.join("%(title).200B.%(ext)s");

    let mut cmd = doctor::command(&yt);
    cmd.args([
        "--no-playlist",
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
    for a in format_args(&req.quality) {
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
        Some(p) => Outcome::File(p),
        None => Outcome::Failed("fichier téléchargé introuvable".into()),
    }
}

fn fail(message: String) -> DownloadEvent {
    DownloadEvent::Failed { message }
}

fn format_args(quality: &str) -> Vec<&'static str> {
    match quality {
        "audio" => vec!["-x", "--audio-format", "m4a", "--audio-quality", "0"],
        "p720" => vec![
            "-f",
            "bv*[height<=720]+ba/b[height<=720]",
            "--merge-output-format",
            "mp4",
        ],
        "p1080" => vec![
            "-f",
            "bv*[height<=1080]+ba/b[height<=1080]",
            "--merge-output-format",
            "mp4",
        ],
        _ => vec!["-f", "bv*+ba/b", "--merge-output-format", "mp4"],
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
}
