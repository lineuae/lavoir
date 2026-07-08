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

- [x] Entrée : drag & drop (événements Tauri `onDragDropEvent`, chemins réels) + sélecteur de fichiers (plugin dialog), multi-fichiers, dédup
- [x] Inspection exiftool `-json -a -u -G1 -c %.6f` en mode `-stay_open` (session persistante sérialisée par Mutex, marqueurs `{ready<seq>}` sur stdout+stderr) ; classifieur `classify.ts` : Localisation / Appareil / Dates / Logiciel / Autres, avec dénylist technique
- [x] GPS : coordonnées décimales signées en clair + copier + lien OpenStreetMap sur clic explicite — jamais de carte intégrée
- [x] Résumé-choc par fichier (`shockLine`) : « Cette photo révèle sa position, l'appareil et la date — 47 champs. »
- [x] Vignette EXIF cachée : détectée via ThumbnailLength, extraite à la demande (`extract_thumbnail`, base64) avec l'avertissement « image d'avant recadrage »
- [x] Lavage sans réencodage : `-all= -tagsFromFile @ -Orientation -ICC_Profile -overwrite_original` — orientation et profil couleur préservés (validé sur corpus : GPS/appareil/dates/vignette supprimés, orientation « Rotate 90 CW » conservée)
- [x] Formats : JPEG, PNG, WebP, TIFF, GIF câblés ; HEIC/HEIF/AVIF autorisés au lavage mais **write HEIC non validé sur vrai fichier** (repoussé au corpus phase 7 — pas de HEIC sous la main) ; aperçu HEIC/TIFF impossible dans le webview → pastille de format
- [x] Options : tout retirer (défaut) / GPS seulement / conserver la date de prise de vue
- [x] Renommage neutre optionnel (`media-0001.jpg`) ; sinon suffixe `-propre` (mot à confirmer en passe microcopie)
- [x] Vérification intégrée : re-scan de la copie lavée, diff avant/après affiché (« 47 → 0 champ sensible ») ; alerte rouge si résidu
- [x] Lot : « Laver N fichiers » d'un clic, sortie à côté de l'original, original jamais modifié
- [x] Vérifié : 2 tests d'intégration Rust (cycle inspect→clean→rescan, modes all + gps, original préservé) ; protocole stay_open validé (Node, 184 ms puis 16 ms) ; svelte-check + cargo test verts ; app démarre sans erreur
- [x] Clic/dépôt réel dans la fenêtre confirmé par l'utilisateur (2026-07-07) : dépôt + aperçu + lavage OK en conditions réelles
- [ ] Reste à valider dès qu'un vrai fichier est dispo (pas bloquant) : photo verticale → orientation conservée après lavage ; photo iPhone P3 → couleurs non délavées (ICC) ; **HEIC iPhone → le lavage écrit-il vraiment ?** (le cas le plus important, cf. phase 7 corpus)

## Phase 3 — Laver : vidéos

- [x] Inspection : même voie exiftool (lit les Keys QuickTime, dont `com.apple.quicktime.location.ISO6709`, restitué en Composite signé → `report.gps` renseigné pour la vidéo comme pour l'image)
- [x] Lavage par remux sans réencodage : `ffmpeg -y -nostdin -map 0 -map -0:d -map_metadata -1 -c copy -fflags +bitexact` — retire toutes les métadonnées de conteneur **et** les flux de données Apple (`mebx` : position, timed metadata) ; A/V copiés bit à bit, rotation (portrait) préservée, sortie qui décode sans erreur. `-map -0:d` est sans effet (pas d'erreur) si le fichier n'a pas de data stream. Les trois modes image se ramènent au même nettoyage complet (pas de lavage partiel lossless sur vidéo) — noté dans l'UI (« Vidéos : nettoyage complet »)
- [x] Classifieur rendu vidéo-conscient (`classify.ts`) : les groupes structurels `QuickTime`/`TrackN` ne comptent que la date canonique du conteneur (`CreateDate`/`ModifyDate`), jamais les dates répétées par piste ni les tags `*Time` (TimeScale…) ; dates nulles `0000:00:00` (remises à zéro par le remux) ignorées ; variantes localisées `-eng-US` dédupliquées ; notes constructeur `Apple-maker-note*` → appareil. Vérifié sur le vrai MOV iPhone 14 : **13 → 0 champ sensible**
- [x] Corpus : validé sur un vrai conteneur MOV iPhone 14 (HEVC) avec un atome `com.apple.quicktime.location.ISO6709` **injecté** (identique à une capture) — le retrait de l'atome de position, des tags `com.apple.quicktime.*` et des 5 pistes `mebx` est prouvé. Reste souhaitable : un MOV réellement géotagué à la prise (la localisation était coupée sur les fichiers de test)
- [x] Formats : MOV/MP4/M4V/MKV/WEBM câblés (`is_remuxable_video`) ; exercé sur MOV
- [x] Vérification post-lavage par exiftool (re-scan de la sortie, diff avant/après), comme les images
- [x] Test d'intégration Rust `strips_a_video` (fixture `seed.mov`, 1,6 Ko : injection Keys GPS/Make/Model → remux → re-scan sans résidu, original préservé) ; `cargo test` 3/3 vert, `svelte-check` 0 erreur
- [ ] Progression pour gros fichiers (`-progress pipe:1`) — reporté : le remux est rapide (18 Mo quasi instantané) mais un 2 Go bloque quelques secondes sans barre. À câbler via un Channel Tauri quand on voudra le confort

## Phase 4 — Récupérer (téléchargeur)

- [x] Champ URL premier plan : autofocus, détection de collage (`onpaste` → sonde si ça ressemble à un lien), Ctrl+V global dans la vue (champ non focalisé → lecture presse-papiers + sonde) ; Entrée sonde, puis Entrée relance en téléchargement
- [x] Sonde `yt-dlp --dump-single-json --no-playlist` → carte titre / source (extractor) / durée / auteur / qualité max. **Miniature distante volontairement écartée** : la charger depuis un CDN violerait la CSP stricte (`img-src` ne liste aucun hôte distant) et l'esprit no-trace (requête webview → serveur tiers), au même titre qu'on refuse les tuiles de carte. Carte textuelle, dans la veine Raycast/Linear. `_type == "playlist"` et `is_live` refusés en v1
- [x] Choix simple : Meilleure qualité / 1080p / 720p / Audio (m4a) — mappés sur les format selectors (`bv*+ba/b` + `--merge-output-format mp4` ; audio `-x --audio-format m4a`)
- [x] **Téléchargement dans le temp → lavage dans le temp → seule la copie propre est déplacée vers la destination** (le fichier brut ne sort jamais du temp) ; `--ffmpeg-location` vers le dossier des sidecars (résolu par `tool_path`, sans suffixe en dev/prod), `--windows-filenames` + `--trim-filenames 200`, `--cache-dir` vers notre temp, `--no-mtime` (pas de fuite de date par l'horodatage du fichier)
- [x] Progression machine-readable (`--progress-template` préfixé `LAVOIR`/`LAVOIR_PP`, stdout+stderr fusionnés) streamée par un `Channel` Tauri → barre vitesse/ETA ; annulation par kill de l'arbre de process (`taskkill /T /F`) ; file d'attente 2 simultanés max (sémaphore Mutex+Condvar maison)
- [x] Destination par défaut `Téléchargements\lavoir`, modifiable (sélecteur de dossier) et persistée ; post-téléchargement : Ouvrir / Dossier ; **Laver coché par défaut, enchaîne sur les phases 2-3** (réutilise `laver::wash_download` : remux vidéo/audio, exiftool image)
- [x] Erreurs humanisées : mapping stderr yt-dlp → vidéo privée / connexion requise / vérification d'âge / géobloqué / contenu supprimé / lien non supporté / 404 / réseau — jamais d'échec silencieux (repli sur la dernière ligne `ERROR:` nettoyée)
- [x] Option « utiliser ma session navigateur » (`--cookies-from-browser`) dans les réglages, avec l'avertissement Windows affiché : Chrome/Edge chiffrent leurs cookies (app-bound) → Firefox est le choix fiable
- [x] Périmètre : contenu public ou accessible à son propre compte — pas de contournement d'auth, pas de DRM ; live et playlists hors périmètre v1
- [x] Mise à jour yt-dlp : bouton `-U` dans les réglages + version affichée + opt-in « vérifier au démarrage » (off par défaut, résultat rangé dans un état lu à l'ouverture des réglages). Note : `-U` réécrit le binaire épinglé — attendu ; il échoue proprement dans Program Files (droits) → message dédié
- [x] Vérifié E2E sur réseau réel (binaire épinglé, vidéo courte stable) : sonde (tous les champs parsés OK), téléchargement audio (lignes `LAVOIR` de progression + `ExtractAudio`/`MoveFiles` parsées, `.m4a` produit), lavage du vrai fichier (remux retire `Encoder: Lavf…` + handler `Metadata`, audio bit-à-bit), et chemin d'erreur (« Video unavailable » → « Contenu supprimé ou indisponible »). 3 tests Rust (parsing progression, humanisation, dedup) + `svelte-check` 0/0
- [ ] Test de réalité par plateforme — **YouTube validé** ci-dessus ; reste TikTok, X, Reddit, Instagram (le plus dur), stories publiques Snap : à mener avec la matrice de la phase 7. Piste : affiner les format selectors si « best » ramène du webm là où mp4 conviendrait mieux (préférer `ext=mp4`/`ext=m4a` sans réencodage)

## Phase 5 — No-trace

- [x] Janitor du temp (`janitor.rs`) : purge complète au démarrage **et** à la fermeture (`remove_dir_all(temp_root)`, cache yt-dlp compris) + balayage toutes les 60 s des dossiers `dl/<job>` abandonnés de plus de 10 min (décision pure `should_remove`, testée), en épargnant les téléchargements actifs reconnus par leur id (`DownloadManager::active_ids`). À la fermeture (`RunEvent::ExitRequested`), on **coupe d'abord** les téléchargements en cours (`cancel_all` → `taskkill /T`) pour qu'aucun yt-dlp/ffmpeg ne survive à l'app, puis on purge. Les `.part` de yt-dlp vivent dans `dl/<job>` → couverts. Purge au démarrage sûre malgré single-instance : la 2ᵉ instance sort dans le `setup` du plugin (`std::process::exit`) avant le nôtre (vérifié dans la source du plugin)
- [x] Liste des téléchargements en mémoire seulement (état `$state` de la page, aucune persistance — disparaît au changement de vue et à la fermeture) ; réglages JSON = uniquement destination / cookies / opt-in maj, **jamais** d'URL ni de nom de fichier
- [x] Pas de logs persistants : erreurs affichées dans l'UI, buffer mémoire des 80 dernières lignes le temps du job puis jeté ; `--ignore-config` + `--cache-dir` vers notre temp sur chaque appel yt-dlp
- [x] Audit par diff de répertoires autour d'un vrai téléchargement : **aucun** nouveau fichier dans `%APPDATA%` / `%LOCALAPPDATA%` / profil, `%APPDATA%\yt-dlp` jamais créé (piège #8 neutralisé par `--cache-dir`), notre temp ne contient que le fichier attendu. Le Process Monitor complet sur l'app **packagée** (writes transitoires, déballage Perl d'exiftool) est replié dans la vérification finale de la phase 7

## Phase 6 — Passe design

- [x] Direction posée dans `app.css` (système arrêté, plus « provisoire ») : rampe de **gris froids** (fond `#0d0f13` → texte `#e6e9ef`, avec un niveau `faint` pour le tertiaire), **deux** couleurs porteuses seulement — accent teal aqua `#45c8b2` (l'eau + le propre + l'action) et danger `#e2665f` (fuite/erreur). Police système Segoe UI Variable. **Échelle de texte tenue à 11/12/13/15 px** (le 10 px et le 14 px consolidés partout), graisses 400/500/600. Sélection et barre de défilement soignées pour le sombre. Densité d'outil pro, référence Raycast/Linear tenue
- [x] Microcopie : ton posé et précis, verbes d'action (Laver / Récupérer / Ajouter / Vider), signature « Tout ressort propre. Rien ne reste. » discrète sous l'état vide de Laver ; **suffixe `-propre` confirmé** (colle à la tagline) ; « Renommer en neutre »
- [x] États dessinés un par un (aucun skeleton réflexe) : état vide de Laver retravaillé (goutte, hiérarchie, whitespace, indice clavier), zone de dépôt et overlay, chargements en texte, erreurs humanisées en danger, progression complète dans `DownloadRow` (file / octets+vitesse+ETA / barre indéterminée si taille inconnue / post-traitement / lavage / terminé / annulé / erreur)
- [x] Icône définitive : piste B du projet design (« goutte + onde ») — goutte teal relevée (dégradé vertical + reflet spéculaire) au-dessus de deux arcs teal qui rident la surface, sur carré arrondi quasi-noir ; coordonnées reprises du SVG `icoB`. Générée sans outillage externe (`gen-icon.ps1`, GDI+) → `tauri icon` a régénéré tout le jeu (ico/icns/PNG/iOS/Android). Vérifiée : l'onde s'efface aux petites tailles, la goutte reste lisible à 32/16 px
- [x] Raccourcis : Ctrl+V (coller une URL, déjà en place) + Échap (effacer la saisie) dans Récupérer ; Ctrl+O (ouvrir des fichiers) + Échap (vider la liste) dans Laver
- [ ] Notification système en fin de téléchargement long — **en attente** : demande une nouvelle dépendance (`tauri-plugin-notification` + capability). À trancher avant de l'ajouter (règle « pas de dépendance sans validation »)

## Phase 7 — Packaging & vérification finale

- [x] Harnais de corpus (`laver.rs`) : `strips_common_image_formats` injecte GPS/appareil/dates dans des graines **JPEG/PNG/WebP/TIFF**, lave, et asserte zéro tag sensible via une denylist **indépendante de `classify.ts`** (vrai contre-contrôle, `is_sensitive`) ; le remux **MOV** reste couvert par `strips_a_video`. Le harnais a débusqué et fait corriger deux vrais défauts de format : identité IFD0 des **TIFF** qui survivait à `-all=` (piège 14) et **WebP** porteur de métadonnées vu « Extended WEBP » donc affiché non lavable. Exercé de bout en bout sur un dossier de fichiers injectés (5 fichiers : JPEG/PNG/WebP/TIFF/MOV, chacun N→0)
- [x] **Vrai corpus validé (2026-07-08)** — harnais lancé sur `C:\Users\linel\Videos\ltest` : 2 vrais HEIC iPhone 15 Pro Max géotagués + 1 MOV géotagué à la prise + 1 JPEG, chacun **N→0** sensible (HEIC **33→0**, MOV **12→0**). **write HEIC enfin prouvé** : après lavage l'image reste intacte (3213×5712, HEVC) et le profil **Display P3 est préservé** — l'avertissement exiftool « ICC_Profile deleted. colors may be affected » est **transitoire** : le `-all=` le supprime puis `-tagsFromFile @ -ICC_Profile` le recopie (bloc ICC identique à l'original après lavage). Un MOV déjà lavé en app re-scanné **0→0** (recoupe le lavage in-app). Reproductible : `$env:LAVOIR_CORPUS="…"; cargo test corpus -- --nocapture`
  - Chemin de transfert (pour de futurs fichiers) : **ni Phone Link ni OneDrive**. Phone Link convertit (HEIC→JPG, HEVC→H.264) et retire le GPS ; OneDrive « fichiers à la demande » laisse des placeholders tronqués / extensions mélangées. Passer par iPhone en USB → Explorateur → DCIM (Réglages → Photos → « Vers Mac ou PC → Conserver les originaux ») ou iCloud.com → originaux. Et la localisation doit être **active à la prise**
- [x] Tests Rust : janitor (`should_remove`), parsing de progression (`progress_parsing_handles_missing_fields`), sanitisation des noms (`output_path_names_and_avoids_collisions` côté lavage + `dedup_avoids_clobbering` côté téléchargement). `cargo test` **10/10 vert**, `svelte-check` 0/0
- [ ] **Reste (réseau/matériel) : E2E réel** — un lien public **par plateforme** (YouTube validé phase 4 ; restent TikTok/X/Reddit/Instagram/Snap), une vidéo **> 2 Go**, un lot de 20 photos. Les chemins d'erreur (URL invalide, lien non supporté, contenu supprimé) sont déjà couverts par `humanize_maps_known_failures`
- [x] Build NSIS de production vert : `lavoir_0.1.0_x64-setup.exe` (78,2 Mio), `--doctor` release **4/4** (yt-dlp 2026.07.04, ffmpeg/ffprobe 8.1.2, exiftool 13.59), sidecars + `exiftool_files` bien à côté de l'exe release
- [ ] **Reste (matériel) : installation réelle** de l'app packagée (double-clic sur l'installeur), sidecars OK en prod hors arbre de build, préchauffage exiftool au 1er run (déballage Perl), et l'audit ProcMon complet sur l'app installée
- [x] Attendus Windows documentés dans le README : SmartScreen (app non signée), Defender qui flague parfois yt-dlp (faux positif PyInstaller connu)
- [x] README court écrit (deux gestes, no-trace, build depuis les sources, attendus Windows, corpus)
- [ ] **Reste : captures d'écran** du README (depuis l'app lancée — impossible à faire sans œil sur la fenêtre)

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
14. TIFF : `-all=` ne vide pas l'IFD0 (c'est la structure de l'image) → Make/
    Model/Software/Artist y survivent. Effacement nommé obligatoire
    (`IDENTITY_STRIP`/`DATE_STRIP` dans `laver.rs`) — trouvé au corpus phase 7
