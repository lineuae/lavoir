import { invoke, convertFileSrc, Channel } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import type { FileReport, CleanOptions, CleanResult, CleanEvent, GpsFix } from "./types";

const IMAGE_EXTS = ["jpg", "jpeg", "png", "webp", "tiff", "tif", "gif", "heic", "heif", "avif"];
const VIDEO_EXTS = ["mp4", "mov", "mkv", "webm", "m4v", "avi"];

export const ACCEPTED_EXTS = [...IMAGE_EXTS, ...VIDEO_EXTS];

// Le webview (Chromium) ne sait pas afficher HEIC/HEIF/TIFF : on montre une
// pastille de format plutôt qu'une image cassée.
const RENDERABLE = new Set(["jpg", "jpeg", "png", "webp", "gif", "avif"]);

export function ext(path: string): string {
  const m = /\.([^.\\/]+)$/.exec(path);
  return m ? m[1].toLowerCase() : "";
}

export function isAccepted(path: string): boolean {
  return ACCEPTED_EXTS.includes(ext(path));
}

export function previewSrc(report: FileReport): string | null {
  return RENDERABLE.has(ext(report.path)) ? convertFileSrc(report.path) : null;
}

export function inspectFiles(paths: string[]): Promise<FileReport[]> {
  return invoke("inspect_files", { paths });
}

export function cleanFiles(
  paths: string[],
  options: CleanOptions,
  onEvent: (ev: CleanEvent) => void,
): Promise<CleanResult[]> {
  const channel = new Channel<CleanEvent>();
  channel.onmessage = onEvent;
  return invoke("clean_files", { paths, options, onEvent: channel });
}

export function extractThumbnail(path: string): Promise<string | null> {
  return invoke("extract_thumbnail", { path });
}

export async function pickFiles(): Promise<string[]> {
  const selected = await open({
    multiple: true,
    filters: [{ name: "Médias", extensions: ACCEPTED_EXTS }],
  });
  if (!selected) return [];
  return Array.isArray(selected) ? selected : [selected];
}

export function gpsText(gps: GpsFix): string {
  return `${gps.lat.toFixed(6)}, ${gps.lon.toFixed(6)}`;
}

// Ouverture explicite d'une carte externe — la seule requête réseau que
// l'app déclenche, et uniquement sur clic. OpenStreetMap plutôt que Google.
export function openOnMap(gps: GpsFix): Promise<void> {
  const url = `https://www.openstreetmap.org/?mlat=${gps.lat}&mlon=${gps.lon}#map=16/${gps.lat}/${gps.lon}`;
  return openUrl(url);
}

export function revealFolder(path: string): Promise<void> {
  return openPath(parentDir(path));
}

export function openFile(path: string): Promise<void> {
  return openPath(path);
}

function parentDir(path: string): string {
  const i = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return i > 0 ? path.slice(0, i) : path;
}
