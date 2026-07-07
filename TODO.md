# lavoir — plan de construction

Un lavoir pour les médias : ce qui entre (fichier local ou lien) en ressort propre — métadonnées inspectées puis retirées — et le lavoir lui-même ne garde rien : pas d'historique, pas de logs, temporaires purgés.

**Tagline** : « Tout ressort propre. Rien ne reste. »
**Cible v1** : app desktop Windows, usage perso. macOS/Linux plus tard (code portable, mais ni testé ni packagé).

## Décisions actées

- **Nom** : lavoir. Écartés : airlock (espace privacy encombré), sas / rinse / nickel (collisions ou trop génériques).
- **Stack** : Tauri 2 + SvelteKit 5 (adapter-static) + TypeScript, backend Rust — même famille que winglass.
- **Moteurs (sidecars)** : exiftool (inspection tous formats + lavage images), ffmpeg + ffprobe (lavage vidéos par remux + fusion et fixups yt-dlp), yt-dlp (téléchargements). Poids réel constaté : ~212 Mo — assumé en v1, piste d'allègement en réserve.
- **Langue UI** : français.
- **Design** : références Raycast / Linear — sombre d'abord, dense, clavier-friendly ; 2-3 couleurs (fond quasi-noir, texte, une seule couleur d'accent à trancher en passe design), 3-4 tailles de texte, pas de grilles de cartes.
- **Réseau** : aucune requête sauf le téléchargement demandé et la mise à jour yt-dlp (bouton manuel ; vérification au lancement en opt-in, désactivée par défaut). Pas de télémétrie, pas de géocodage, pas de tuiles de carte.
- **Traces** : pas d'historique persistant, pas de log d'URL, cache yt-dlp redirigé vers notre temp, temp purgé (démarrage + fermeture + janitor 10 min).
- **Originaux** : jamais modifiés — le lavage écrit une copie (suffixe à choisir en passe microcopie) ; écrasement sur demande explicite uniquement.

## Phase 1 — Squelette

- [x] `git init` + scaffold Tauri 2 / SvelteKit 5 / TS dans `C:\Users\linel\lavoir`
- [x] `scripts/fetch-sidecars.ps1` : versions épinglées (yt-dlp 2026.07.04, ffmpeg gyan essentials 8.1.2, exiftool 13.59) + SHA256 officielles vérifiées + renommage au format sidecar Tauri ; binaires dans `.gitignore` ; sync `exiftool_files` vers target/ pour le dev
- [x] Config Tauri : `externalBin` (4 moteurs), CSP stricte, NSIS seul, capabilities réduites à `core:default` (les plugins dialog/opener/notification seront ajoutés quand une phase les consomme), plugin single-instance
- [x] Coquille UI : deux vues (Laver / Récupérer) + Réglages avec diagnostic fonctionnel des moteurs, navigation, thème sombre de base
- [x] `PROJECT.md` du projet : conventions, règles no-trace, référence design, gestion des sidecars
- [x] Vérifié : dev OK (fenêtre stable, 4 moteurs résolus par `--doctor` sur le binaire debug) **et** packagé OK (doctor release 4/4, fenêtre release stable, `exiftool_files` confirmé dans l'installeur NSIS de 77,9 Mo)

## Phase 2 — Laver : images

- [ ] Entrée : drag & drop (événements Tauri, pas HTML5) + sélecteur de fichiers, multi-fichiers
- [ ] Inspection exiftool `-json` en mode `-stay_open` (process persistant, sinon ~1 s de démarrage Perl par appel) : tableau groupé — Localisation, Appareil, Dates, Logiciel, Autres
- [ ] GPS : coordonnées en clair + bouton copier + lien externe sur clic explicite — jamais de carte intégrée (zéro requête réseau avec des coordonnées)
- [ ] Résumé-choc par fichier : « Cette photo révèle sa position, l'appareil, la date — 47 tags »
- [ ] Vignette EXIF cachée : l'extraire et l'afficher si présente (peut montrer l'image d'avant recadrage — fuite méconnue, la montrer fait comprendre le produit)
- [ ] Lavage sans réencodage : `exiftool -all=` en **préservant Orientation + profil ICC** (sinon photos couchées et couleurs délavées — syntaxe exacte à valider sur corpus)
- [ ] Formats : JPEG, PNG (chunks tEXt/iTXt/zTXt/eXIf), WebP, HEIC (exiftool sait écrire le HEIC — à valider), TIFF
- [ ] Options : tout retirer (défaut) / GPS seulement / conserver la date de prise de vue
- [ ] Renommage neutre optionnel (`IMG_20240705_Paris.jpg` fuit aussi)
- [ ] Vérification intégrée : re-scan de la sortie par exiftool, diff avant/après (« 47 tags → 3 techniques restants »)
- [ ] Lot : tout laver d'un clic, sauvegarde à côté de l'original avec suffixe

## Phase 3 — Laver : vidéos

- [ ] Inspection : même voie exiftool (il lit QuickTime/Matroska, dont `com.apple.quicktime.location.ISO6709`)
- [ ] Lavage par remux sans réencodage : `ffmpeg -map_metadata -1 -c copy` + `-fflags +bitexact` (sinon ffmpeg signe la sortie avec son tag encoder) ; sortir les data streams Apple (`-map 0 -map -0:d`) — à valider sur corpus
- [ ] Corpus obligatoire : un vrai MOV iPhone avec GPS (les atomes QuickTime sont le cas qui compte)
- [ ] Formats : MP4, MOV, MKV, WebM
- [ ] Vérification post-lavage par exiftool, comme les images
- [ ] Progression pour gros fichiers (`-progress pipe:1`) — le remux est rapide mais pas instantané sur 2 Go

## Phase 4 — Récupérer (téléchargeur)

- [ ] Champ URL premier plan : autofocus, détection de collage, Ctrl+V global dans la vue
- [ ] Sonde `yt-dlp --dump-json --no-download` → carte titre / miniature / durée / source avant de lancer
- [ ] Choix simple : Meilleure qualité / 1080p / 720p / Audio seul (m4a) — mappés sur les format selectors
- [ ] Téléchargement dans le temp lavoir puis déplacement vers la destination ; `--ffmpeg-location` vers notre sidecar ; `--windows-filenames` + longueur bornée ; `--cache-dir` vers notre temp
- [ ] Progression machine-readable (`--progress-template`) → barre vitesse/ETA ; annulation propre (kill de l'arbre de process) ; file d'attente (2 simultanés max)
- [ ] Destination par défaut `Téléchargements\lavoir`, modifiable ; post-téléchargement : Ouvrir / Dossier / **Laver (coché par défaut, enchaîne sur les phases 2-3)**
- [ ] Erreurs humanisées : mapping stderr yt-dlp → compte privé / connexion requise / géobloqué / contenu supprimé / URL non supportée — jamais d'échec silencieux
- [ ] Option « utiliser ma session navigateur » (`--cookies-from-browser`) pour les posts visibles uniquement connecté à son propre compte — réalité Windows à tester : le chiffrement app-bound de Chrome la casse sans doute → probablement Firefox uniquement, l'UI doit le dire
- [ ] Périmètre : contenu public ou accessible à son propre compte — pas de contournement d'auth, pas de DRM
- [ ] Mise à jour yt-dlp : bouton `-U` dans les réglages + version affichée + opt-in « vérifier au lancement » (off par défaut)
- [ ] Test de réalité par plateforme (YouTube, TikTok, X, Reddit, Instagram — le plus dur —, stories publiques Snap) et messages adaptés

## Phase 5 — No-trace

- [ ] Janitor du temp : purge au démarrage, à la fermeture, et toutes les 60 s pour ce qui dépasse 10 min (couvrir les `.part` de yt-dlp)
- [ ] Liste des téléchargements en mémoire seulement, disparaît à la fermeture ; réglages JSON sans aucune URL ni nom de fichier
- [ ] Pas de logs persistants : erreurs dans l'UI, buffer mémoire de session
- [ ] Audit final avec Process Monitor : vérifier ce que les sidecars écrivent VRAIMENT hors du temp (caches, configs…)

## Phase 6 — Passe design

- [ ] Poser la direction avant de coder l'UI finale : palette définitive (quasi-noir + texte + un accent), une police, 3-4 tailles, densité d'outil pro — moodboard Raycast/Linear tenu jusqu'au bout
- [ ] Microcopie française : un ton, des verbes précis (« Laver », « Récupérer », « Tout ressort propre »), zéro copy générique
- [ ] États vides / erreurs / progression dessinés un par un (pas de skeletons réflexes)
- [ ] Icône : master SVG → PNG 1024 → `tauri icon` (motif goutte / planche de lavoir stylisée — à explorer)
- [ ] Raccourcis : Ctrl+V (coller une URL), Ctrl+O (ouvrir des fichiers), Échap (annuler)
- [ ] Notification système en fin de téléchargement long

## Phase 7 — Packaging & vérification finale

- [ ] Corpus de test réel : HEIC iPhone avec GPS, JPEG Android avec GPS, PNG avec tEXt, WebP, MOV iPhone, MP4 — script qui lave tout le corpus et **asserte zéro tag sensible** en sortie
- [ ] Tests Rust : janitor, parsing de progression, sanitisation des noms
- [ ] E2E réel : un lien public par plateforme, une vidéo > 2 Go, un lot de 20 photos, un fichier corrompu, une URL invalide
- [ ] Build NSIS, installation réelle, sidecars OK en prod, préchauffage exiftool au démarrage (déballage Perl au premier run)
- [ ] Attendus Windows à documenter : SmartScreen (app non signée), Defender qui flague parfois yt-dlp (faux positif connu)
- [ ] README court + captures

## Réserve (hors v1 — ne pas commencer)

- gallery-dl en second sidecar si les posts photo Instagram passent mal avec yt-dlp seul
- Carrousels / galeries multi-images
- macOS / Linux
- Accès téléphone (retour au plan PWA + Tailscale : l'UI SvelteKit se réutilise, pas les sidecars)
- Updater Tauri pour l'app elle-même
- Élargir le lavoir aux PDF / documents Office
- Alléger l'installeur : build ffmpeg « shared » (ffmpeg + ffprobe partagent les DLLs avcodec, ~moitié des 194 Mo actuels) — demande d'adapter le bundling des sidecars

## Pièges identifiés (ne pas les redécouvrir)

1. Orientation EXIF : strip naïf = photos couchées
2. Profil ICC : strippé = couleurs délavées (Display P3 iPhone)
3. Vignette EXIF : peut contenir l'image d'avant recadrage
4. GPS vidéo iPhone : atomes QuickTime, pas EXIF
5. ffmpeg signe sa sortie (tag encoder) : `-fflags +bitexact`
6. exiftool : premier lancement lent (déballage Perl) → `-stay_open` + préchauffage
7. Sidecars Tauri : suffixe target-triple obligatoire, résolution dev ≠ prod
8. yt-dlp écrit un cache dans %APPDATA% par défaut
9. Cookies Chrome/Windows : chiffrement app-bound → Firefox probablement seul à marcher
10. Titres de vidéos → noms de fichiers illégaux ou trop longs sous Windows
11. SmartScreen / Defender : avertissements attendus, faux positifs yt-dlp
12. Live Photos iPhone : paire .HEIC + .MOV — laver les deux
13. Fichiers partiels `.part` de yt-dlp : à couvrir par le janitor
