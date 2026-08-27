# Draws the app icon and writes it as a multi-size .ico.
#
# Kept next to the icon so the asset is reproducible rather than a binary blob
# nobody can regenerate. Run it from anywhere:
#
#   powershell -File crates/perch/assets/make-icon.ps1
#
# The shape is deliberately blunt, because at 16px in a taskbar there is room
# for exactly one idea: a bird sitting on a bar, cream on teal.
#
# It draws the *name* rather than the product, which is the right way round here.
# "Perch" tells nobody what the app is, so the icon is the only place the name
# gets explained — and it has to be a silhouette you would not confuse with the
# rest of a taskbar, which a rounded screen with a play triangle in it, the most
# common icon on any machine, very much is. That is what this used to be.
#
# The teal is a deliberate move off the old purple, which sat a few degrees from
# Twitch's own. Fair enough while the app was called nativetwitch; a trademark
# question now that it is not.
#
# Three details in the drawing are load-bearing at small sizes, and all three
# came from rendering the thing and looking at it rather than reasoning about it:
#
#   - **The dip between crown and back.** That notch is the single cue that says
#     "bird" rather than "blob". An earlier draft had a wing swept over the body
#     and no head at all, and it read as a shark fin.
#   - **The tail is short and fat, not long and fine.** A tapered point looks
#     better at 256 and antialiases to nothing at 16. Blunting the tip instead
#     was worse again: the flat left a notch that reads as damage.
#   - **No eye, no beak line, no feet.** Each one survives to 256px and turns to
#     grey mud at 16.

Add-Type -AssemblyName System.Drawing

$teal = [System.Drawing.Color]::FromArgb(255, 0x17, 0x60, 0x5c)
$cream = [System.Drawing.Color]::FromArgb(255, 0xf2, 0xef, 0xe6)
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

# A perching bird facing left, as one closed outline on the 256 grid.
function New-BirdPath {
    $b = New-Object System.Drawing.Drawing2D.GraphicsPath
    $b.AddBezier(68, 120, 82, 110, 88, 102, 104, 96)     # beak tip -> front of crown
    $b.AddBezier(104, 96, 120, 90, 132, 96, 138, 110)    # crown -> the neck dip
    $b.AddBezier(138, 110, 150, 112, 162, 116, 172, 120) # nape -> back
    $b.AddLine(172, 120, 206, 80)                        # back -> tail tip
    $b.AddLine(206, 80, 192, 158)                        # tail underside -> rump
    $b.AddBezier(192, 158, 186, 168, 174, 174, 160, 174) # rump -> belly on the bar
    $b.AddLine(160, 174, 116, 174)                       # belly along the bar
    $b.AddBezier(116, 174, 100, 168, 88, 150, 86, 132)   # breast rising
    $b.AddBezier(86, 132, 84, 126, 76, 122, 68, 120)     # throat -> back to the beak
    $b.CloseFigure()
    return $b
}

# One master image. `boost` scales the mark about the tile's centre and thickens
# the bar to match; see the size table below for why.
function New-Master([double]$boost) {
    $bmp = New-Object System.Drawing.Bitmap $master, $master
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)

    # A little inset, so the icon does not touch the edges of its cell.
    $body = New-RoundedPath 16 16 224 224 56
    $tealBrush = New-Object System.Drawing.SolidBrush $teal
    $g.FillPath($tealBrush, $body)

    # The mark is drawn in cream rather than knocked out to transparent, which is
    # what the play triangle used to do. A knockout shows whatever is behind the
    # icon, and a bird whose colour is the taskbar's is a bird that disappears
    # against half of them.
    $mark = New-Object System.Drawing.SolidBrush $cream

    $barH = if ($boost -gt 1.0) { 20.0 } else { 16.0 }
    $barY = if ($boost -gt 1.0) { 174.0 } else { 176.0 }

    $mtx = New-Object System.Drawing.Drawing2D.Matrix
    $mtx.Translate(128, 132)
    $mtx.Scale($boost, $boost)
    $mtx.Translate(-128, -132)

    # The perch: wide enough to read as a branch rather than as an underline.
    $bar = New-RoundedPath 56 $barY 144 $barH ($barH / 2)
    $bar.Transform($mtx)
    $g.FillPath($mark, $bar)

    $bird = New-BirdPath
    $bird.Transform($mtx)
    $g.FillPath($mark, $bird)

    $g.Dispose(); $tealBrush.Dispose(); $mark.Dispose()
    $body.Dispose(); $bar.Dispose(); $bird.Dispose(); $mtx.Dispose()
    return $bmp
}

# Which master each size is drawn down from.
#
# Below about 32px the downscale eats the beak and the tail and leaves a pale
# smear on a line. Taking those sizes from a master whose mark is 16% larger
# puts enough covered pixels back that the head still reads. Hand-tuning the
# small end is what icon sets normally do; the alternative — drawing directly at
# 16px — is the mush this file used to warn about.
$standard = New-Master 1.0
$bold = New-Master 1.16
$plan = @(
    @{ Size = 16; Bmp = $bold },
    @{ Size = 24; Bmp = $bold },
    @{ Size = 32; Bmp = $standard },
    @{ Size = 48; Bmp = $standard },
    @{ Size = 64; Bmp = $standard },
    @{ Size = 128; Bmp = $standard },
    @{ Size = 256; Bmp = $standard }
)

# ── scale, encode, and assemble the .ico ────────────────────────────
# Entries up to 128px are uncompressed DIBs; 256 is a PNG. That split is what
# every icon tool does, and it matters here: GDI+ cannot read a PNG-payload
# entry back, so a PNG-only file is one nothing on this machine can open to
# check the result.
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

foreach ($entry in $plan) {
    $s = $entry.Size
    $scaled = New-Object System.Drawing.Bitmap $s, $s
    $sg = [System.Drawing.Graphics]::FromImage($scaled)
    $sg.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
    $sg.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
    $sg.Clear([System.Drawing.Color]::Transparent)
    $sg.DrawImage($entry.Bmp, (New-Object System.Drawing.Rectangle 0, 0, $s, $s))
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
$standard.Dispose(); $bold.Dispose()

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
$target = Join-Path $PSScriptRoot "perch.ico"
[System.IO.File]::WriteAllBytes($target, $ico.ToArray())
$w.Dispose(); $ico.Dispose()

Write-Output "wrote $target ($((Get-Item $target).Length) bytes, $($payloads.Count) sizes)"
