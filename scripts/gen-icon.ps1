# Génère le PNG source 1024x1024 de l'icône lavoir, sans outillage externe
# (System.Drawing). Placeholder de phase 1 : goutte accent sur carré sombre ;
# l'icône définitive est un livrable de la phase 6 (TODO.md).
# Ensuite : npm run tauri -- icon assets/icon-source.png

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing

$size = 1024
$assetsDir = Join-Path (Join-Path $PSScriptRoot "..") "assets"
New-Item -ItemType Directory -Force $assetsDir | Out-Null
$outPath = Join-Path $assetsDir "icon-source.png"

$bmp = New-Object System.Drawing.Bitmap $size, $size
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
$g.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality

# Fond : carré arrondi aux couleurs des jetons (--color-bg / --color-edge).
$bgColor = [System.Drawing.ColorTranslator]::FromHtml('#0e1013')
$borderColor = [System.Drawing.ColorTranslator]::FromHtml('#262b33')
$radius = 180
$diameter = $radius * 2
$bgPath = New-Object System.Drawing.Drawing2D.GraphicsPath
$bgPath.AddArc(0, 0, $diameter, $diameter, 180, 90)
$bgPath.AddArc(($size - $diameter), 0, $diameter, $diameter, 270, 90)
$bgPath.AddArc(($size - $diameter), ($size - $diameter), $diameter, $diameter, 0, 90)
$bgPath.AddArc(0, ($size - $diameter), $diameter, $diameter, 90, 90)
$bgPath.CloseFigure()

$g.FillPath((New-Object System.Drawing.SolidBrush $bgColor), $bgPath)
$g.DrawPath((New-Object System.Drawing.Pen $borderColor, 4), $bgPath)

# Goutte : pointe en haut, panse circulaire en bas (--color-accent).
$dropColor = [System.Drawing.ColorTranslator]::FromHtml('#4cc2b4')
$drop = New-Object System.Drawing.Drawing2D.GraphicsPath

$tipX = 512; $tipY = 218
$cx = 512; $cy = 622; $r = 208

$drop.AddBezier(
    $tipX, $tipY,
    512, 400,
    ($cx - $r), ($cy - 190),
    ($cx - $r), $cy
)
# Demi-cercle inférieur : gauche -> bas -> droite (sens anti-horaire en GDI+).
$drop.AddArc(($cx - $r), ($cy - $r), ($r * 2), ($r * 2), 180, -180)
$drop.AddBezier(
    ($cx + $r), $cy,
    ($cx + $r), ($cy - 190),
    512, 400,
    $tipX, $tipY
)
$drop.CloseFigure()

$g.FillPath((New-Object System.Drawing.SolidBrush $dropColor), $drop)

$g.Dispose()
$bmp.Save($outPath, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

Write-Host "Icone ecrite : $outPath ($([Math]::Round((Get-Item $outPath).Length / 1KB, 1)) KB)"
