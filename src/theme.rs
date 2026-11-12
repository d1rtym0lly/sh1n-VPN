pub struct Theme {
    pub name: &'static str,
    pub note: &'static str,
    pub bg: u32,
    pub rail: u32,
    pub card: u32,
    pub panel: u32,
    pub line: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub second: u32,
    pub ink: u32,
}

const fn color(c: u32) -> u32 {
    (c & 255) << 16 | (c & 65280) | (c >> 16)
}
macro_rules! theme {
    ($name:expr,$note:expr,$bg:expr,$rail:expr,$card:expr,$panel:expr,$line:expr,$text:expr,$muted:expr,$accent:expr,$second:expr,$ink:expr) => {
        Theme {
            name: $name,
            note: $note,
            bg: color($bg),
            rail: color($rail),
            card: color($card),
            panel: color($panel),
            line: color($line),
            text: color($text),
            muted: color($muted),
            accent: color($accent),
            second: color($second),
            ink: color($ink),
        }
    };
}

pub static THEMES: [Theme; 10] = [
    theme!(
        "Signal / Mint",
        "Графит и мятный акцент",
        0x11171b,
        0x0c1115,
        0x192226,
        0x202e33,
        0x334349,
        0xf1f8f7,
        0xaebfbd,
        0x86ebd4,
        0x4dc1b3,
        0x102927
    ),
    theme!(
        "Catppuccin / Mocha",
        "Сирень и тёплые пастельные тона",
        0x1e1e2e,
        0x181825,
        0x29293d,
        0x313244,
        0x45475a,
        0xcdd6f4,
        0xbac2de,
        0xcba6f7,
        0xf5c2e7,
        0x241635
    ),
    theme!(
        "Catppuccin / Latte",
        "Светлая тема с лавандой",
        0xeff1f5,
        0xe6e9ef,
        0xffffff,
        0xe6e9ef,
        0xc5cbd8,
        0x30374a,
        0x50586e,
        0x722ccb,
        0x1e66f5,
        0xffffff
    ),
    theme!(
        "Dribbblish / Nord",
        "Холодный синий и полярная ночь",
        0x252b36,
        0x20252f,
        0x303948,
        0x384354,
        0x4c596c,
        0xeceff4,
        0xb9c9d9,
        0x88c0d0,
        0x81a1c1,
        0x182634
    ),
    theme!(
        "Dribbblish / Dracula",
        "Фиолетовый неон",
        0x242631,
        0x1e2029,
        0x303342,
        0x393d50,
        0x4b5066,
        0xf8f8f2,
        0xc5bdd9,
        0xbd93f9,
        0xff79c6,
        0x26173b
    ),
    theme!(
        "Dribbblish / Gruvbox",
        "Графит, охра и песочный",
        0x282828,
        0x1d2021,
        0x34312e,
        0x3c3836,
        0x51463c,
        0xfbf1c7,
        0xd0bfa0,
        0xfabd2f,
        0xb8bb26,
        0x322510
    ),
    theme!(
        "Dribbblish / Sunset",
        "Коралловый на вечернем фоне",
        0x251b29,
        0x1b1420,
        0x382635,
        0x463044,
        0x644456,
        0xffedf3,
        0xe1b7c5,
        0xffab8d,
        0xe988a3,
        0x381c20
    ),
    theme!(
        "Sleek / Ocean",
        "Морская глубина и бирюза",
        0x12202e,
        0x0c1721,
        0x182c3e,
        0x20394e,
        0x345365,
        0xecf8ff,
        0xa9c4d8,
        0x72d9fa,
        0x52bdb1,
        0x102837
    ),
    theme!(
        "Onepunch / Rose",
        "Тёмный графит с розовым",
        0x211a24,
        0x18131b,
        0x302437,
        0x3d2d45,
        0x5c4263,
        0xf8edf8,
        0xd7bdd9,
        0xf99eca,
        0xd3a8ff,
        0x351828
    ),
    theme!(
        "Matte / Ivory",
        "Матовая светлая палитра",
        0xf3efe6,
        0xe8e1d4,
        0xfffbf3,
        0xede5d8,
        0xd0c4b4,
        0x2d2924,
        0x595047,
        0x805028,
        0x926539,
        0xffffff
    ),
];
