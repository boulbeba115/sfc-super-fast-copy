# Renders the Super Fast Copy icon (gradient tile with a lightning bolt) into
#   assets/superfastcopy.ico    multi-size icon for the exe, installer and Explorer menu
#   assets/icon-128.rgba   raw 128x128 RGBA pixels for the window icon and header logo
Add-Type -AssemblyName System.Drawing

$assets = Join-Path (Split-Path $PSScriptRoot -Parent) 'assets'
New-Item -ItemType Directory -Force $assets | Out-Null

function Render([int]$s) {
    $bmp = New-Object System.Drawing.Bitmap $s, $s, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.PixelOffsetMode = 'HighQuality'
    $g.Clear([System.Drawing.Color]::Transparent)

    # Rounded tile with a diagonal purple -> cyan gradient.
    $m = [math]::Max(1, $s * 0.04); $w = $s - 2 * $m; $r = $w * 0.26
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.AddArc($m, $m, 2 * $r, 2 * $r, 180, 90)
    $path.AddArc($m + $w - 2 * $r, $m, 2 * $r, 2 * $r, 270, 90)
    $path.AddArc($m + $w - 2 * $r, $m + $w - 2 * $r, 2 * $r, 2 * $r, 0, 90)
    $path.AddArc($m, $m + $w - 2 * $r, 2 * $r, 2 * $r, 90, 90)
    $path.CloseFigure()
    $brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush (New-Object System.Drawing.PointF 0, 0), (New-Object System.Drawing.PointF $s, $s), ([System.Drawing.Color]::FromArgb(255, 124, 92, 255)), ([System.Drawing.Color]::FromArgb(255, 0, 198, 255))
    $g.FillPath($brush, $path)

    # Lightning bolt.
    $bolt = @(@(0.50, 0.08), @(0.70, 0.08), @(0.55, 0.40), @(0.76, 0.40), @(0.38, 0.94), @(0.47, 0.56), @(0.25, 0.56))
    $pt = { param($x, $y, $dy) New-Object System.Drawing.PointF ($s * (0.14 + $x * 0.72)), ($s * (0.14 + $y * 0.72) + $dy) }
    $shadow = [System.Drawing.PointF[]]($bolt | ForEach-Object { & $pt $_[0] $_[1] ($s * 0.025) })
    $g.FillPolygon((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(70, 20, 10, 60))), $shadow)
    $pts = [System.Drawing.PointF[]]($bolt | ForEach-Object { & $pt $_[0] $_[1] 0 })
    $g.FillPolygon((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(250, 255, 255, 255))), $pts)
    $g.Dispose()
    $bmp
}

# ICO with PNG-compressed entries.
$sizes = 16, 20, 24, 32, 40, 48, 64, 128, 256
$pngs = foreach ($s in $sizes) {
    $b = Render $s; $ms = New-Object System.IO.MemoryStream
    $b.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png); $b.Dispose()
    , $ms.ToArray()
}
$out = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter $out
$bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $d = if ($sizes[$i] -ge 256) { 0 } else { $sizes[$i] }
    $bw.Write([byte]$d); $bw.Write([byte]$d); $bw.Write([byte]0); $bw.Write([byte]0)
    $bw.Write([uint16]1); $bw.Write([uint16]32)
    $bw.Write([uint32]$pngs[$i].Length); $bw.Write([uint32]$offset)
    $offset += $pngs[$i].Length
}
foreach ($p in $pngs) { $bw.Write($p) }
[System.IO.File]::WriteAllBytes((Join-Path $assets 'superfastcopy.ico'), $out.ToArray())

# Raw RGBA (GDI+ stores BGRA).
$b = Render 128
$rect = New-Object System.Drawing.Rectangle 0, 0, 128, 128
$data = $b.LockBits($rect, 'ReadOnly', ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb))
$bytes = New-Object byte[] (128 * 128 * 4)
[System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $bytes, 0, $bytes.Length)
$b.UnlockBits($data); $b.Dispose()
for ($i = 0; $i -lt $bytes.Length; $i += 4) { $t = $bytes[$i]; $bytes[$i] = $bytes[$i + 2]; $bytes[$i + 2] = $t }
[System.IO.File]::WriteAllBytes((Join-Path $assets 'icon-128.rgba'), $bytes)

# Preview for humans.
$b = Render 256; $b.Save((Join-Path $assets 'icon-256.png'), [System.Drawing.Imaging.ImageFormat]::Png); $b.Dispose()
Get-ChildItem $assets | Select-Object Name, Length
