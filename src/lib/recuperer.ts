import { invoke, Channel } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { Probe, DownloadRequest, DownloadEvent, Settings, UpdateResult } from "./types";

export function probeUrl(url: string): Promise<Probe> {
  return invoke("probe_url", { url });
}

// Le Channel diffuse la progression jusqu'à l'événement terminal ; on renvoie
// l'id du job pour pouvoir l'annuler.
export function startDownload(
  req: DownloadRequest,
  onEvent: (ev: DownloadEvent) => void,
): Promise<string> {
  const channel = new Channel<DownloadEvent>();
  channel.onmessage = onEvent;
  return invoke("start_download", { req, onEvent: channel });
}

export function cancelDownload(jobId: string): Promise<void> {
  return invoke("cancel_download", { jobId });
}

export function defaultDestination(): Promise<string> {
  return invoke("default_destination");
}

export async function pickDestination(current: string): Promise<string | null> {
  const chosen = await open({ directory: true, defaultPath: current || undefined });
  return typeof chosen === "string" ? chosen : null;
}

export function getSettings(): Promise<Settings> {
  return invoke("get_settings");
}

export function setSettings(settings: Settings): Promise<void> {
  return invoke("set_settings", { settings });
}

export function updateYtdlp(): Promise<UpdateResult> {
  return invoke("update_ytdlp");
}

export function launchUpdateStatus(): Promise<UpdateResult | null> {
  return invoke("launch_update_status");
}

const QUALITY_LABELS = {
  best: "Meilleure qualité",
  p1080: "1080p",
  p720: "720p",
  audio: "Audio (m4a)",
} as const;

export const QUALITIES = Object.entries(QUALITY_LABELS).map(([id, label]) => ({
  id: id as keyof typeof QUALITY_LABELS,
  label,
}));

const isUrl = /^https?:\/\/\S+$/i;
export function looksLikeUrl(text: string): boolean {
  return isUrl.test(text.trim());
}

export function formatDuration(seconds: number | null): string | null {
  if (seconds === null || !Number.isFinite(seconds)) return null;
  const s = Math.round(seconds);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const pad = (n: number) => n.toString().padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(sec)}` : `${m}:${pad(sec)}`;
}

export function formatBytes(n: number | null): string {
  if (n === null || !Number.isFinite(n)) return "—";
  if (n < 1024) return `${n} o`;
  const units = ["Ko", "Mo", "Go"];
  let value = n / 1024;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i++;
  }
  return `${value.toFixed(value >= 100 ? 0 : 1)} ${units[i]}`;
}

export function formatEta(seconds: number | null): string | null {
  if (seconds === null || !Number.isFinite(seconds) || seconds < 0) return null;
  const s = Math.round(seconds);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  const rem = s % 60;
  return rem === 0 ? `${m} min` : `${m} min ${rem} s`;
}
