//! Résolution et sonde des moteurs embarqués (yt-dlp, ffmpeg, ffprobe, exiftool).
//!
//! Les sidecars Tauri sont copiés à côté de l'exécutable (target/debug en dev,
//! dossier d'installation en prod) : la résolution est donc toujours
//! « répertoire de l'exe + nom.exe », dans les deux mondes.

use serde::Serialize;
use std::path::PathBuf;
use std::process::Command;

#[derive(Serialize, Clone)]
pub struct ToolStatus {
    pub name: String,
    pub version: Option<String>,
    pub error: Option<String>,
}

const TOOLS: [(&str, &str); 4] = [
    ("yt-dlp", "--version"),
    ("ffmpeg", "-version"),
    ("ffprobe", "-version"),
    ("exiftool", "-ver"),
];

pub fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn tool_path(tool: &str) -> PathBuf {
    let beside_exe = exe_dir().join(format!("{tool}.exe"));

    // En dev l'exe tourne depuis target/{debug,release} ; si le CLI Tauri n'a
    // pas (encore) copié les sidecars à côté, on retombe sur src-tauri/binaries.
    // v1 = Windows x64 uniquement, le triple est assumé en dur.
    #[cfg(debug_assertions)]
    if !beside_exe.exists() {
        let in_sources = exe_dir()
            .join("../../binaries")
            .join(format!("{tool}-x86_64-pc-windows-msvc.exe"));
        if in_sources.exists() {
            return in_sources;
        }
    }

    beside_exe
}

/// Construit une commande vers un moteur sans jamais faire clignoter de console
/// Windows (les sidecars sont des exes console). Point d'entrée unique pour
/// toute invocation d'un binaire embarqué.
pub fn command(path: &std::path::Path) -> Command {
    let mut cmd = Command::new(path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// « ffmpeg version 7.1.1-essentials_build-www.gyan.dev … » → « 7.1.1 » ;
/// yt-dlp et exiftool impriment déjà le numéro nu.
fn short_version(tool: &str, stdout: &str) -> String {
    let first = stdout.lines().next().unwrap_or("").trim();
    match tool {
        "ffmpeg" | "ffprobe" => first
            .split_whitespace()
            .nth(2)
            .unwrap_or(first)
            .split('-')
            .next()
            .unwrap_or(first)
            .to_string(),
        _ => first.to_string(),
    }
}

fn probe(tool: &str, flag: &str) -> ToolStatus {
    let path = tool_path(tool);
    if !path.exists() {
        return ToolStatus {
            name: tool.into(),
            version: None,
            error: Some("introuvable — lancer scripts/fetch-sidecars.ps1".into()),
        };
    }

    let mut cmd = command(&path);
    cmd.arg(flag);

    match cmd.output() {
        Ok(out) if out.status.success() => ToolStatus {
            name: tool.into(),
            version: Some(short_version(tool, &String::from_utf8_lossy(&out.stdout))),
            error: None,
        },
        Ok(out) => ToolStatus {
            name: tool.into(),
            version: None,
            error: Some(
                String::from_utf8_lossy(&out.stderr)
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .unwrap_or("sortie en erreur sans message")
                    .trim()
                    .to_string(),
            ),
        },
        Err(e) => ToolStatus {
            name: tool.into(),
            version: None,
            error: Some(e.to_string()),
        },
    }
}

pub fn statuses() -> Vec<ToolStatus> {
    TOOLS.iter().map(|(tool, flag)| probe(tool, flag)).collect()
}

#[tauri::command]
pub fn sidecar_versions() -> Vec<ToolStatus> {
    statuses()
}

pub fn run_cli() -> i32 {
    let mut code = 0;
    println!("lavoir --doctor ({})", exe_dir().display());
    for status in statuses() {
        match (&status.version, &status.error) {
            (Some(version), _) => println!("  {:<10} {version}", status.name),
            (_, Some(error)) => {
                println!("  {:<10} ERREUR : {error}", status.name);
                code = 1;
            }
            _ => unreachable!("probe renvoie toujours version ou erreur"),
        }
    }
    code
}
