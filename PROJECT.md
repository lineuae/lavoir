# lavoir — guide développeur

Guide pour reprendre le développement de ce projet.
À tenir à jour quand l'architecture bouge.

## Ce qu'est lavoir

Un lavoir pour les médias. Deux fonctions, un principe :

- **Laver** — inspecter puis retirer les métadonnées de photos et vidéos
  (EXIF, GPS, atomes QuickTime…), sans réencodage, sans jamais modifier
  l'original.
- **Récupérer** — télécharger un média depuis un lien public (yt-dlp),
  qui ressort lavé par défaut.
- **No-trace** — l'app ne garde rien : pas d'historique, pas de log d'URL,
  temporaires purgés. « Tout ressort propre. Rien ne reste. »

Tauri 2 + SvelteKit 5 (runes) + Tailwind 4, backend Rust. Windows x64
uniquement en v1. UI en français.

**Le plan de construction vit dans `TODO.md`** : phases, cases à cocher,
décisions actées, pièges déjà identifiés (ne pas les redécouvrir — lire
cette section avant les phases 2-4). Ce fichier-ci décrit comment
travailler ; TODO.md décrit quoi faire et où on en est.

## Arborescence

```
lavoir/
├── PROJECT.md / TODO.md      ← ce guide + le plan de construction
├── package.json             npm ; scripts dev/build/check/tauri
├── svelte.config.js         adapter-static, fallback index.html (SPA)
├── vite.config.js           port 1420 fixe, src-tauri ignoré
├── src/
│   ├── app.css              jetons de design (@theme Tailwind 4) — provisoires, phase 6
│   ├── app.html             coquille, lang="fr", fond sombre anti-flash
│   ├── lib/
│   │   ├── types.ts         miroir des structs Rust de laver.rs
│   │   ├── classify.ts      rangement des tags en catégories + phrase-choc (source unique du « sensible »)
│   │   ├── laver.ts         wrappers invoke + dépôt/aperçu/carte/ouverture
│   │   ├── recuperer.ts     wrappers invoke + Channel de progression + formatage
│   │   ├── FileCard.svelte  carte par fichier : aperçu, résumé, GPS, vignette, résultat
│   │   └── DownloadRow.svelte  ligne d'un téléchargement : progression, annuler, ouvrir
│   └── routes/
│       ├── +layout.svelte   barre du haut : Laver / Récupérer + Réglages
│       ├── +page.svelte     vue Laver (dépôt, options, lavage lot — fonctionnel)
│       ├── recuperer/       vue Récupérer (URL, sonde, qualité, file — fonctionnel)
│       └── reglages/        version app, moteurs, maj yt-dlp, cookies (fonctionnel)
├── src-tauri/
│   ├── tauri.conf.json      externalBin, resources (exiftool_files), CSP, NSIS
│   ├── capabilities/        minimales : core:default + dialog/opener open
│   ├── binaries/            moteurs épinglés — .gitignoré, cf. fetch-sidecars
│   └── src/
│       ├── main.rs          entrée fine → lavoir_lib::run()
│       ├── lib.rs           builder + plugins + states + setup + routage --doctor
│       ├── doctor.rs        résolution des sidecars + sonde de versions + command()
│       ├── janitor.rs       purge du temp : démarrage + fermeture + balayage 60 s (+ tests)
│       ├── laver.rs         session exiftool stay_open + inspect/clean/thumbnail/wash_download (+ tests)
│       ├── recuperer.rs     sonde yt-dlp + téléchargement streamé + file + maj (+ tests)
│       └── settings.rs      préférences JSON (destination, cookies, maj au lancement)
├── scripts/
│   ├── fetch-sidecars.ps1   télécharge/vérifie/installe les moteurs épinglés
│   └── gen-icon.ps1         PNG source de l'icône (placeholder phase 1)
└── assets/icon-source.png   1024×1024, nourrit `npm run tauri -- icon`
```

## Lancer le projet

```powershell
# Clone frais ou après `cargo clean` : installer les moteurs d'abord.
.\scripts\fetch-sidecars.ps1

# Dev avec rechargement à chaud :
npm run tauri dev

# Vérifications rapides :
npm run check                                   # svelte-check
cargo check   # (depuis src-tauri)

# Build production → src-tauri/target/release/ + bundle/nsis/ :
npm run tauri build

# Sonde des moteurs sans ouvrir de fenêtre (sortie visible uniquement redirigée,
# l'exe release est en subsystem windows) :
.\src-tauri\target\release\lavoir.exe --doctor > doctor.txt; Get-Content doctor.txt
```

## Toolchain (critique)

- **Rust doit résoudre vers MSVC, pas GNU/mingw.** Même piège que winglass :
  si le link échoue avec `export ordinal too large`, un Rust GNU autonome
  passe devant rustup. Correctif immédiat :
  `$env:Path = "$env:USERPROFILE\.cargo\bin;" + $env:Path`.
- Node 20+, VS Build Tools (workload VC++), WebView2 (préinstallé Win11).

## Moteurs embarqués (sidecars)

Quatre binaires épinglés dans `scripts/fetch-sidecars.ps1` — version + URL +
SHA256 officielle. Jamais commités. Pour monter une version : mettre à jour
le tableau `$pins` (nouvelle somme depuis la source officielle), relancer le
script, relancer les vérifications.

| Moteur | Rôle | Source de la somme |
|---|---|---|
| yt-dlp | téléchargements | `SHA2-256SUMS` de la release GitHub |
| ffmpeg + ffprobe | lavage vidéo (remux), fusion yt-dlp | `<package>.zip.sha256` sur gyan.dev |
| exiftool | inspection tous formats + lavage images | `checksums.txt` sur exiftool.org |

Subtilités à ne pas perdre :

- **Suffixe target-triple obligatoire** dans `src-tauri/binaries/`
  (`yt-dlp-x86_64-pc-windows-msvc.exe`). Le CLI Tauri les copie sans suffixe
  à côté de l'exe (target/debug en dev, dossier d'installation en prod).
- **exiftool = launcher + dossier `exiftool_files`** (distribution Perl). Le
  launcher cherche le dossier à côté de lui. En prod, `bundle.resources` le
  place à la racine d'installation ; en dev, le CLI ne copie pas les
  ressources → fetch-sidecars synchronise le dossier vers target/{debug,release}.
  Symptôme d'un oubli : exiftool en erreur dans Réglages après `cargo clean`.
- **Le nom du launcher exiftool pilote ses options** : `exiftool(-k).exe`
  ferait une pause clavier ; le renommage sidecar supprime ce comportement.
- La résolution runtime vit dans `doctor::tool_path()` : à côté de l'exe,
  avec repli dev vers `src-tauri/binaries/`. Toute future invocation d'un
  moteur passe par là.
- Tailles réelles : ~212 Mo dans binaries/ (ffmpeg 97 + ffprobe 97 + yt-dlp 17
  + exiftool ~11 avec son dossier). Piste phase 7 : build ffmpeg « shared »
  (DLLs communes aux deux exes) pour dégonfler l'installeur.
- SourceForge : télécharger via `downloads.sourceforge.net` — l'endpoint
  `/download` sert une page HTML hors navigateur.
- Attendus Windows : SmartScreen râle (app non signée) et Defender flague
  parfois yt-dlp (faux positif connu). À documenter dans le README (phase 7).

## Règles no-trace (à respecter dans le code)

- Aucune requête réseau en dehors : du téléchargement demandé, et de la mise
  à jour de yt-dlp déclenchée explicitement. Pas de télémétrie, pas de
  géocodage, pas de tuiles de carte (les coordonnées GPS s'affichent en
  clair, lien externe sur clic explicite seulement).
- Pas d'historique persistant, pas de log d'URL ni de nom de fichier. Les
  réglages (JSON) ne contiennent que des préférences.
- Tout fichier temporaire va sous `recuperer::temp_root()` (`%TEMP%/lavoir`),
  couvert par le janitor (`janitor.rs`) : purge complète au démarrage et à la
  fermeture, balayage 60 s des dossiers `dl/<job>` abandonnés >10 min (jamais
  un téléchargement actif, reconnu par son id). La fermeture coupe d'abord les
  téléchargements en cours (`RunEvent::ExitRequested` → `cancel_all`) avant de
  purger — sinon les process yt-dlp/ffmpeg survivraient et verrouilleraient le
  temp. La purge de démarrage ne tourne que dans l'instance primaire (la 2ᵉ
  sort dans le `setup` du plugin single-instance, avant le nôtre).
- yt-dlp : toujours `--cache-dir` vers notre temp (sinon il écrit dans
  %APPDATA%).
- Les originaux ne sont jamais modifiés ; le lavage écrit une copie.

## Laver — architecture (phase 2)

- **Session exiftool persistante** (`laver.rs`). exiftool démarre en ~1 s
  (Perl) : on garde un process `-stay_open` derrière `Mutex<Option<ExifSession>>`.
  Chaque commande est encadrée par `{ready<seq>}` — sur stdout via `-execute<seq>`,
  sur stderr via `-echo4` — pour lire la sortie sans ambiguïté. Une erreur de
  tube détruit la session ; elle est reconstruite au prochain appel. Gain
  mesuré : 1er appel ~180 ms, suivants ~15 ms.
- **Copier-puis-nettoie** : le lavage ne touche jamais l'original. On
  `fs::copy` vers le fichier de sortie, on nettoie la copie avec
  `-overwrite_original`, puis on la re-scanne — le diff avant/après est la
  preuve montrée à l'utilisateur.
- **Recettes exiftool** (validées sur corpus, cf. pièges TODO 1-3, 14) :
  - tout : `-all=` + effacement nommé de l'identité et des dates (cf. piège 14
    TIFF) + `-tagsFromFile @ -Orientation -ICC_Profile` (sans ces deux derniers :
    photos couchées + couleurs délavées). Les listes `IDENTITY_STRIP` /
    `DATE_STRIP` de `laver.rs` portent les tags effacés nommément.
  - GPS seul : `-gps:all= -xmp:gpslatitude= -xmp:gpslongitude= -xmp:gpsaltitude=`.
  - garder la date : idem « tout » sans l'effacement des dates, + recopie
    DateTimeOriginal/CreateDate/ModifyDate.
- **Le classifieur (`classify.ts`) est la source unique** de « ce qui compte
  comme sensible ». Inspection ET vérification post-lavage l'utilisent, donc
  les compteurs avant/après sont cohérents. Rust reste mécanique (lance
  exiftool, renvoie les tags bruts) ; toute la présentation est côté TS.
- **Aperçus** via protocole asset (`convertFileSrc`, scope `**`) ; le webview
  ne rend pas HEIC/TIFF → pastille de format. Vignette EXIF renvoyée en
  base64 (encodeur maison, pas de dépendance).
- **HEIC non validé en écriture** : autorisé au lavage mais aucun vrai
  fichier testé — à faire au corpus phase 7.

## Récupérer — architecture (phase 4)

- **Le brut ne sort jamais du temp.** yt-dlp écrit dans un sous-dossier unique
  `temp_root()/dl/<job>` ; si « laver » est coché (défaut), on lave *dans* le
  temp (réutilise `laver::wash_download`) ; seule la copie propre est déplacée
  vers la destination, puis le sous-dossier est supprimé. `wash_download`
  route par extension : vidéo **et** audio par remux ffmpeg (mêmes flux à
  copier, mêmes métadonnées de conteneur à retirer — un `.m4a` téléchargé porte
  `Encoder: Lavf…` et un handler `Metadata` que le remux efface), image par
  exiftool, type inconnu → aucun lavage (on ne fait jamais échouer un
  téléchargement sur un format non reconnu).
- **Progression** : `--progress-template` avec un préfixe maison (`LAVOIR\t…`
  pour le téléchargement, `LAVOIR_PP\t…` pour le post-traitement). Un worker par
  job lit stdout **et** stderr fusionnés (la ligne de progression peut sortir
  sur l'un ou l'autre) et pousse des `DownloadEvent` par un `Channel` Tauri
  (pas d'événement global : pas besoin de permission event). Les lignes
  non préfixées alimentent un buffer des 80 dernières, pour humaniser une
  erreur.
- **File & annulation** : sémaphore maison (Mutex+Condvar, 2 places) rendu par
  un garde RAII ; chaque job garde un `AtomicBool` (annulé) et un `AtomicU32`
  (PID). `cancel_download` lève le flag et tue l'arbre (`taskkill /PID <pid> /T
  /F`) — indispensable car yt-dlp lance ffmpeg en enfant. Un job encore en file
  (PID = 0) est stoppé avant spawn.
- **`--ffmpeg-location`** reçoit le *dossier* des sidecars via
  `tool_path("ffmpeg").parent()` : en dev comme en prod, ffmpeg **et** ffprobe
  y sont sans suffixe (Tauri les copie ainsi). Piège vérifié : pointer sur
  `binaries/` (noms suffixés) casse l'extraction audio — toujours passer par
  `tool_path`, jamais `binaries/` en dur.
- **No-trace** : `--cache-dir` et les fichiers en cours vivent sous
  `temp_root()` (= `%TEMP%/lavoir`, futur périmètre du janitor phase 5) ;
  `--no-mtime` ; aucune URL persistée ; la liste des téléchargements est en
  mémoire de la page (disparaît au changement de vue et à la fermeture).
- **Réglages** persistés dans `settings.rs` (`%APPDATA%/…/settings.json`, que
  des préférences) : destination, `--cookies-from-browser` (Firefox fiable sous
  Windows, Chrome/Edge app-bound), et l'opt-in « vérifier yt-dlp au démarrage »
  (off ; `spawn_launch_check` range le résultat pour la page Réglages).
- **Frontend** : `recuperer.ts` crée le `Channel`, le passe à `start_download`
  et renvoie l'id (pour annuler). Le job est un objet `$state` (proxy Svelte 5
  profond) muté au fil des événements ; on capture `jobs[0]` (le proxy) après
  insertion pour que les mutations soient réactives.

## Design (arrêté en phase 6)

- Références : Raycast / Linear. Sombre, dense, clavier d'abord.
- Jetons dans `app.css` (`@theme`) : une rampe de **gris froids** (`bg` →
  `surface` → `raised` → `edge` → `faint` → `dim` → `text`) et **deux** couleurs
  porteuses seulement — `--color-accent` (teal aqua `#45c8b2` : l'eau, le propre,
  l'action) et `--color-danger` (`#e2665f` : fuite, erreur). **Ne jamais
  introduire d'autre teinte.** `faint` = texte tertiaire (indices, notes) ;
  `dim` = secondaire ; `text` = primaire.
- **Échelle de texte : 11 / 12 / 13 / 15 px, et rien d'autre** (le 19 px n'est
  admis que pour un titre d'état vide). Graisses 400 / 500 / 600. Titres en
  `tracking-tight`, micro-libellés en `uppercase tracking-wide`.
- Motion seulement quand elle porte du sens : `transition-colors` sur les
  survols, `transition-[width]` sur les barres, barre indéterminée animée quand
  la taille est inconnue. Jamais de `transition-all` ni de skeleton réflexe.
- Raccourcis clavier : Ctrl+V (coller une URL, Récupérer), Ctrl+O (ouvrir des
  fichiers, Laver), Échap (effacer/annuler). Indices `<kbd>` discrets.
- Icône : `scripts/gen-icon.ps1` (GDI+, sans outillage externe) écrit
  `assets/icon-source.png` — piste B du projet design (« goutte + onde ») :
  goutte teal relevée (dégradé + reflet) au-dessus de deux arcs teal qui rident
  la surface, sur carré arrondi quasi-noir. Coordonnées reprises du SVG `icoB`.
  L'onde s'efface aux petites tailles, la goutte reste lisible à 16 px.
  `npm run tauri -- icon assets/icon-source.png` régénère le jeu.
- UI en français, verbes précis (« Laver », « Récupérer »), jamais d'emojis.
  Signature du produit : « Tout ressort propre. Rien ne reste. »

## Style de travail

- Petits commits atomiques, messages conventionnels en anglais
  (`feat:`, `fix:`, `chore:`…).
- Tester dans `npm run tauri dev` avant de considérer une chose finie ;
  un type-check qui passe n'est pas une preuve de fonctionnement.
- Cocher TODO.md au fil de l'avancement ; mettre à jour ce fichier quand
  l'architecture bouge.
