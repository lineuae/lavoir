//! Inspection et lavage des métadonnées d'images, via exiftool.
//!
//! exiftool démarre lentement (déballage Perl, ~1 s) : on garde un process
//! persistant en mode `-stay_open`, sérialisé derrière un Mutex. Chaque
//! commande est encadrée par un marqueur `{ready<seq>}` sur stdout (via
//! `-execute<seq>`) et sur stderr (via `-echo4`), ce qui permet de lire la
//! sortie de bout en bout sans ambiguïté.
//!
//! Le lavage ne touche jamais l'original : on copie vers un fichier de sortie
//! puis on nettoie la copie, et on la re-scanne pour prouver le résultat.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

use tauri::ipc::Channel;

use crate::doctor;

#[derive(Serialize, Clone)]
pub struct Tag {
    pub group: String,
    pub name: String,
    pub value: String,
}

#[derive(Serialize, Clone)]
pub struct GpsFix {
    pub lat: f64,
    pub lon: f64,
}

#[derive(Serialize, Clone)]
pub struct ThumbInfo {
    pub bytes: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FileReport {
    pub path: String,
    pub file_name: String,
    pub file_type: String,
    pub mime_type: String,
    pub is_video: bool,
    pub can_clean: bool,
    pub tags: Vec<Tag>,
    pub gps: Option<GpsFix>,
    pub thumbnail: Option<ThumbInfo>,
    pub error: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanOptions {
    /// "all" | "gps" | "keepDate"
    pub mode: String,
    pub rename_neutral: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CleanResult {
    pub src: String,
    pub dst: Option<String>,
    pub dst_name: Option<String>,
    pub after_tags: Vec<Tag>,
    pub ok: bool,
    pub error: Option<String>,
}

/// Progression du lavage, diffusée par Channel pendant `clean_files`. Seul le
/// remux vidéo en émet — l'exiftool des images est instantané ; `percent` suit
/// les octets écrits par ffmpeg rapportés à la taille de l'entrée, indexé par
/// le chemin source pour que le frontend sache quelle carte animer.
#[derive(Serialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum CleanEvent {
    Progress { src: String, percent: f64 },
}

// --- Session exiftool persistante -------------------------------------------

pub struct ExifSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr_rx: Receiver<String>,
    seq: u64,
}

fn new_command(path: &Path) -> Command {
    let mut cmd = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

impl ExifSession {
    fn spawn() -> Result<Self, String> {
        Self::spawn_at(&doctor::tool_path("exiftool"))
    }

    fn spawn_at(path: &Path) -> Result<Self, String> {
        let mut child = new_command(path)
            .args(["-stay_open", "True", "-@", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("exiftool ne démarre pas : {e}"))?;

        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let stderr = child.stderr.take().unwrap();

        // exiftool ne produit que quelques lignes de stderr par commande ; un
        // thread dédié les draine pour ne jamais bloquer sur un tube plein.
        let (tx, stderr_rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                match line {
                    Ok(l) => {
                        if tx.send(l).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self {
            child,
            stdin,
            stdout,
            stderr_rx,
            seq: 0,
        })
    }

    /// Envoie une commande (chaque argument sur sa ligne) et renvoie
    /// (stdout, stderr) une fois le marqueur de fin reçu.
    fn execute(&mut self, args: &[&str]) -> Result<(String, String), String> {
        // Purge des lignes stderr résiduelles d'une commande précédente.
        while self.stderr_rx.try_recv().is_ok() {}

        self.seq += 1;
        let marker = format!("{{ready{}}}", self.seq);

        for arg in args {
            writeln!(self.stdin, "{arg}").map_err(|e| format!("écriture exiftool : {e}"))?;
        }
        // -echo4 imprime le marqueur sur stderr après traitement ;
        // -execute<seq> l'imprime sur stdout.
        writeln!(self.stdin, "-echo4\n{marker}\n-execute{}", self.seq)
            .map_err(|e| format!("écriture exiftool : {e}"))?;
        self.stdin
            .flush()
            .map_err(|e| format!("flush exiftool : {e}"))?;

        let mut out = String::new();
        loop {
            let mut line = String::new();
            let n = self
                .stdout
                .read_line(&mut line)
                .map_err(|e| format!("lecture exiftool : {e}"))?;
            if n == 0 {
                return Err("exiftool a fermé sa sortie".into());
            }
            if line.trim_end() == marker {
                break;
            }
            out.push_str(&line);
        }

        // stderr est secondaire (détection du succès d'écriture) : on le draine
        // jusqu'au marqueur mais sans bloquer indéfiniment si exiftool est muet.
        let mut err = String::new();
        loop {
            match self.stderr_rx.recv_timeout(Duration::from_secs(5)) {
                Ok(line) if line.trim_end() == marker => break,
                Ok(line) => {
                    err.push_str(&line);
                    err.push('\n');
                }
                Err(_) => break,
            }
        }

        Ok((out, err))
    }
}

impl Drop for ExifSession {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "-stay_open\nFalse");
        let _ = self.stdin.flush();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
pub struct ExifState(Mutex<Option<ExifSession>>);

impl ExifState {
    /// Exécute `f` avec la session vivante ; une erreur de tube détruit la
    /// session pour qu'elle soit reconstruite à l'appel suivant.
    fn with<T>(&self, f: impl FnOnce(&mut ExifSession) -> Result<T, String>) -> Result<T, String> {
        let mut guard = self.0.lock().unwrap();
        if guard.is_none() {
            *guard = Some(ExifSession::spawn()?);
        }
        let result = f(guard.as_mut().unwrap());
        if result.is_err() {
            *guard = None;
        }
        result
    }
}

// --- Inspection --------------------------------------------------------------

fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Array(a) => a.iter().map(value_to_string).collect::<Vec<_>>().join(", "),
        Value::Object(_) => v.to_string(),
        Value::Null => String::new(),
    }
}

/// Signe la coordonnée selon son hémisphère (« South »/« West » → négatif).
fn signed(value: f64, reference: Option<&str>) -> f64 {
    match reference.map(|r| r.chars().next().unwrap_or(' ').to_ascii_uppercase()) {
        Some('S') | Some('W') => -value.abs(),
        _ => value.abs(),
    }
}

/// « 48.858400 N » / « 2.294500 W » → coordonnée décimale signée. exiftool
/// restitue ainsi la position vidéo (atome QuickTime) dans le groupe Composite.
fn parse_signed_coord(v: &str) -> Option<f64> {
    let mut parts = v.split_whitespace();
    let value: f64 = parts.next()?.parse().ok()?;
    Some(signed(value, parts.next()))
}

fn inspect_one(session: &mut ExifSession, path: &str) -> FileReport {
    let file_name = Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());

    let mut report = FileReport {
        path: path.to_string(),
        file_name,
        file_type: String::new(),
        mime_type: String::new(),
        is_video: false,
        can_clean: false,
        tags: Vec::new(),
        gps: None,
        thumbnail: None,
        error: None,
    };

    // -a doublons, -u tags inconnus (compte honnête), -G1 groupe précis,
    // -c coordonnées GPS en degrés décimaux (sans toucher les autres tags).
    let out = match session.execute(&[
        "-charset",
        "filename=UTF8",
        "-json",
        "-a",
        "-u",
        "-G1",
        "-c",
        "%.6f",
        path,
    ]) {
        Ok((out, _)) => out,
        Err(e) => {
            report.error = Some(e);
            return report;
        }
    };

    let parsed: Vec<Value> = match serde_json::from_str(&out) {
        Ok(v) => v,
        Err(_) => {
            report.error = Some("exiftool n'a pas pu lire ce fichier".into());
            return report;
        }
    };
    let Some(obj) = parsed.into_iter().next().and_then(|v| match v {
        Value::Object(m) => Some(m),
        _ => None,
    }) else {
        report.error = Some("aucune métadonnée lisible".into());
        return report;
    };

    let mut gps_lat: Option<f64> = None;
    let mut gps_lat_ref: Option<String> = None;
    let mut gps_lon: Option<f64> = None;
    let mut gps_lon_ref: Option<String> = None;
    let mut comp_lat: Option<f64> = None;
    let mut comp_lon: Option<f64> = None;
    let mut thumb_bytes: Option<u64> = None;

    for (key, value) in &obj {
        let Some((group, name)) = key.split_once(':') else {
            continue; // SourceFile
        };

        match group {
            "File" => {
                if name == "FileType" {
                    report.file_type = value_to_string(value);
                } else if name == "MIMEType" {
                    report.mime_type = value_to_string(value);
                }
            }
            "GPS" => match name {
                "GPSLatitude" => gps_lat = value_to_string(value).parse().ok(),
                "GPSLatitudeRef" => gps_lat_ref = Some(value_to_string(value)),
                "GPSLongitude" => gps_lon = value_to_string(value).parse().ok(),
                "GPSLongitudeRef" => gps_lon_ref = Some(value_to_string(value)),
                _ => {}
            },
            // Une vidéo n'a pas de groupe GPS : sa position vient d'un atome
            // QuickTime, qu'exiftool restitue déjà signée en Composite.
            "Composite" => match name {
                "GPSLatitude" => comp_lat = parse_signed_coord(&value_to_string(value)),
                "GPSLongitude" => comp_lon = parse_signed_coord(&value_to_string(value)),
                _ => {}
            },
            _ => {}
        }

        if name == "ThumbnailLength" || name == "PreviewImageLength" {
            if let Ok(n) = value_to_string(value).parse::<u64>() {
                if n > 0 {
                    thumb_bytes = Some(n);
                }
            }
        }

        report.tags.push(Tag {
            group: group.to_string(),
            name: name.to_string(),
            value: value_to_string(value),
        });
    }

    report.gps = match (gps_lat, gps_lon) {
        (Some(lat), Some(lon)) => Some(GpsFix {
            lat: signed(lat, gps_lat_ref.as_deref()),
            lon: signed(lon, gps_lon_ref.as_deref()),
        }),
        _ => match (comp_lat, comp_lon) {
            (Some(lat), Some(lon)) => Some(GpsFix { lat, lon }),
            _ => None,
        },
    };
    report.thumbnail = thumb_bytes.map(|bytes| ThumbInfo { bytes });
    report.is_video = report.mime_type.starts_with("video/");
    report.can_clean = if report.is_video {
        is_remuxable_video(&report.file_type)
    } else {
        is_writable_image(&report.file_type)
    };

    report
}

/// Formats image qu'exiftool sait réécrire. « Extended WEBP » est le type que
/// renvoie exiftool dès qu'un WebP porte des métadonnées (conteneur VP8X) —
/// c'est-à-dire tout WebP qui vaut la peine d'être lavé ; sans lui, un WebP
/// géotagué serait affiché comme non lavable.
fn is_writable_image(file_type: &str) -> bool {
    matches!(
        file_type,
        "JPEG" | "PNG" | "WEBP" | "Extended WEBP" | "TIFF" | "GIF" | "HEIC" | "HEIF" | "AVIF"
    )
}

/// Conteneurs qu'on sait laver par remux ffmpeg sans réencodage.
fn is_remuxable_video(file_type: &str) -> bool {
    matches!(file_type, "MOV" | "MP4" | "M4V" | "MKV" | "WEBM")
}

// --- Lavage ------------------------------------------------------------------

fn output_path(src: &Path, rename_neutral: bool, index: usize) -> PathBuf {
    let parent = src.parent().unwrap_or_else(|| Path::new("."));
    let ext = src.extension().map(|e| e.to_string_lossy().into_owned());
    let base = if rename_neutral {
        format!("media-{:04}", index + 1)
    } else {
        let stem = src.file_stem().unwrap_or_default().to_string_lossy();
        format!("{stem}-propre")
    };

    let with_ext = |name: &str| -> PathBuf {
        match &ext {
            Some(e) => parent.join(format!("{name}.{e}")),
            None => parent.join(name),
        }
    };

    let mut candidate = with_ext(&base);
    let mut n = 2;
    while candidate.exists() {
        candidate = with_ext(&format!("{base}-{n}"));
        n += 1;
    }
    candidate
}

/// Identité qui survit à `-all=` dans un TIFF : ces tags vivent dans l'IFD0,
/// qu'exiftool refuse de vider en bloc (c'est la structure même de l'image), si
/// bien qu'un `-all=` laissait passer Make/Model/Software/Artist… On les efface
/// donc nommément. C'est un no-op inoffensif sur JPEG/PNG/WebP, où `-all=` les a
/// déjà retirés (découvert au corpus de la phase 7 sur un vrai TIFF).
const IDENTITY_STRIP: [&str; 14] = [
    "-Make=",
    "-Model=",
    "-Software=",
    "-Artist=",
    "-HostComputer=",
    "-Copyright=",
    "-ImageDescription=",
    "-CameraSerialNumber=",
    "-SerialNumber=",
    "-XPTitle=",
    "-XPComment=",
    "-XPAuthor=",
    "-XPKeywords=",
    "-XPSubject=",
];

/// Dates qui, pour la même raison que l'identité ci-dessus, subsistent dans
/// l'IFD0 d'un TIFF après `-all=`. Effacées en mode « tout », recopiées en
/// mode « garder la date ».
const DATE_STRIP: [&str; 4] = [
    "-ModifyDate=",
    "-DateTimeOriginal=",
    "-CreateDate=",
    "-OffsetTimeOriginal=",
];

fn clean_args<'a>(mode: &str, dst: &'a str) -> Vec<&'a str> {
    let mut args = vec!["-charset", "filename=UTF8"];
    match mode {
        "gps" => {
            args.extend([
                "-gps:all=",
                "-xmp:gpslatitude=",
                "-xmp:gpslongitude=",
                "-xmp:gpsaltitude=",
            ]);
        }
        "keepDate" => {
            // Tout retirer (identité IFD0 comprise), puis recopier orientation,
            // profil couleur et dates depuis l'original.
            args.push("-all=");
            args.extend(IDENTITY_STRIP);
            args.extend([
                "-tagsFromFile",
                "@",
                "-Orientation",
                "-ICC_Profile",
                "-DateTimeOriginal",
                "-CreateDate",
                "-ModifyDate",
                "-OffsetTimeOriginal",
            ]);
        }
        _ => {
            // "all" — tout retirer (identité et dates résiduelles des TIFF
            // comprises) sauf orientation (sinon photos couchées) et profil ICC
            // (sinon couleurs délavées).
            args.push("-all=");
            args.extend(IDENTITY_STRIP);
            args.extend(DATE_STRIP);
            args.extend(["-tagsFromFile", "@", "-Orientation", "-ICC_Profile"]);
        }
    }
    args.push("-overwrite_original");
    args.push(dst);
    args
}

fn clean_one(session: &mut ExifSession, src: &str, mode: &str, dst: PathBuf) -> CleanResult {
    let dst_name = dst
        .file_name()
        .map(|n| n.to_string_lossy().into_owned());
    let dst_str = dst.to_string_lossy().into_owned();

    let mut result = CleanResult {
        src: src.to_string(),
        dst: Some(dst_str.clone()),
        dst_name,
        after_tags: Vec::new(),
        ok: false,
        error: None,
    };

    if let Err(e) = std::fs::copy(src, &dst) {
        result.dst = None;
        result.error = Some(format!("copie impossible : {e}"));
        return result;
    }

    let (_out, err) = match session.execute(&clean_args(mode, &dst_str)) {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_file(&dst);
            result.dst = None;
            result.error = Some(e);
            return result;
        }
    };

    // exiftool signale l'échec d'écriture en clair sur stderr (messages anglais).
    if err.contains("weren't updated") || err.contains("0 image files updated") {
        let _ = std::fs::remove_file(&dst);
        result.dst = None;
        result.error = Some(
            err.lines()
                .find(|l| l.contains("rror") || l.contains("arning"))
                .unwrap_or("exiftool n'a pas pu laver ce format")
                .trim()
                .to_string(),
        );
        return result;
    }

    // Preuve : on re-scanne la copie lavée. Le frontend compare avant/après.
    result.after_tags = inspect_one(session, &dst_str).tags;
    result.ok = true;
    result
}

fn is_video_ext(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| {
            matches!(
                e.to_ascii_lowercase().as_str(),
                "mov" | "mp4" | "m4v" | "mkv" | "webm"
            )
        })
        .unwrap_or(false)
}

/// Lavage vidéo : remux ffmpeg sans réencodage. On retire toutes les
/// métadonnées de conteneur et les flux de données Apple (atomes `mebx` :
/// position, timed metadata) en gardant les pistes A/V bit à bit. Le remux ne
/// pouvant pas supprimer les dates obligatoires (mvhd/tkhd), ffmpeg les remet à
/// zéro ; `+bitexact` l'empêche de resigner la sortie avec son tag encoder.
///
/// Il n'y a pas de lavage partiel sans réencodage sur une vidéo : les trois
/// modes image (tout / GPS / date) se ramènent ici au même nettoyage complet.
///
/// `on_progress` reçoit l'avancement (0-100) pendant le remux ; sans intérêt
/// pour un petit fichier (quasi instantané), il évite la barre indéterminée sur
/// les gros conteneurs.
fn clean_video(
    session: &mut ExifSession,
    ffmpeg: &Path,
    src: &str,
    dst: PathBuf,
    on_progress: impl FnMut(f64),
) -> CleanResult {
    let dst_name = dst.file_name().map(|n| n.to_string_lossy().into_owned());
    let dst_str = dst.to_string_lossy().into_owned();

    let mut result = CleanResult {
        src: src.to_string(),
        dst: Some(dst_str.clone()),
        dst_name,
        after_tags: Vec::new(),
        ok: false,
        error: None,
    };

    if let Err(msg) = run_ffmpeg_remux(ffmpeg, src, &dst_str, on_progress) {
        let _ = std::fs::remove_file(&dst);
        result.dst = None;
        result.error = Some(msg);
        return result;
    }

    result.after_tags = inspect_one(session, &dst_str).tags;
    result.ok = true;
    result
}

/// Le remux proprement dit. Les flags retirent les métadonnées de conteneur et
/// les flux de données Apple (`mebx`) en copiant l'A/V bit à bit ; `+bitexact`
/// empêche ffmpeg de resigner la sortie avec son tag encodeur. On lit la
/// progression sur stdout (`-progress pipe:1`) : pour une copie de flux,
/// `total_size` (octets écrits) croît vers la taille de l'entrée et tient donc
/// lieu de pourcentage sans qu'on ait à sonder la durée. stderr (les erreurs
/// réelles) est drainé dans un thread pour ne jamais bloquer sur un tube plein.
/// Octets écrits → pourcentage borné à 99 (la sortie diffère légèrement de
/// l'entrée ; le 100 est laissé au résultat final). `None` si la taille d'entrée
/// est inconnue → le frontend garde alors sa barre indéterminée.
fn remux_percent(written: u64, input_size: u64) -> Option<f64> {
    (input_size > 0).then(|| (written as f64 / input_size as f64 * 100.0).min(99.0))
}

fn run_ffmpeg_remux(
    ffmpeg: &Path,
    src: &str,
    dst: &str,
    mut on_progress: impl FnMut(f64),
) -> Result<(), String> {
    let input_size = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);

    let mut child = new_command(ffmpeg)
        .args([
            "-y", "-nostdin", "-hide_banner",
            "-loglevel", "error",
            "-progress", "pipe:1", "-nostats",
            "-i", src,
            "-map", "0", "-map", "-0:d",
            "-map_metadata", "-1",
            "-c", "copy",
            "-fflags", "+bitexact",
            dst,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg ne démarre pas : {e}"))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let err_handle = thread::spawn(move || {
        let mut buf = String::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            buf.push_str(&line);
            buf.push('\n');
        }
        buf
    });

    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if let Some(v) = line.strip_prefix("total_size=") {
            if let Some(pct) = v.trim().parse::<u64>().ok().and_then(|d| remux_percent(d, input_size)) {
                on_progress(pct);
            }
        }
    }

    let status = child.wait().map_err(|e| format!("ffmpeg : {e}"))?;
    let stderr_text = err_handle.join().unwrap_or_default();

    if status.success() {
        Ok(())
    } else {
        Err(stderr_text
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("ffmpeg n'a pas pu laver cette vidéo")
            .trim()
            .to_string())
    }
}

// --- Lavage d'un téléchargement (réutilisé par recuperer.rs) -----------------

fn ext_lower(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Conteneurs A/V qu'on lave par remux ffmpeg (vidéo *et* audio : mêmes flux à
/// copier bit à bit, mêmes métadonnées de conteneur à retirer).
fn is_remux_ext(ext: &str) -> bool {
    matches!(
        ext,
        "mov" | "mp4" | "m4v" | "mkv" | "webm" | "m4a" | "mp3" | "aac" | "opus" | "ogg" | "oga"
            | "flac" | "wav"
    )
}

fn is_image_ext(ext: &str) -> bool {
    matches!(
        ext,
        "jpg" | "jpeg" | "png" | "webp" | "tiff" | "tif" | "gif" | "heic" | "heif" | "avif"
    )
}

/// Lave un fichier fraîchement téléchargé et renvoie le chemin de la copie
/// propre. yt-dlp/ffmpeg laissent des traces dans le conteneur (URL source dans
/// un commentaire, tags d'encodeur…) : on repasse par le même nettoyage complet
/// que les phases 2-3. Format inconnu → rien à laver, on garde le fichier tel
/// quel (on ne fait jamais échouer un téléchargement sur un type non reconnu).
pub(crate) fn wash_download(
    state: &ExifState,
    ffmpeg: &Path,
    src: &Path,
) -> Result<PathBuf, String> {
    let src_str = src.to_string_lossy().into_owned();
    let dst = output_path(src, false, 0);
    let ext = ext_lower(src);

    let result = if is_remux_ext(&ext) {
        // Le lavage post-téléchargement affiche déjà un état « Lavage… » côté
        // Récupérer : pas de progression par fichier à diffuser ici.
        state.with(|s| Ok(clean_video(s, ffmpeg, &src_str, dst.clone(), |_| {})))?
    } else if is_image_ext(&ext) {
        state.with(|s| Ok(clean_one(s, &src_str, "all", dst.clone())))?
    } else {
        return Ok(src.to_path_buf());
    };

    if result.ok {
        result
            .dst
            .map(PathBuf::from)
            .ok_or_else(|| "lavage sans fichier de sortie".to_string())
    } else {
        Err(result.error.unwrap_or_else(|| "lavage échoué".to_string()))
    }
}

// --- Commandes Tauri ---------------------------------------------------------

#[tauri::command]
pub fn inspect_files(
    state: tauri::State<ExifState>,
    paths: Vec<String>,
) -> Result<Vec<FileReport>, String> {
    state.with(|session| Ok(paths.iter().map(|p| inspect_one(session, p)).collect()))
}

// `async` force l'exécution hors du thread principal (Tauri l'ordonnance sur un
// worker) : sans cela, ce lavage synchrone bloquerait la boucle d'événements et
// les `CleanEvent` de progression ne seraient remis au webview qu'à la fin du
// lot — la barre resterait figée. Même raison que le thread de `start_download`.
#[tauri::command(async)]
pub fn clean_files(
    state: tauri::State<ExifState>,
    paths: Vec<String>,
    options: CleanOptions,
    on_event: Channel<CleanEvent>,
) -> Result<Vec<CleanResult>, String> {
    let ffmpeg = doctor::tool_path("ffmpeg");
    state.with(|session| {
        Ok(paths
            .iter()
            .enumerate()
            .map(|(i, src)| {
                let path = Path::new(src);
                let dst = output_path(path, options.rename_neutral, i);
                if is_video_ext(path) {
                    clean_video(session, &ffmpeg, src, dst, |percent| {
                        let _ = on_event.send(CleanEvent::Progress {
                            src: src.clone(),
                            percent,
                        });
                    })
                } else {
                    clean_one(session, src, &options.mode, dst)
                }
            })
            .collect())
    })
}

/// Vignette EXIF intégrée en data-URL, extraite à la demande (appel one-shot :
/// la sortie binaire ne passe pas par le protocole texte de stay_open).
#[tauri::command]
pub fn extract_thumbnail(path: String) -> Option<String> {
    let exiftool = doctor::tool_path("exiftool");
    let output = new_command(&exiftool)
        .args(["-b", "-ThumbnailImage", &path])
        .output()
        .ok()?;
    if output.stdout.is_empty() {
        return None;
    }
    Some(format!(
        "data:image/jpeg;base64,{}",
        base64_encode(&output.stdout)
    ))
}

fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(ALPHABET[(n >> 18 & 63) as usize] as char);
        out.push(ALPHABET[(n >> 12 & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6 & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Graines sans métadonnée : les tests y injectent GPS/appareil/date, puis
    // vérifient le cycle inspection → lavage → re-scan de bout en bout.
    const SEED: &[u8] = include_bytes!("../tests/fixtures/seed.jpg");
    const SEED_MOV: &[u8] = include_bytes!("../tests/fixtures/seed.mov");
    const SEED_PNG: &[u8] = include_bytes!("../tests/fixtures/seed.png");
    const SEED_WEBP: &[u8] = include_bytes!("../tests/fixtures/seed.webp");
    const SEED_TIFF: &[u8] = include_bytes!("../tests/fixtures/seed.tif");

    fn session() -> ExifSession {
        let exe = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries/exiftool-x86_64-pc-windows-msvc.exe");
        ExifSession::spawn_at(&exe).expect("exiftool doit être installé (fetch-sidecars.ps1)")
    }

    fn ffmpeg() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries/ffmpeg-x86_64-pc-windows-msvc.exe")
    }

    fn temp_file(name: &str, bytes: &[u8]) -> PathBuf {
        let dir = std::env::temp_dir().join("lavoir-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn temp_seed(name: &str) -> PathBuf {
        temp_file(name, SEED)
    }

    /// Denylist volontairement indépendante de `classify.ts` : un vrai
    /// contre-contrôle du lavage, pas une tautologie. Un tag est « sensible »
    /// s'il révèle la position, l'identité de l'appareil ou de l'auteur,
    /// l'horodatage de capture, ou une vignette d'avant recadrage. Les dates
    /// remises à zéro par le remux vidéo (`0000…`) ne comptent pas.
    fn is_sensitive(tag: &Tag) -> bool {
        let (g, n, v) = (tag.group.as_str(), tag.name.as_str(), tag.value.trim());
        if v.is_empty() {
            return false;
        }
        if g == "GPS" || (g == "Composite" && n.starts_with("GPS")) {
            return true;
        }
        if matches!(
            n,
            "GPSCoordinates" | "GPSPosition" | "GPSLatitude" | "GPSLongitude"
                | "GPSAltitude" | "LocationInformation"
        ) {
            return true;
        }
        if matches!(
            n,
            "Make" | "Model" | "Software" | "HostComputer" | "Artist" | "OwnerName"
                | "CameraID" | "LensModel" | "LensMake" | "XPAuthor" | "XPComment"
                | "XPKeywords" | "XPSubject" | "XPTitle"
        ) {
            return true;
        }
        if n.contains("SerialNumber") {
            return true;
        }
        // Atomes de conteneur Apple/Android (position, timed metadata).
        if n.starts_with("com.apple") || n.starts_with("com.android") {
            return true;
        }
        if matches!(
            n,
            "DateTimeOriginal" | "CreateDate" | "ModifyDate" | "OffsetTimeOriginal"
                | "SubSecDateTimeOriginal"
        ) {
            return !v.starts_with("0000");
        }
        if matches!(
            n,
            "ThumbnailImage" | "ThumbnailLength" | "PreviewImage" | "PreviewImageLength"
        ) {
            return true;
        }
        false
    }

    fn residue(tags: &[Tag]) -> Vec<String> {
        tags.iter()
            .filter(|t| is_sensitive(t))
            .map(|t| format!("{}:{} = {}", t.group, t.name, t.value))
            .collect()
    }

    #[test]
    fn inspects_and_strips_a_photo() {
        let mut s = session();
        let src = temp_seed("inspect.jpg");
        let src_str = src.to_string_lossy().into_owned();

        // Injection d'un profil « iPhone à Paris ».
        s.execute(&[
            "-Make=Apple",
            "-Model=iPhone 14 Pro",
            "-DateTimeOriginal=2024:07:05 14:32:10",
            "-GPSLatitude=48.858370",
            "-GPSLatitudeRef=N",
            "-GPSLongitude=2.294481",
            "-GPSLongitudeRef=E",
            "-overwrite_original",
            &src_str,
        ])
        .expect("injection exiftool");

        let report = inspect_one(&mut s, &src_str);
        assert!(report.error.is_none(), "inspection : {:?}", report.error);
        assert_eq!(report.file_type, "JPEG");
        assert!(report.can_clean);
        let gps = report.gps.expect("le GPS doit être détecté");
        assert!((gps.lat - 48.858370).abs() < 1e-4, "lat = {}", gps.lat);
        assert!((gps.lon - 2.294481).abs() < 1e-4, "lon = {}", gps.lon);
        assert!(report.tags.iter().any(|t| t.name == "Make" && t.value == "Apple"));

        // Lavage complet → copie propre, original intact.
        let dst = output_path(&src, false, 0);
        let result = clean_one(&mut s, &src_str, "all", dst);
        assert!(result.ok, "lavage : {:?}", result.error);
        assert!(src.exists(), "l'original ne doit pas être supprimé");

        let residual_gps = result.after_tags.iter().filter(|t| t.group == "GPS").count();
        assert_eq!(residual_gps, 0, "il reste du GPS après lavage");
        assert!(
            !result.after_tags.iter().any(|t| t.name == "Make"),
            "la marque de l'appareil survit au lavage",
        );

        let _ = std::fs::remove_file(&src);
        if let Some(d) = result.dst {
            let _ = std::fs::remove_file(d);
        }
    }

    #[test]
    fn strips_common_image_formats() {
        let mut s = session();
        // TIFF est le cas piège : Make/Model/Software vivent dans l'IFD0 qu'un
        // `-all=` ne peut pas retirer en bloc — sans l'effacement nommé du
        // lavage, l'identité de l'appareil survivrait ici.
        for (bytes, name, want_type) in [
            (SEED_PNG, "fmt.png", "PNG"),
            // Un WebP porteur de métadonnées bascule en conteneur étendu (VP8X).
            (SEED_WEBP, "fmt.webp", "Extended WEBP"),
            (SEED_TIFF, "fmt.tif", "TIFF"),
        ] {
            let src = temp_file(name, bytes);
            let src_str = src.to_string_lossy().into_owned();

            s.execute(&[
                "-Make=Apple",
                "-Model=iPhone 14 Pro",
                "-Software=iOS 17",
                "-DateTimeOriginal=2024:07:05 14:32:10",
                "-GPSLatitude=48.858370",
                "-GPSLatitudeRef=N",
                "-GPSLongitude=2.294481",
                "-GPSLongitudeRef=E",
                "-overwrite_original",
                &src_str,
            ])
            .unwrap_or_else(|e| panic!("injection {name} : {e}"));

            let report = inspect_one(&mut s, &src_str);
            assert!(report.error.is_none(), "{name} inspection : {:?}", report.error);
            assert_eq!(report.file_type, want_type, "{name} : type inattendu");
            assert!(report.can_clean, "{name} devrait être lavable");
            assert!(report.gps.is_some(), "{name} : GPS injecté non détecté");

            let dst = output_path(&src, false, 0);
            let result = clean_one(&mut s, &src_str, "all", dst);
            assert!(result.ok, "{name} lavage : {:?}", result.error);
            assert!(src.exists(), "{name} : l'original ne doit pas être supprimé");

            let left = residue(&result.after_tags);
            assert!(left.is_empty(), "{name} : résidu sensible après lavage : {left:?}");

            let _ = std::fs::remove_file(&src);
            if let Some(d) = result.dst {
                let _ = std::fs::remove_file(d);
            }
        }
    }

    #[test]
    fn gps_only_keeps_the_camera() {
        let mut s = session();
        let src = temp_seed("gpsonly.jpg");
        let src_str = src.to_string_lossy().into_owned();

        s.execute(&[
            "-Make=Canon",
            "-GPSLatitude=40.7",
            "-GPSLatitudeRef=N",
            "-GPSLongitude=-74.0",
            "-GPSLongitudeRef=W",
            "-overwrite_original",
            &src_str,
        ])
        .expect("injection exiftool");

        let dst = output_path(&src, false, 0);
        let result = clean_one(&mut s, &src_str, "gps", dst);
        assert!(result.ok, "lavage gps : {:?}", result.error);
        assert_eq!(result.after_tags.iter().filter(|t| t.group == "GPS").count(), 0);
        assert!(
            result.after_tags.iter().any(|t| t.name == "Make"),
            "le mode GPS seul doit conserver l'appareil",
        );

        let _ = std::fs::remove_file(&src);
        if let Some(d) = result.dst {
            let _ = std::fs::remove_file(d);
        }
    }

    #[test]
    fn strips_a_video() {
        let mut s = session();
        let ffmpeg = ffmpeg();
        let src = temp_file("clip.mov", SEED_MOV);
        let src_str = src.to_string_lossy().into_owned();

        // Position + appareil injectés en atomes QuickTime, comme un iPhone.
        s.execute(&[
            "-api",
            "QuickTimeUTC=1",
            "-Keys:GPSCoordinates=48.8584 2.2945",
            "-Keys:Make=Apple",
            "-Keys:Model=iPhone 14",
            "-overwrite_original",
            &src_str,
        ])
        .expect("injection exiftool");

        let report = inspect_one(&mut s, &src_str);
        assert!(report.error.is_none(), "inspection : {:?}", report.error);
        assert!(report.is_video, "le MOV doit être vu comme une vidéo");
        assert!(report.can_clean, "un MOV doit être lavable");
        // La position, absente du groupe GPS, doit venir du Composite QuickTime.
        let gps = report.gps.expect("la position vidéo doit être détectée");
        assert!((gps.lat - 48.8584).abs() < 1e-3, "lat = {}", gps.lat);
        assert!((gps.lon - 2.2945).abs() < 1e-3, "lon = {}", gps.lon);

        let dst = output_path(&src, false, 0);
        let result = clean_video(&mut s, &ffmpeg, &src_str, dst, |_| {});
        assert!(result.ok, "lavage vidéo : {:?}", result.error);
        assert!(src.exists(), "l'original ne doit pas être supprimé");

        // Le remux ne laisse ni position, ni identité d'appareil.
        for name in ["GPSCoordinates", "Make", "Model"] {
            assert!(
                !result.after_tags.iter().any(|t| t.name == name),
                "« {name} » survit au lavage vidéo",
            );
        }
        let rescan = inspect_one(&mut s, result.dst.as_ref().unwrap());
        assert!(rescan.gps.is_none(), "position résiduelle après remux");

        let _ = std::fs::remove_file(&src);
        if let Some(d) = result.dst {
            let _ = std::fs::remove_file(d);
        }
    }

    #[test]
    fn corpus_folder_when_provided() {
        // Acceptation opt-in : pointe LAVOIR_CORPUS vers un dossier de vrais
        // médias (dump DCIM, HEIC géotagués…). Chaque fichier est lavé dans un
        // dossier temporaire — l'original n'est jamais touché — puis on asserte
        // zéro résidu sensible. Vert et sauté tant que la variable n'est pas là.
        let Ok(dir) = std::env::var("LAVOIR_CORPUS") else {
            eprintln!("LAVOIR_CORPUS non défini — test de corpus réel sauté (cf. README).");
            return;
        };
        let work = std::env::temp_dir().join("lavoir-corpus");
        let _ = std::fs::remove_dir_all(&work);
        std::fs::create_dir_all(&work).unwrap();

        let mut s = session();
        let ffmpeg = ffmpeg();
        let mut washed = 0usize;
        let mut failures: Vec<String> = Vec::new();

        let entries = std::fs::read_dir(&dir).expect("dossier LAVOIR_CORPUS illisible");
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext = ext_lower(&path);
            let (img, vid) = (is_image_ext(&ext), is_remux_ext(&ext));
            if !img && !vid {
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            let orig = path.to_string_lossy().into_owned();
            let before = residue(&inspect_one(&mut s, &orig).tags).len();

            // On lave une copie, jamais l'original.
            let copy = work.join(&name);
            std::fs::copy(&path, &copy).unwrap();
            let copy_str = copy.to_string_lossy().into_owned();
            let dst = output_path(&copy, false, washed);
            let result = if vid {
                clean_video(&mut s, &ffmpeg, &copy_str, dst, |_| {})
            } else {
                clean_one(&mut s, &copy_str, "all", dst)
            };

            if !result.ok {
                failures.push(format!("{name} : lavage échoué — {:?}", result.error));
                continue;
            }
            let left = residue(&result.after_tags);
            eprintln!("{name:<44} {before:>3} → {}", left.len());
            if !left.is_empty() {
                failures.push(format!("{name} : résidu {left:?}"));
            }
            washed += 1;
        }
        let _ = std::fs::remove_dir_all(&work);

        eprintln!("corpus : {washed} fichier(s) lavé(s)");
        assert!(failures.is_empty(), "corpus — échec(s) :\n{}", failures.join("\n"));
    }

    #[test]
    fn remux_percent_maps_bytes_and_clamps() {
        // Taille inconnue → pas de pourcentage (barre indéterminée).
        assert_eq!(remux_percent(1000, 0), None);
        // Progression normale.
        assert_eq!(remux_percent(500, 1000), Some(50.0));
        // La sortie peut dépasser l'entrée d'un cheveu : borné à 99, jamais 100+.
        assert_eq!(remux_percent(1010, 1000), Some(99.0));
    }

    #[test]
    fn output_path_names_and_avoids_collisions() {
        let dir = std::env::temp_dir().join("lavoir-outpath");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("photo.jpg");
        let named = |p: PathBuf| p.file_name().unwrap().to_str().unwrap().to_owned();

        // Suffixe -propre par défaut ; renommage neutre en media-NNNN (1-indexé).
        assert_eq!(named(output_path(&src, false, 0)), "photo-propre.jpg");
        assert_eq!(named(output_path(&src, true, 4)), "media-0005.jpg");

        // Collision : on suffixe -2, -3… sans écraser une sortie déjà là.
        std::fs::write(dir.join("photo-propre.jpg"), b"x").unwrap();
        assert_eq!(named(output_path(&src, false, 0)), "photo-propre-2.jpg");
        std::fs::write(dir.join("photo-propre-2.jpg"), b"x").unwrap();
        assert_eq!(named(output_path(&src, false, 0)), "photo-propre-3.jpg");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
