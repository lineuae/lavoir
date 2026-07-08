# lavoir

Un lavoir pour les médias : ce qui entre — un fichier local ou un lien — en
ressort propre, ses métadonnées inspectées puis retirées. Et le lavoir lui-même
ne garde rien : pas d'historique, pas de journal d'URL, temporaires purgés.

> Tout ressort propre. Rien ne reste.

Application de bureau Windows (Tauri 2 + SvelteKit 5, backend Rust), pour un
usage personnel. macOS / Linux ne sont pas visés en v1.

## Deux gestes

**Laver.** Dépose une photo ou une vidéo : lavoir montre ce qu'elle révèle —
position GPS en clair, appareil, dates, logiciel, jusqu'à la vignette EXIF
cachée qui peut précéder un recadrage — puis retire tout, **sans réencoder** et
**sans jamais toucher l'original** (le lavage écrit une copie `-propre`). Chaque
lavage est vérifié : la copie est re-scannée et le diff avant/après est affiché
(« 47 → 0 champ sensible »).

- Images (exiftool) : JPEG, PNG, WebP, TIFF, GIF, HEIC/HEIF/AVIF. L'orientation
  et le profil couleur (ICC) sont préservés — sans quoi les photos ressortent
  couchées ou délavées.
- Vidéos (remux ffmpeg, copie des flux bit à bit) : MOV, MP4, M4V, MKV, WebM. Les
  atomes de position QuickTime et les pistes de métadonnées Apple sont retirés.
- Trois modes pour les images : tout retirer, GPS seulement, ou tout sauf la
  date de prise de vue.

**Récupérer.** Colle un lien public : lavoir sonde le média (titre, source,
durée, qualité) puis le télécharge via yt-dlp. Le fichier brut **ne quitte
jamais le dossier temporaire** — il y est téléchargé, lavé sur place, et seule la
copie propre est déplacée vers la destination. Contenu public ou accessible à ton
propre compte uniquement : pas de contournement d'authentification, pas de DRM ;
les directs et les playlists sont hors périmètre v1.

## Ne rien laisser

- Aucune requête réseau en dehors du téléchargement que tu demandes et de la
  mise à jour manuelle de yt-dlp. Pas de télémétrie, pas de géocodage, aucune
  tuile de carte : les coordonnées s'affichent en clair, avec un lien externe
  seulement sur clic explicite.
- Pas d'historique, pas de journal d'URL ni de nom de fichier. Les réglages
  (JSON) ne gardent que des préférences : destination, cookies, opt-in de mise à
  jour.
- Le cache de yt-dlp est redirigé dans notre dossier temporaire, lui-même purgé
  au démarrage, à la fermeture, et balayé toutes les minutes pour les
  téléchargements abandonnés.

## Construire depuis les sources

Prérequis : Node 20+, la toolchain Rust **MSVC** (pas GNU/mingw), les VS Build
Tools (charge de travail C++), WebView2 (préinstallé sur Windows 11).

```powershell
# 1. Récupérer les moteurs embarqués (exiftool, ffmpeg, ffprobe, yt-dlp).
#    Versions épinglées, sommes SHA256 vérifiées ; ~212 Mo, non commités.
.\scripts\fetch-sidecars.ps1

# 2. Développement (rechargement à chaud) :
npm install
npm run tauri dev

# 3. Build de production → installeur NSIS dans src-tauri/target/release/bundle :
npm run tauri build
```

Vérifier que les moteurs sont bien résolus, sans ouvrir de fenêtre :

```powershell
.\src-tauri\target\release\lavoir.exe --doctor > doctor.txt; Get-Content doctor.txt
```

## À prévoir sous Windows

L'application n'est pas signée : **SmartScreen** affichera un avertissement au
premier lancement (« Informations complémentaires » → « Exécuter quand même »).
**Windows Defender** signale parfois `yt-dlp.exe` comme suspect — c'est un faux
positif connu des exécutables PyInstaller, pas un comportement de lavoir. Le
binaire est épinglé par version et sa somme SHA256 est vérifiée à chaque
récupération (`scripts/fetch-sidecars.ps1`).

## Corpus de test

Le lavage est couvert par des tests d'intégration Rust (`cargo test`) qui
injectent GPS / appareil / dates dans des graines JPEG, PNG, WebP, TIFF et MOV,
lavent, puis re-scannent en asserant zéro tag sensible. Pour éprouver le lavage
sur de vrais fichiers (HEIC iPhone géotagué, MOV géotagué, etc.), pointe le
harnais vers un dossier :

```powershell
$env:LAVOIR_CORPUS = "C:\chemin\vers\mes\photos"; cargo test corpus -- --nocapture
```

## Licence

MIT.
