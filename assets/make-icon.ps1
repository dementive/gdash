# Draws gdash's app icon (the green orb from the top bar) and writes assets\gdash.ico with
# PNG frames at the sizes Windows asks for. Run once; the .ico is committed and embedded by
# build.rs.
Add-Type -AssemblyName System.Drawing

function New-Orb([int]$size) {
    $bmp = New-Object System.Drawing.Bitmap $size, $size, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)
    $m = [Math]::Max(1.0, $size * 0.06)
    $r = New-Object System.Drawing.RectangleF $m, $m, ($size - 2 * $m), ($size - 2 * $m)

    # Dark rim so the orb reads on light backgrounds too.
    $rim = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 18, 52, 6))
    $g.FillEllipse($rim, $r)

    # Body: radial gradient lit from the upper left (same colours as the UI orb).
    $inset = [Math]::Max(1.0, $size * 0.025)
    $b = New-Object System.Drawing.RectangleF ($r.X + $inset), ($r.Y + $inset), ($r.Width - 2 * $inset), ($r.Height - 2 * $inset)
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $path.AddEllipse($b)
    $pgb = New-Object System.Drawing.Drawing2D.PathGradientBrush $path
    $pgb.CenterPoint = New-Object System.Drawing.PointF ($b.X + $b.Width * 0.36), ($b.Y + $b.Height * 0.32)
    $blend = New-Object System.Drawing.Drawing2D.ColorBlend 3
    $blend.Colors = @(
        [System.Drawing.Color]::FromArgb(255, 41, 117, 10),
        [System.Drawing.Color]::FromArgb(255, 120, 219, 41),
        [System.Drawing.Color]::FromArgb(255, 224, 255, 184)
    )
    $blend.Positions = @(0.0, 0.6, 1.0)
    $pgb.InterpolationColors = $blend
    $g.FillPath($pgb, $path)

    # Glossy highlight.
    if ($size -ge 24) {
        $hw = $b.Width * 0.46; $hh = $b.Height * 0.26
        $hr = New-Object System.Drawing.RectangleF ($b.X + $b.Width * 0.2), ($b.Y + $b.Height * 0.1), $hw, $hh
        $hl = New-Object System.Drawing.Drawing2D.LinearGradientBrush $hr, ([System.Drawing.Color]::FromArgb(150, 255, 255, 255)), ([System.Drawing.Color]::FromArgb(0, 255, 255, 255)), 90.0
        $g.FillEllipse($hl, $hr)
    }
    $g.Dispose()
    return $bmp
}

$sizes = 16, 20, 24, 32, 40, 48, 64, 256
$frames = foreach ($s in $sizes) {
    $bmp = New-Orb $s
    $ms = New-Object System.IO.MemoryStream
    $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    , $ms.ToArray()
}

$out = Join-Path $PSScriptRoot 'gdash.ico'
$fs = [System.IO.File]::Create($out)
$w = New-Object System.IO.BinaryWriter $fs
$w.Write([UInt16]0); $w.Write([UInt16]1); $w.Write([UInt16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
for ($i = 0; $i -lt $sizes.Count; $i++) {
    $s = $sizes[$i]
    $dim = if ($s -ge 256) { 0 } else { $s }
    $w.Write([byte]$dim); $w.Write([byte]$dim); $w.Write([byte]0); $w.Write([byte]0)
    $w.Write([UInt16]1); $w.Write([UInt16]32)
    $w.Write([UInt32]$frames[$i].Length); $w.Write([UInt32]$offset)
    $offset += $frames[$i].Length
}
foreach ($f in $frames) { $w.Write($f) }
$w.Close()
"wrote $out"
