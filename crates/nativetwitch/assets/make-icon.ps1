# Draws the app icon and writes it as a multi-size .ico.
#
# Kept next to the icon so the asset is reproducible rather than a binary blob
# nobody can regenerate. Run it from anywhere:
#
#   powershell -File crates/nativetwitch/assets/make-icon.ps1
#
# The shape is deliberately blunt, because at 16px in a taskbar there is room
# for exactly one idea: a rounded screen in the app's accent purple with a play
# triangle knocked out of it. Everything is drawn once at 256px and scaled down
# with high-quality interpolation; drawing directly at 16px gives mush.

Add-Type -AssemblyName System.Drawing

$accent = [System.Drawing.Color]::FromArgb(255, 0x9d, 0x7b, 0xff)
$deep = [System.Drawing.Color]::FromArgb(255, 0x6f, 0x4a, 0xe0)
$master = 256

function New-RoundedPath([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $r * 2
    $path.AddArc($x, $y, $d, $d, 180, 90)
    $path.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $path.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $path.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $path.CloseFigure()
    return $path
}

# ── the master image ────────────────────────────────────────────────
$bmp = New-Object System.Drawing.Bitmap $master, $master
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$g.Clear([System.Drawing.Color]::Transparent)

# A little inset, so the icon does not touch the edges of its cell.
$inset = 16.0
$size = $master - ($inset * 2)
$body = New-RoundedPath $inset $inset $size $size 56

$brush = New-Object System.Drawing.Drawing2D.LinearGradientBrush(
    (New-Object System.Drawing.Point 0, 0),
    (New-Object System.Drawing.Point 0, $master),
    $accent, $deep)
$g.FillPath($brush, $body)

# The play triangle, knocked out rather than drawn on top, so it reads at any
# size and against any taskbar colour.
$cut = New-Object System.Drawing.Drawing2D.GraphicsPath
$cut.AddPolygon(@(
        (New-Object System.Drawing.PointF 104, 84),
        (New-Object System.Drawing.PointF 104, 172),
        (New-Object System.Drawing.PointF 180, 128)
    ))
$g.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
$g.FillPath((New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::Transparent)), $cut)

$g.Dispose(); $brush.Dispose(); $body.Dispose(); $cut.Dispose()
# ── scale, encode, and assemble the .ico ────────────────────────────
# Entries up to 128px are uncompressed DIBs; 256 is a PNG. That split is what
# every icon tool does, and it matters here: GDI+ cannot read a PNG-payload
# entry back, so a PNG-only file is one nothing on this machine can open to
# check the result.
$sizes = @(16, 32, 48, 64, 128, 256)
$payloads = @()

function Get-DibBytes([System.Drawing.Bitmap]$image) {
    $iw = $image.Width
    $ih = $image.Height
    $rect = New-Object System.Drawing.Rectangle 0, 0, $iw, $ih
    $data = $image.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly,
        [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $pixels = New-Object byte[] ($data.Stride * $ih)
    [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $pixels, 0, $pixels.Length)
    $stride = $data.Stride
    $image.UnlockBits($data)

    $ms = New-Object System.IO.MemoryStream
    $bw = New-Object System.IO.BinaryWriter $ms

    # BITMAPINFOHEADER. The height is doubled because the AND mask that follows
    # is notionally part of the same bitmap.
    $bw.Write([uint32]40)
    $bw.Write([int32]$iw)
    $bw.Write([int32]($ih * 2))
    $bw.Write([uint16]1)
    $bw.Write([uint16]32)
    $bw.Write([uint32]0)                    # BI_RGB
    $bw.Write([uint32]($iw * $ih * 4))
    $bw.Write([int32]0); $bw.Write([int32]0)
    $bw.Write([uint32]0); $bw.Write([uint32]0)

    # Colour data, bottom-up.
    for ($y = $ih - 1; $y -ge 0; $y--) {
        $bw.Write($pixels, $y * $stride, $iw * 4)
    }

    # AND mask: all zeros. The alpha channel already does this job, but the
    # field is not optional.
    $maskStride = [math]::Floor(($iw + 31) / 32) * 4
    $bw.Write((New-Object byte[] ($maskStride * $ih)))

    $bw.Flush()
    $bytes = $ms.ToArray()
    $bw.Dispose(); $ms.Dispose()
    return , $bytes
}

foreach ($s in $sizes) {
    $scaled = New-Object System.Drawing.Bitmap $s, $s
    $sg = [System.Drawing.Graphics]::FromImage($scaled)
    $sg.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $sg.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $sg.Clear([System.Drawing.Color]::Transparent)
    $sg.DrawImage($bmp, (New-Object System.Drawing.Rectangle 0, 0, $s, $s))
    $sg.Dispose()

    if ($s -ge 256) {
        $stream = New-Object System.IO.MemoryStream
        $scaled.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
        $bytes = $stream.ToArray()
        $stream.Dispose()
    } else {
        $bytes = Get-DibBytes $scaled
    }

    $payloads += , @{ Size = $s; Bytes = $bytes }
    $scaled.Dispose()
}
$bmp.Dispose()

$ico = New-Object System.IO.MemoryStream
$w = New-Object System.IO.BinaryWriter $ico

# ICONDIR
$w.Write([uint16]0)                  # reserved
$w.Write([uint16]1)                  # type: icon
$w.Write([uint16]$payloads.Count)

# ICONDIRENTRY per image. A dimension of 0 means 256.
$offset = 6 + (16 * $payloads.Count)
foreach ($p in $payloads) {
    $dim = if ($p.Size -ge 256) { 0 } else { $p.Size }
    $w.Write([byte]$dim)             # width
    $w.Write([byte]$dim)             # height
    $w.Write([byte]0)                # palette size
    $w.Write([byte]0)                # reserved
    $w.Write([uint16]1)              # colour planes
    $w.Write([uint16]32)             # bits per pixel
    $w.Write([uint32]$p.Bytes.Length)
    $w.Write([uint32]$offset)
    $offset += $p.Bytes.Length
}
foreach ($p in $payloads) { $w.Write($p.Bytes) }

$w.Flush()
$target = Join-Path $PSScriptRoot "nativetwitch.ico"
[System.IO.File]::WriteAllBytes($target, $ico.ToArray())
$w.Dispose(); $ico.Dispose()

Write-Output "wrote $target ($((Get-Item $target).Length) bytes, $($payloads.Count) sizes)"
