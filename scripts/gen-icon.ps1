# Génère le PNG source 1024x1024 de l'icône lavoir, sans outillage externe
# (System.Drawing / GDI+). Motif « goutte + onde » (piste B du projet design) :
# la goutte tombée, la surface qui bouge — deux arcs teal sous la panse. Sur un
# carré arrondi quasi-noir, pensée pour rester lisible à 16 px (taskbar) où seule
# la goutte subsiste, l'onde s'effaçant à petite taille comme dans les maquettes.
# Coordonnées reprises fidèlement du SVG « icoB » de Lavoir - Design.html.
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

# Fond : carré arrondi aux couleurs des jetons de design finalisés (app.css).
$bg = [System.Drawing.ColorTranslator]::FromHtml('#0d0f13')
$stroke = [System.Drawing.ColorTranslator]::FromHtml('#232833')
$radius = 200
$d = $radius * 2
$bgPath = New-Object System.Drawing.Drawing2D.GraphicsPath
$bgPath.AddArc(0, 0, $d, $d, 180, 90)
$bgPath.AddArc(($size - $d), 0, $d, $d, 270, 90)
$bgPath.AddArc(($size - $d), ($size - $d), $d, $d, 0, 90)
$bgPath.AddArc(0, ($size - $d), $d, $d, 90, 90)
$bgPath.CloseFigure()
$g.FillPath((New-Object System.Drawing.SolidBrush $bg), $bgPath)
$g.DrawPath((New-Object System.Drawing.Pen $stroke, 3), $bgPath)

# Onde arrière : deux arcs teal sous la panse, la surface qui se ride après la
# chute. Tracés avant la goutte (donc derrière), clippés au socle pour ne jamais
# déborder des coins. Béziers quadratiques du SVG convertis en cubiques.
$accent = [System.Drawing.ColorTranslator]::FromHtml('#45c8b2')
$g.SetClip($bgPath)
$onde1 = New-Object System.Drawing.Drawing2D.GraphicsPath
$onde1.AddBezier(190, 830, 404.67, 756.67, 619.33, 756.67, 834, 830)
$onde2 = New-Object System.Drawing.Drawing2D.GraphicsPath
$onde2.AddBezier(100, 900, 374.67, 820, 649.33, 820, 924, 900)
foreach ($ride in @(@($onde1, 71), @($onde2, 36))) {
  $pen = New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb($ride[1], $accent.R, $accent.G, $accent.B)), 6
  $pen.StartCap = [System.Drawing.Drawing2D.LineCap]::Round
  $pen.EndCap = [System.Drawing.Drawing2D.LineCap]::Round
  $g.DrawPath($pen, $ride[0])
}
$g.ResetClip()

# Goutte relevée : pointe en cusp en haut, panse circulaire en bas. Deux béziers
# symétriques (tangente verticale au point le plus large) + demi-cercle bas.
$cx = 512
$tipY = 170
$cy = 558
$r = 210

$drop = New-Object System.Drawing.Drawing2D.GraphicsPath
$drop.AddBezier($cx, $tipY, 468, 268, ($cx - $r), 393, ($cx - $r), $cy)
$drop.AddArc(($cx - $r), ($cy - $r), ($r * 2), ($r * 2), 180, -180)
$drop.AddBezier(($cx + $r), $cy, ($cx + $r), 393, 556, 268, $cx, $tipY)
$drop.CloseFigure()

# Remplissage : dégradé vertical discret (lumière en haut), pas un arc-en-ciel.
$top = [System.Drawing.ColorTranslator]::FromHtml('#5fd6c2')
$bot = [System.Drawing.ColorTranslator]::FromHtml('#3bb8a4')
$p1 = New-Object System.Drawing.PointF $cx, $tipY
$p2 = New-Object System.Drawing.PointF $cx, 770
$fill = New-Object System.Drawing.Drawing2D.LinearGradientBrush $p1, $p2, $top, $bot
$g.FillPath($fill, $drop)

# Reflet spéculaire léger en haut à gauche de la panse (goutte d'eau).
$g.SetClip($drop)
$hi = New-Object System.Drawing.Drawing2D.GraphicsPath
$hi.AddEllipse(402, 310, 116, 168)
$g.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(36, 255, 255, 255))), $hi)
$g.ResetClip()

$g.Dispose()
$bmp.Save($outPath, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()

Write-Host "Icone ecrite : $outPath ($([Math]::Round((Get-Item $outPath).Length / 1KB, 1)) KB)"
