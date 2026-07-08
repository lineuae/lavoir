// Reflète les structs Rust de laver.rs — synchronisé à la main.

export type Tag = { group: string; name: string; value: string };
export type GpsFix = { lat: number; lon: number };
export type ThumbInfo = { bytes: number };

export type FileReport = {
  path: string;
  fileName: string;
  fileType: string;
  mimeType: string;
  isVideo: boolean;
  canClean: boolean;
  tags: Tag[];
  gps: GpsFix | null;
  thumbnail: ThumbInfo | null;
  error: string | null;
};

export type CleanMode = "all" | "gps" | "keepDate";
export type CleanOptions = { mode: CleanMode; renameNeutral: boolean };

export type CleanResult = {
  src: string;
  dst: string | null;
  dstName: string | null;
  afterTags: Tag[];
  ok: boolean;
  error: string | null;
};

// Progression du lavage vidéo (remux ffmpeg), diffusée par Channel, indexée par
// chemin source. Les images n'en émettent pas (exiftool est instantané).
export type CleanEvent = { type: "progress"; src: string; percent: number };

// --- Récupérer (miroir de recuperer.rs / settings.rs) ------------------------

export type Probe = {
  title: string;
  source: string;
  uploader: string | null;
  durationSeconds: number | null;
  maxHeight: number | null;
  isLive: boolean;
  webpageUrl: string;
};

export type Quality = "best" | "p1080" | "p720" | "audio";

export type DownloadRequest = {
  url: string;
  quality: Quality;
  wash: boolean;
  destination: string;
  cookiesFromBrowser: string | null;
};

// `type` discrimine la variante — même forme que l'enum Rust sérialisé.
export type DownloadEvent =
  | { type: "queued" }
  | { type: "started" }
  | { type: "progress"; downloaded: number | null; total: number | null; speed: number | null; eta: number | null }
  | { type: "postprocess"; stage: string }
  | { type: "washing" }
  | { type: "completed"; path: string; name: string }
  | { type: "failed"; message: string }
  | { type: "cancelled" };

export type Settings = {
  destination: string | null;
  cookiesFromBrowser: string | null;
  checkYtdlpOnLaunch: boolean;
};

export type UpdateResult = {
  updated: boolean;
  version: string | null;
  message: string;
};
