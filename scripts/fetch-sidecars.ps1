# Récupère les moteurs embarqués de lavoir — versions épinglées, sommes SHA256
# vérifiées contre les valeurs publiées par chaque projet — et les installe dans
# src-tauri/binaries/ au format sidecar Tauri (suffixe target-triple).
#
# À relancer :
#   - sur un clone frais (les binaires ne sont pas commités),
#   - après un `cargo clean` (resynchronise exiftool_files vers target/),
#   - pour monter une version : mettre à jour $pins (version + sha256).
#
# Les téléchargements sont mis en cache dans %TEMP%\lavoir-sidecars ; la somme
# est revérifiée à chaque exécution.

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

$triple = "x86_64-pc-windows-msvc"
$root = Split-Path $PSScriptRoot -Parent
$binDir = Join-Path $root "src-tauri\binaries"
$work = Join-Path $env:TEMP "lavoir-sidecars"

$pins = [ordered]@{
    "yt-dlp" = @{
        Version = "2026.07.04"
        Url     = "https://github.com/yt-dlp/yt-dlp/releases/download/2026.07.04/yt-dlp.exe"
        Sha256  = "52fe3c26dcf71fbdc85b528589020bb0b8e383155cfa81b64dd447bbe35e24b8"
        File    = "yt-dlp.exe"
    }
    "ffmpeg" = @{
        Version = "8.1.2"
        Url     = "https://www.gyan.dev/ffmpeg/builds/packages/ffmpeg-8.1.2-essentials_build.zip"
        Sha256  = "db580001caa24ac104c8cb856cd113a87b0a443f7bdf47d8c12b1d740584a2ec"
        File    = "ffmpeg-essentials.zip"
    }
    "exiftool" = @{
        Version = "13.59"
        # Hébergé sur SourceForge — passer par downloads.sourceforge.net :
        # l'endpoint /download sert une page HTML hors navigateur. La somme
        # vient de https://exiftool.org/checksums.txt et fait foi.
        Url     = "https://downloads.sourceforge.net/project/exiftool/exiftool-13.59_64.zip"
        Sha256  = "44b512b25af500724ba579d0a53c8fc5851628b692dd5e5d94ae4a15c2cba9ec"
        File    = "exiftool.zip"
    }
}

New-Item -ItemType Directory -Force $binDir | Out-Null
New-Item -ItemType Directory -Force $work | Out-Null

function Get-Pinned([string]$Name) {
    $pin = $pins[$Name]
    $dest = Join-Path $work $pin.File
    if (-not (Test-Path $dest)) {
        Write-Host "Téléchargement de $Name $($pin.Version)..."
        curl.exe -fsSL --retry 2 $pin.Url -o $dest
        if ($LASTEXITCODE -ne 0) { throw "échec du téléchargement de $Name (curl $LASTEXITCODE)" }
    }
    $actual = (Get-FileHash $dest -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $pin.Sha256) {
        Remove-Item $dest -Force
        throw "Somme SHA256 inattendue pour $Name : $actual (attendu $($pin.Sha256)). Fichier supprimé, relancer le script."
    }
    Write-Host "$Name $($pin.Version) : somme vérifiée."
    $dest
}

# --- yt-dlp : exe unique, copie directe -------------------------------------
$ytdlp = Get-Pinned "yt-dlp"
Copy-Item $ytdlp (Join-Path $binDir "yt-dlp-$triple.exe") -Force

# --- ffmpeg : zip gyan essentials, on extrait ffmpeg + ffprobe ---------------
$ffzip = Get-Pinned "ffmpeg"
$ffdir = Join-Path $work "ffmpeg-extract"
if (Test-Path $ffdir) { Remove-Item $ffdir -Recurse -Force }
Expand-Archive $ffzip $ffdir
foreach ($tool in "ffmpeg", "ffprobe") {
    $exe = Get-ChildItem $ffdir -Recurse -Filter "$tool.exe" | Select-Object -First 1
    if (-not $exe) { throw "$tool.exe introuvable dans l'archive gyan" }
    Copy-Item $exe.FullName (Join-Path $binDir "$tool-$triple.exe") -Force
}

# --- exiftool : launcher + dossier exiftool_files (distribution Perl) --------
$exzip = Get-Pinned "exiftool"
$exdir = Join-Path $work "exiftool-extract"
if (Test-Path $exdir) { Remove-Item $exdir -Recurse -Force }
Expand-Archive $exzip $exdir
$launcher = Get-ChildItem $exdir -Recurse -Filter "exiftool*.exe" | Select-Object -First 1
if (-not $launcher) { throw "launcher exiftool introuvable dans l'archive" }
# Le nom du launcher pilote ses options (« exiftool(-k).exe » = pause finale) :
# le renommer au format sidecar supprime aussi ce comportement.
Copy-Item $launcher.FullName (Join-Path $binDir "exiftool-$triple.exe") -Force
$filesDir = Get-ChildItem $exdir -Recurse -Directory -Filter "exiftool_files" | Select-Object -First 1
if (-not $filesDir) { throw "dossier exiftool_files introuvable dans l'archive" }
$binFiles = Join-Path $binDir "exiftool_files"
if (Test-Path $binFiles) { Remove-Item $binFiles -Recurse -Force }
Copy-Item $filesDir.FullName $binFiles -Recurse

# --- Sync dev : le launcher copié dans target/ cherche exiftool_files à côté
# de lui ; Tauri copie les sidecars en dev mais pas les ressources. ------------
foreach ($profile in "debug", "release") {
    $targetDir = Join-Path $root "src-tauri\target\$profile"
    New-Item -ItemType Directory -Force $targetDir | Out-Null
    $sync = Join-Path $targetDir "exiftool_files"
    if (Test-Path $sync) { Remove-Item $sync -Recurse -Force }
    Copy-Item $binFiles $sync -Recurse
}

Write-Host ""
Write-Host "Moteurs installés dans src-tauri\binaries :"
Get-ChildItem $binDir -File | ForEach-Object {
    Write-Host ("  {0,-42} {1,8:N1} Mo" -f $_.Name, ($_.Length / 1MB))
}
