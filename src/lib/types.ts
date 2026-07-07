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
