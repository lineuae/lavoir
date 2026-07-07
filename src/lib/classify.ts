// Range les tags exiftool bruts dans des catégories parlantes, et fabrique la
// phrase-choc. C'est la source unique de « ce qui compte comme fuite » —
// l'inspection et la vérification après lavage s'en servent toutes les deux.

import type { Tag, FileReport } from "./types";

export type Bucket = "loc" | "device" | "dates" | "software" | "other";

// Groupes purement structurels ou dérivés : jamais des données personnelles.
const IGNORED_GROUPS = new Set([
  "System",
  "File",
  "JFIF",
  "ExifTool",
  "Composite",
  "ICC_Profile",
  "ICC-header",
  "MPF",
  "APP14",
  "Adobe",
]);

// Tags techniques (dimensions, versions, couleur, orientation…) qui vivent dans
// des groupes de métadonnées mais ne révèlent rien sur l'auteur ou la scène.
const TECHNICAL = new Set([
  "Orientation",
  "XResolution",
  "YResolution",
  "ResolutionUnit",
  "YCbCrPositioning",
  "YCbCrSubSampling",
  "YCbCrCoefficients",
  "ExifVersion",
  "FlashpixVersion",
  "ComponentsConfiguration",
  "ColorSpace",
  "InteropIndex",
  "InteropVersion",
  "ImageWidth",
  "ImageHeight",
  "ExifImageWidth",
  "ExifImageHeight",
  "BitsPerSample",
  "Compression",
  "PhotometricInterpretation",
  "SamplesPerPixel",
  "RowsPerStrip",
  "StripOffsets",
  "StripByteCounts",
  "PlanarConfiguration",
  "ThumbnailOffset",
  "ThumbnailLength",
  "PreviewImageStart",
  "PreviewImageLength",
  "Padding",
  "JFIFVersion",
  "EncodingProcess",
  "ColorComponents",
  "BitDepth",
  "ColorType",
  "Filter",
  "Interlace",
  "Quality",
  "ImageSize",
  "Megapixels",
]);

const DEVICE_NAMES = new Set([
  "Make",
  "Model",
  "LensModel",
  "LensInfo",
  "LensMake",
  "LensSerialNumber",
  "SerialNumber",
  "BodySerialNumber",
  "CameraSerialNumber",
  "InternalSerialNumber",
  "OwnerName",
  "CameraOwnerName",
  "HostComputer",
]);

const SOFTWARE_NAMES = new Set([
  "Software",
  "ProcessingSoftware",
  "CreatorTool",
  "Application",
  "HistorySoftwareAgent",
  "WriterName",
  "ReaderName",
]);

// Groupes des « notes constructeur » : tout leur contenu décrit l'appareil.
const MAKER_GROUPS = /^(MakerNotes|Apple|Canon|Nikon|Sony|Panasonic|Olympus|Fujifilm|Samsung|Sigma|Ricoh|Pentax|Leica|GoPro|DJI)/;

export function classify(tag: Tag): Bucket | null {
  if (IGNORED_GROUPS.has(tag.group)) return null;
  if (TECHNICAL.has(tag.name)) return null;

  if (tag.group === "GPS" || tag.name.startsWith("GPS")) return "loc";
  if (/Date|Time/.test(tag.name)) return "dates";
  if (SOFTWARE_NAMES.has(tag.name)) return "software";
  if (DEVICE_NAMES.has(tag.name) || MAKER_GROUPS.test(tag.group)) return "device";
  return "other";
}

export type Grouped = {
  buckets: Record<Bucket, Tag[]>;
  sensitiveCount: number;
};

const EMPTY = (): Record<Bucket, Tag[]> => ({
  loc: [],
  device: [],
  dates: [],
  software: [],
  other: [],
});

export function group(tags: Tag[]): Grouped {
  const buckets = EMPTY();
  let sensitiveCount = 0;
  for (const tag of tags) {
    const bucket = classify(tag);
    if (bucket) {
      buckets[bucket].push(tag);
      sensitiveCount++;
    }
  }
  return { buckets, sensitiveCount };
}

export function sensitiveCount(tags: Tag[]): number {
  let n = 0;
  for (const tag of tags) if (classify(tag)) n++;
  return n;
}

export const BUCKET_LABELS: Record<Bucket, string> = {
  loc: "Localisation",
  device: "Appareil",
  dates: "Dates",
  software: "Logiciel",
  other: "Autres",
};

const REVEAL_PHRASES: Record<Bucket, string> = {
  loc: "sa position",
  device: "l'appareil",
  dates: "la date",
  software: "le logiciel",
  other: "des informations personnelles",
};

// « Cette photo révèle sa position, l'appareil et la date — 47 tags. »
export function shockLine(report: FileReport, grouped: Grouped): string {
  const noun = report.isVideo ? "Cette vidéo" : "Cette photo";
  const present: Bucket[] = (["loc", "device", "dates", "software", "other"] as Bucket[]).filter(
    (b) => grouped.buckets[b].length > 0,
  );

  if (present.length === 0) {
    return `${noun} ne contient aucune métadonnée sensible.`;
  }

  const phrases = present.map((b) => REVEAL_PHRASES[b]);
  const list =
    phrases.length === 1
      ? phrases[0]
      : `${phrases.slice(0, -1).join(", ")} et ${phrases[phrases.length - 1]}`;

  const n = grouped.sensitiveCount;
  return `${noun} révèle ${list} — ${n} ${n > 1 ? "champs" : "champ"}.`;
}
