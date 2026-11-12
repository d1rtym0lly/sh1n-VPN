param([switch]$CoverOnly)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$scratch = Join-Path $root '.media-work'
New-Item -ItemType Directory -Force $scratch | Out-Null
Copy-Item -LiteralPath "$env:WINDIR\Fonts\segoeuib.ttf" -Destination (Join-Path $scratch 'bold.ttf') -Force
Copy-Item -LiteralPath "$env:WINDIR\Fonts\segoeui.ttf" -Destination (Join-Path $scratch 'regular.ttf') -Force
$ink = '0xf1f8f7'
$mint = '0x86ebd4'
$sub = '0xaebfbd'

function Title([string]$id,[string]$text,[string]$font,[int]$size,[string]$color,[int]$x,[int]$y) {
    [IO.File]::WriteAllText((Join-Path $scratch "$id.txt"), $text, [Text.UTF8Encoding]::new($false))
    "drawtext=fontfile=.media-work/$font.ttf:textfile=.media-work/$id.txt:fontsize=$size" +
    ":fontcolor=$color" + ":x=$x" + ":y=$y" + ':expansion=none'
}
function Encode {
    & ffmpeg -hide_banner -loglevel error -y @args
    if ($LASTEXITCODE) { throw "ffmpeg failed: $LASTEXITCODE" }
}
Push-Location $root
try {
    $cover = @(
        'scale=1280:720:flags=lanczos',
        'drawbox=x=77:y=186:w=39:h=4:color=0x86ebd4:t=fill',
        (Title 'cover-tag' 'BETA 01  /  WINDOWS x64' 'bold' 19 $mint 132 175),
        (Title 'cover-name' 'sh1n VPN' 'bold' 107 $ink 73 225),
        (Title 'cover-line' 'Свои серверы. Один клиент.' 'regular' 34 $sub 81 359),
        (Title 'cover-foot' 'RUST   /   XRAY CORE' 'bold' 17 $mint 83 585)
    ) -join ','
    Encode -i 'media/hero-background.jpg' -vf $cover '-frames:v' 1 '-q:v' 2 'media/cover.jpg'
    if ($CoverOnly) { return }

    $rate = 30
    $encodeArgs = @('-c:v','libx264','-preset','fast','-crf','19','-pix_fmt','yuv420p','-r',"$rate",'-an')
    $fade = 'fade=t=in:st=0:d=0.34,fade=t=out:st=3.7:d=0.3'
    Encode -loop 1 -framerate $rate -i 'media/cover.jpg' -t 4 -vf $fade @encodeArgs '.media-work/part-0.mp4'
    $slides = @(
        @{ image='app-home'; label='01   /   ПОДКЛЮЧЕНИЕ'; head='Выберите'; line='режим'; sub='Прокси или VPN / TUN' },
        @{ image='app-transfer'; label='02   /   ПЕРЕНОС'; head='Импорт'; line='и экспорт'; sub='Happ, v2rayN, WireGuard' },
        @{ image='app-themes'; label='03   /   ТЕМЫ'; head='Десять'; line='палитр'; sub='Светлые и тёмные' }
    )
    for ($i=0; $i -lt $slides.Count; $i++) {
        $slide = $slides[$i]
        $id = $i+1
        $titles = @(
            (Title "tag-$id" $slide.label 'bold' 18 $mint 62 187),
            (Title "head-$id" $slide.head 'bold' 54 $ink 60 243),
            (Title "line-$id" $slide.line 'bold' 54 $ink 60 302),
            (Title "sub-$id" $slide.sub 'regular' 23 $sub 62 395),
            (Title 'slide-brand' 'sh1n VPN' 'bold' 17 $mint 62 612)
        ) -join ','
        $filter = "[1:v]scale=845:-2:flags=lanczos,format=yuv420p[screen];[0:v][screen]overlay=x=405:y=(H-h)/2,$titles,$fade"
        Encode -f lavfi -i "color=c=0x10171c:s=1280x720:r=$rate" -loop 1 -framerate $rate -i "media/$($slide.image).png" -filter_complex_threads 2 -filter_complex $filter -t 4 @encodeArgs ".media-work/part-$id.mp4"
    }
    $end = @(
        'scale=1280:720:flags=lanczos',
        (Title 'end-tag' 'ПЕРВАЯ ПУБЛИЧНАЯ БЕТА' 'bold' 18 $mint 82 188),
        (Title 'end-name' 'sh1n VPN' 'bold' 94 $ink 75 234),
        (Title 'end-link' 'Скачать на GitHub' 'regular' 33 $sub 82 357),
        (Title 'end-platform' 'ПОРТАТИВНО   /   WINDOWS x64' 'bold' 17 $mint 82 585),
        $fade
    ) -join ','
    Encode -loop 1 -framerate $rate -i 'media/hero-background.jpg' -vf $end -t 4 @encodeArgs '.media-work/part-4.mp4'
    [IO.File]::WriteAllLines((Join-Path $scratch 'concat.txt'), [string[]](0..4 | ForEach-Object { "file 'part-$_.mp4'" }))
    # Original synthesized ambience; no external music or samples.
    $tone='aevalsrc=0.024*sin(2*PI*110*t)*(0.55+0.45*cos(2*PI*0.24*t))+0.018*sin(2*PI*164.81*t)+0.013*sin(2*PI*220*t)*(0.5+0.5*sin(2*PI*0.37*t)):s=48000:d=20'
    New-Item -ItemType Directory -Force 'dist/beta' | Out-Null
    Encode -f concat -safe 0 -i '.media-work/concat.txt' -f lavfi -i $tone -map 0:v -map 1:a -af 'afade=t=in:d=1,afade=t=out:st=18:d=2' '-c:v' copy '-c:a' aac '-b:a' 96k -movflags +faststart -shortest 'dist/beta/sh1n-VPN-beta-preview.mp4'
} finally {
    Pop-Location
}
