use crate::{
    core::{Core, Mode},
    profile,
    store::{self, Store},
    subscription,
    theme::{Theme, THEMES},
};
use std::{
    cell::RefCell,
    fs::File,
    io::{Read, Write},
    mem::size_of,
    os::windows::ffi::OsStringExt,
    ptr::{null, null_mut},
    sync::mpsc,
    thread,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::DwmSetWindowAttribute, Gdi::*},
    System::{
        DataExchange::*, LibraryLoader::GetModuleHandleW, Memory::*, Threading::CreateMutexW,
    },
    UI::{
        Controls::Dialogs::*,
        Controls::*,
        HiDpi::*,
        Input::KeyboardAndMouse::{GetKeyState, SetFocus, VK_CONTROL},
        Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP},
        WindowsAndMessaging::*,
    },
};

const HOME: u16 = 100;
const THEMES_NAV: u16 = 101;
const CONNECT: u16 = 102;
const PROXY: u16 = 103;
const TUN: u16 = 104;
const CLIP: u16 = 105;
const FILE: u16 = 106;
const TRANSFER: u16 = 107;
const REMOVE: u16 = 108;
const SEARCH: u16 = 109;
const LIST: u16 = 110;
const INPUT: u16 = 111;
const IMPORT: u16 = 112;
const ACTIONS: u16 = 113;
const EXPORT_ALL: u16 = 114;
const EXPORT_ONE: u16 = 115;
const HELP: u16 = 116;
const HOME_IMPORT: u16 = 117;
const THEME: u16 = 130;
const BASE: i32 = 208;
const BRAND: &str = "sh1n VPN";
const MUTEX: &str = "Local\\sh1nVPNStore";
#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Transfer,
    Themes,
}
thread_local! { static UI: RefCell<Option<App>> = const { RefCell::new(None) }; }

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn num(n: usize, one: &str, few: &str, many: &str) -> String {
    let term = match (n % 100, n % 10) {
        (11..=14, _) => many,
        (_, 1) => one,
        (_, 2..=4) => few,
        _ => many,
    };
    format!("{n} {term}")
}
fn profiles(n: usize) -> String {
    num(n, "профиль", "профиля", "профилей")
}

pub fn run(showcase: Option<&str>) {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mutex = CreateMutexW(
            null(),
            0,
            wide(if showcase.is_some() {
                "Local\\sh1nVPNShowcase"
            } else {
                MUTEX
            })
            .as_ptr(),
        );
        if mutex.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
            if !mutex.is_null() {
                CloseHandle(mutex);
            }
            msg(null_mut(), "sh1n VPN уже открыт.", MB_ICONINFORMATION);
            return;
        }
        let inst = GetModuleHandleW(null());
        let icon = app_icon(inst);
        let class = wide("Sh1nVpnWindow");
        let wnd = WNDCLASSW {
            hInstance: inst,
            lpszClassName: class.as_ptr(),
            lpfnWndProc: Some(proc),
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hIcon: if icon.is_null() {
                LoadIconW(null_mut(), IDI_APPLICATION)
            } else {
                icon
            },
            style: CS_HREDRAW | CS_VREDRAW,
            ..Default::default()
        };
        if RegisterClassW(&wnd) == 0 {
            CloseHandle(mutex);
            return;
        }
        let dpi = GetDpiForSystem().max(96) as i32;
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            class.as_ptr(),
            wide(if showcase.is_some() {
                "sh1n VPN — демо"
            } else {
                BRAND
            })
            .as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            1100 * dpi / 96,
            755 * dpi / 96,
            null_mut(),
            null_mut(),
            inst,
            null(),
        );
        if hwnd.is_null() {
            CloseHandle(mutex);
            return;
        }
        let (store, writable) = if showcase.is_some() {
            (demo_store(), false)
        } else {
            match store::read() {
                Ok(s) => (s, true),
                Err(_) => (Store::default(), false),
            }
        };
        let mut app = App::new(hwnd, store, writable, showcase.is_some());
        app.page = match showcase {
            Some("transfer") => Page::Transfer,
            Some("themes") => Page::Themes,
            _ => Page::Home,
        };
        if app.demo && app.page == Page::Transfer {
            SetWindowTextW(app.handle(INPUT),wide("vless://••••@demo.example:443?security=reality#Demo\r\nhy2://••••@demo.example:443#Demo").as_ptr());
        }
        app.filter();
        app.layout();
        UI.with(|slot| *slot.borrow_mut() = Some(app));
        DragAcceptFiles(hwnd, if showcase.is_some() { 0 } else { 1 });
        ShowWindow(hwnd, SW_SHOWNORMAL);
        UpdateWindow(hwnd);
        let mut message = MSG::default();
        while GetMessageW(&mut message, null_mut(), 0, 0) > 0 {
            if message.message == WM_KEYDOWN && GetKeyState(VK_CONTROL as i32) < 0 {
                let cmd = match message.wParam {
                    0x46 => Some(SEARCH),
                    0x49 => Some(TRANSFER),
                    _ => None,
                };
                if let Some(cmd) = cmd {
                    SendMessageW(hwnd, WM_COMMAND, cmd as usize, 0);
                    continue;
                }
            }
            if IsDialogMessageW(hwnd, &message) == 0 {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        UI.with(|slot| {
            slot.borrow_mut().take();
        });
        if !icon.is_null() {
            DestroyIcon(icon);
        }
        CloseHandle(mutex);
    }
}

pub fn import_cli(expected_host: Option<&str>) -> i32 {
    let mutex = unsafe { CreateMutexW(null(), 0, wide(MUTEX).as_ptr()) };
    if mutex.is_null() {
        return 2;
    }
    let exit = (|| {
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            println!("sh1n VPN уже открыт.");
            return 2;
        }
        let Some(text) = (unsafe { clipboard(null_mut()) }) else {
            println!("Буфер обмена не содержит текста.");
            return 3;
        };
        let text = if let Some(host) = expected_host {
            let Some(url) = subscription::parse(&text).filter(|u| u.host_str() == Some(host))
            else {
                println!("В буфере обмена нет ожидаемой HTTPS-подписки.");
                return 3;
            };
            match subscription::fetch(&url) {
                Ok(s) => s,
                Err(e) => {
                    println!("{e}");
                    return 4;
                }
            }
        } else {
            text
        };
        let Ok(mut s) = store::read() else {
            println!("Не удалось открыть локальное хранилище.");
            return 5;
        };
        let (added, duplicate, skipped, protected) = merge(&mut s, &text);
        if added > 0 && store::write(&s).is_err() {
            println!("Не удалось сохранить профили.");
            return 5;
        }
        println!("Добавлено: {added}; дубликаты: {duplicate}; пропущено: {skipped}; защищено: {protected}; всего: {}.",s.profiles.len());
        if added + duplicate > 0 {
            0
        } else {
            6
        }
    })();
    unsafe {
        CloseHandle(mutex);
    }
    exit
}

fn demo_store() -> Store {
    let mut profiles = Vec::new();
    for (name, kind) in [
        ("Франкфурт · основной", "vless"),
        ("Хельсинки · резервный", "hysteria"),
        ("Амстердам · личный", "wireguard"),
        ("Осло · дополнительный", "trojan"),
    ] {
        profiles.push(profile::Profile {
            name: name.into(),
            kind: kind.into(),
            outbounds: Vec::new(),
        });
    }
    Store {
        theme: 0,
        selected: 0,
        profiles,
    }
}

fn merge(s: &mut Store, data: &str) -> (usize, usize, usize, usize) {
    let profile::Imported {
        profiles: found,
        skipped,
        protected,
        urls,
    } = profile::import(data);
    let before = s.profiles.len();
    let mut duplicate = 0;
    let mut full = 0;
    for p in found {
        if s.profiles.iter().any(|x| x.outbounds == p.outbounds) {
            duplicate += 1;
        } else if s.profiles.len() >= profile::MAX_PROFILES {
            full += 1;
        } else {
            s.profiles.push(p);
        }
    }
    (
        s.profiles.len() - before,
        duplicate,
        skipped + protected + urls + full,
        protected,
    )
}

struct App {
    hwnd: HWND,
    store: Store,
    writable: bool,
    controls: Vec<(u16, HWND)>,
    visible: Vec<usize>,
    mode: Mode,
    page: Page,
    demo: bool,
    notice: String,
    core: Option<Core>,
    pending: Option<mpsc::Receiver<Result<Core, &'static str>>>,
    download: Option<mpsc::Receiver<Result<String, String>>>,
    dpi: i32,
    fonts: [HFONT; 6],
    edit_brush: HBRUSH,
    list_brush: HBRUSH,
}

impl App {
    unsafe fn new(hwnd: HWND, store: Store, writable: bool, demo: bool) -> Self {
        let mut a = Self {
            hwnd,
            store,
            writable,
            controls: Vec::new(),
            visible: Vec::new(),
            mode: Mode::Proxy,
            page: Page::Home,
            demo,
            notice: if writable || demo {
                String::new()
            } else {
                "Хранилище не удалось прочитать. Сохранение отключено.".into()
            },
            core: None,
            pending: None,
            download: None,
            dpi: GetDpiForWindow(hwnd).max(96) as i32,
            fonts: [null_mut(); 6],
            edit_brush: null_mut(),
            list_brush: null_mut(),
        };
        a.store.theme = a.store.theme.min(THEMES.len() - 1);
        for (id, title) in [
            (HOME, "Подключение"),
            (TRANSFER, "Перенос"),
            (THEMES_NAV, "Темы"),
            (CONNECT, "Подключить"),
            (PROXY, "Прокси"),
            (TUN, "VPN / TUN"),
            (CLIP, "Вставить из буфера"),
            (FILE, "Открыть файл…"),
            (IMPORT, "Импортировать"),
            (HOME_IMPORT, "+ Импорт"),
            (ACTIONS, "Действия  ▾"),
            (EXPORT_ALL, "Экспортировать все…"),
            (HELP, "Как перенести профили?"),
        ] {
            a.add(id, "BUTTON", title, WS_TABSTOP | BS_OWNERDRAW as u32);
        }
        for (i, t) in THEMES.iter().enumerate() {
            a.add(
                THEME + i as u16,
                "BUTTON",
                t.name,
                WS_TABSTOP | BS_OWNERDRAW as u32,
            );
        }
        a.add(SEARCH, "EDIT", "", WS_TABSTOP | ES_AUTOHSCROLL as u32);
        a.add(
            INPUT,
            "EDIT",
            "",
            WS_TABSTOP
                | WS_VSCROLL
                | ES_MULTILINE as u32
                | ES_AUTOVSCROLL as u32
                | ES_WANTRETURN as u32,
        );
        SendMessageW(a.handle(INPUT), EM_SETLIMITTEXT, profile::MAX_INPUT, 0);
        a.add(
            LIST,
            "LISTBOX",
            "",
            WS_TABSTOP
                | WS_VSCROLL
                | LBS_NOTIFY as u32
                | LBS_NOINTEGRALHEIGHT as u32
                | LBS_OWNERDRAWFIXED as u32
                | LBS_HASSTRINGS as u32,
        );
        SendMessageW(
            a.handle(SEARCH),
            EM_SETCUEBANNER,
            0,
            wide("Поиск профиля   ·   Ctrl+F").as_ptr() as isize,
        );
        SendMessageW(
            a.handle(SEARCH),
            EM_SETMARGINS,
            3,
            ((a.s(9) << 16) | a.s(9)) as isize,
        );
        a.resources();
        a
    }
    fn t(&self) -> &'static Theme {
        &THEMES[self.store.theme]
    }
    fn s(&self, x: i32) -> i32 {
        (x * self.dpi + 48) / 96
    }
    fn logical(&self, x: i32) -> i32 {
        x * 96 / self.dpi
    }
    fn handle(&self, id: u16) -> HWND {
        self.controls
            .iter()
            .find(|v| v.0 == id)
            .map(|v| v.1)
            .unwrap_or(null_mut())
    }
    unsafe fn add(&mut self, id: u16, class: &str, title: &str, flags: u32) {
        let h = CreateWindowExW(
            0,
            wide(class).as_ptr(),
            wide(title).as_ptr(),
            WS_CHILD | flags,
            0,
            0,
            0,
            0,
            self.hwnd,
            id as usize as HMENU,
            GetModuleHandleW(null()),
            null(),
        );
        self.controls.push((id, h));
    }
    unsafe fn resources(&mut self) {
        for font in self.fonts {
            if !font.is_null() {
                DeleteObject(font.cast());
            }
        }
        for (i, (px, weight, face)) in [
            (15, 400, "Segoe UI"),
            (15, 600, "Segoe UI"),
            (38, 700, "Segoe UI"),
            (25, 600, "Segoe UI"),
            (12, 600, "Segoe UI"),
            (14, 400, "Consolas"),
        ]
        .into_iter()
        .enumerate()
        {
            self.fonts[i] = CreateFontW(
                -self.s(px),
                0,
                0,
                0,
                weight,
                0,
                0,
                0,
                DEFAULT_CHARSET.into(),
                OUT_DEFAULT_PRECIS.into(),
                CLIP_DEFAULT_PRECIS.into(),
                CLEARTYPE_QUALITY.into(),
                0,
                wide(face).as_ptr(),
            );
        }
        SendMessageW(self.handle(SEARCH), WM_SETFONT, self.fonts[0] as usize, 1);
        SendMessageW(self.handle(INPUT), WM_SETFONT, self.fonts[5] as usize, 1);
        SendMessageW(
            self.handle(INPUT),
            EM_SETMARGINS,
            3,
            ((self.s(12) << 16) | self.s(12)) as isize,
        );
        SendMessageW(self.handle(LIST), LB_SETITEMHEIGHT, 0, self.s(48) as isize);
        self.brush();
    }
    unsafe fn brush(&mut self) {
        if !self.edit_brush.is_null() {
            DeleteObject(self.edit_brush.cast());
        }
        if !self.list_brush.is_null() {
            DeleteObject(self.list_brush.cast());
        }
        self.edit_brush = CreateSolidBrush(self.t().panel);
        self.list_brush = CreateSolidBrush(self.t().card);
        let bg = self.t().rail;
        let text = self.t().text;
        let dark = i32::from((bg & 0xff) < 120);
        if DwmSetWindowAttribute(self.hwnd, 20, (&dark as *const i32).cast(), 4) != 0 {
            DwmSetWindowAttribute(self.hwnd, 19, (&dark as *const i32).cast(), 4);
        }
        DwmSetWindowAttribute(self.hwnd, 35, (&bg as *const u32).cast(), 4);
        DwmSetWindowAttribute(self.hwnd, 36, (&text as *const u32).cast(), 4);
    }
    unsafe fn size(&self) -> (i32, i32) {
        let mut r = RECT::default();
        GetClientRect(self.hwnd, &mut r);
        (self.logical(r.right), self.logical(r.bottom))
    }
    unsafe fn layout(&mut self) {
        let (w, h) = self.size();
        if w < 400 || h < 400 {
            return;
        }
        let x = BASE + 34;
        let r = w - 34;
        let gap = 14;
        let home = self.page == Page::Home;
        let transfer = self.page == Page::Transfer;
        let themes = self.page == Page::Themes;
        let place = |id, x, y, w, h, show| {
            let win = self.handle(id);
            if show {
                MoveWindow(win, self.s(x), self.s(y), self.s(w), self.s(h), 1);
            }
            ShowWindow(win, if show { SW_SHOWNA } else { SW_HIDE });
        };
        place(HOME, 15, 149, BASE - 30, 46, true);
        place(TRANSFER, 15, 202, BASE - 30, 46, true);
        place(THEMES_NAV, 15, 255, BASE - 30, 46, true);
        place(CONNECT, r - 191, 149, 165, 50, home);
        place(PROXY, x, 262, (r - x - gap) / 2, 60, home);
        place(
            TUN,
            x + (r - x - gap) / 2 + gap,
            262,
            (r - x - gap) / 2,
            60,
            home,
        );
        place(SEARCH, x + 78, 385, r - x - 334, 27, home);
        place(HOME_IMPORT, r - 237, 376, 112, 44, home);
        place(ACTIONS, r - 115, 376, 115, 44, home);
        place(
            LIST,
            x + 2,
            435,
            r - x - 4,
            (h - 498).max(50),
            home && !self.visible.is_empty(),
        );
        place(INPUT, x + 14, 174, r - x - 28, 103, transfer);
        place(CLIP, x + 16, 298, 166, 44, transfer);
        place(FILE, x + 192, 298, 147, 44, transfer);
        place(IMPORT, r - 175, 298, 159, 44, transfer);
        place(EXPORT_ALL, r - 204, 432, 187, 46, transfer);
        place(HELP, x + 1, 530, 228, 42, transfer);
        let c = (r - x - gap) / 2;
        for i in 0..THEMES.len() {
            place(
                THEME + i as u16,
                x + (i as i32 % 2) * (c + gap),
                118 + (i as i32 / 2) * 88,
                c,
                74,
                themes,
            );
        }
        self.refresh();
    }
    unsafe fn refresh(&self) {
        InvalidateRect(self.hwnd, null(), 0);
        for (id, h) in &self.controls {
            InvalidateRect(*h, null(), i32::from(matches!(*id, SEARCH | INPUT | LIST)));
        }
    }
    unsafe fn filter(&mut self) {
        let mut query = [0u16; 256];
        let n = GetWindowTextW(self.handle(SEARCH), query.as_mut_ptr(), query.len() as i32);
        let q = String::from_utf16_lossy(&query[..n as usize]).to_lowercase();
        let q = q.trim();
        let list = self.handle(LIST);
        SendMessageW(list, WM_SETREDRAW, 0, 0);
        SendMessageW(list, LB_RESETCONTENT, 0, 0);
        self.visible.clear();
        for (i, p) in self.store.profiles.iter().enumerate() {
            if !p.name.to_lowercase().contains(q) && !p.kind.contains(q) {
                continue;
            }
            let row = SendMessageW(list, LB_ADDSTRING, 0, wide(&p.name).as_ptr() as isize);
            if row < 0 {
                continue;
            }
            if i == self.store.selected {
                SendMessageW(list, LB_SETCURSEL, row as usize, 0);
            }
            self.visible.push(i);
        }
        SendMessageW(list, WM_SETREDRAW, 1, 0);
    }
    unsafe fn persist(&mut self) -> bool {
        if !self.writable || store::write(&self.store).is_err() {
            self.notice = "Не удалось сохранить локальное хранилище профилей.".into();
            false
        } else {
            true
        }
    }
    unsafe fn add_profiles(&mut self, data: &str) {
        if self.demo {
            return;
        }
        let previous = self.store.profiles.len();
        let previous_selected = self.store.selected;
        let (added, duplicate, skipped, protected) = merge(&mut self.store, data);
        let mut saved = true;
        if added > 0 {
            if previous == 0 {
                self.store.selected = 0;
            }
            saved = self.persist();
            if !saved {
                self.store.profiles.truncate(previous);
                self.store.selected = previous_selected;
            }
            self.filter();
        }
        if saved {
            self.notice = if protected > 0 && added == 0 {
                "Скрытые и зашифрованные профили Happ не импортируются.".into()
            } else if added + duplicate == 0 {
                "Профили не найдены. Проверьте формат файла или ссылки.".into()
            } else {
                format!("Добавлено: {added}  ·  Дубликаты: {duplicate}  ·  Пропущено: {skipped}")
            };
            if added > 0 {
                SetWindowTextW(self.handle(INPUT), wide("").as_ptr());
                self.page = Page::Home;
            }
        }
        self.layout();
    }
    unsafe fn import(&mut self, data: &str) {
        if self.demo {
            return;
        }
        if data.trim().is_empty() {
            self.notice = "Вставьте ссылку или откройте файл.".into();
            return;
        }
        let Some(url) = subscription::parse(data) else {
            self.add_profiles(data);
            return;
        };
        if self.download.is_some() {
            self.notice = "Импорт подписки уже выполняется.".into();
            return;
        }
        let host = url.host_str().unwrap_or("");
        if msg(self.hwnd,&format!("Загрузить профили подписки с {host}?\n\nСсылка не сохраняется. sh1n VPN не меняет настройки Happ."),
            MB_ICONQUESTION|MB_YESNO|MB_DEFBUTTON2)!=IDYES { return; }
        let (tx, rx) = mpsc::channel();
        self.download = Some(rx);
        self.notice = "Получаем профили подписки…".into();
        thread::spawn(move || {
            let _ = tx.send(subscription::fetch(&url));
        });
        SetTimer(self.hwnd, 2, 300, None);
    }
    unsafe fn poll_import(&mut self) {
        let Some(rx) = &self.download else {
            return;
        };
        let r = match rx.try_recv() {
            Ok(v) => v,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(_) => Err("Импорт неожиданно завершился.".into()),
        };
        self.download = None;
        KillTimer(self.hwnd, 2);
        match r {
            Ok(data) => self.add_profiles(&data),
            Err(e) => self.notice = e,
        }
        self.refresh();
    }
    unsafe fn command(&mut self, id: u16, code: u16) {
        match id {
            HOME | THEMES_NAV | TRANSFER | HOME_IMPORT => {
                self.page = match id {
                    HOME => Page::Home,
                    THEMES_NAV => Page::Themes,
                    _ => Page::Transfer,
                };
                self.layout();
                if id == HOME_IMPORT {
                    SetFocus(self.handle(INPUT));
                }
            }
            PROXY | TUN if self.core.is_none() && self.pending.is_none() => {
                self.mode = if id == TUN { Mode::Tun } else { Mode::Proxy };
                self.notice = String::new();
            }
            CONNECT => self.connect(),
            SEARCH if code == EN_CHANGE as u16 => {
                self.filter();
                self.layout();
            }
            SEARCH if code == 0 => {
                self.page = Page::Home;
                self.layout();
                SetFocus(self.handle(SEARCH));
            }
            LIST if code == LBN_SELCHANGE as u16 => {
                let row = SendMessageW(self.handle(LIST), LB_GETCURSEL, 0, 0);
                if let Some(&i) = self.visible.get(row as usize) {
                    if self.core.is_none() && self.pending.is_none() {
                        self.store.selected = i;
                        if self.writable {
                            let _ = self.persist();
                        }
                    } else if let Some(idx) =
                        self.visible.iter().position(|&n| n == self.store.selected)
                    {
                        SendMessageW(self.handle(LIST), LB_SETCURSEL, idx, 0);
                    }
                }
            }
            LIST if code == LBN_DBLCLK as u16 && self.core.is_none() && self.pending.is_none() => {
                self.connect()
            }
            CLIP if !self.demo => match clipboard(self.hwnd) {
                Some(data) => {
                    let normalized = data.replace("\r\n", "\n").replace('\n', "\r\n");
                    SetWindowTextW(self.handle(INPUT), wide(&normalized).as_ptr());
                    SetFocus(self.handle(INPUT));
                    self.notice = "Текст вставлен. Нажмите «Импортировать».".into();
                }
                None => {
                    self.notice =
                        "В буфере нет подходящего текста. Скопируйте ссылку или JSON.".into()
                }
            },
            IMPORT if !self.demo => match edit_input(self.handle(INPUT)) {
                Some(data) => self.import(&data),
                None => self.notice = "Текст слишком большой или повреждён.".into(),
            },
            FILE if !self.demo => match read_file(self.hwnd) {
                Some(Ok(data)) => self.import(&data),
                Some(Err(())) => {
                    self.notice = "Нужен текстовый UTF-8 файл размером до 4 МБ.".into()
                }
                None => {}
            },
            ACTIONS => {
                let menu = CreatePopupMenu();
                if !menu.is_null() {
                    let sel = self.selected().is_some() && !self.demo;
                    AppendMenuW(
                        menu,
                        if sel { MF_STRING } else { MF_GRAYED },
                        EXPORT_ONE as usize,
                        wide("Экспорт выбранного…").as_ptr(),
                    );
                    AppendMenuW(
                        menu,
                        if !self.store.profiles.is_empty() && !self.demo {
                            MF_STRING
                        } else {
                            MF_GRAYED
                        },
                        EXPORT_ALL as usize,
                        wide("Экспорт всех профилей…").as_ptr(),
                    );
                    AppendMenuW(menu, MF_SEPARATOR, 0, null());
                    AppendMenuW(
                        menu,
                        if sel && self.core.is_none() && self.pending.is_none() {
                            MF_STRING
                        } else {
                            MF_GRAYED
                        },
                        REMOVE as usize,
                        wide("Удалить выбранный…").as_ptr(),
                    );
                    let mut r = RECT::default();
                    GetWindowRect(self.handle(ACTIONS), &mut r);
                    let chosen = TrackPopupMenu(
                        menu,
                        TPM_RIGHTALIGN | TPM_TOPALIGN | TPM_RETURNCMD | TPM_NONOTIFY,
                        r.right,
                        r.bottom + 3,
                        0,
                        self.hwnd,
                        null(),
                    ) as u16;
                    DestroyMenu(menu);
                    if chosen != 0 {
                        self.command(chosen, 0);
                    }
                }
            }
            EXPORT_ONE | EXPORT_ALL if !self.demo => self.export(id == EXPORT_ALL),
            HELP => {
                msg(self.hwnd,"Happ: скопируйте открытую HTTPS-ссылку подписки или выберите «Копировать» / «JSON» в меню сервера.\n\nv2rayN / v2rayNG: выберите нужные серверы и экспортируйте ссылки или JSON. В sh1n VPN вставьте их сюда или откройте файл.\n\nСкрытые профили не извлекаются. Активный VPN не останавливается.",MB_ICONINFORMATION);
            }
            REMOVE if !self.demo && self.core.is_none() && self.pending.is_none() => {
                let i = self.store.selected;
                let row = SendMessageW(self.handle(LIST), LB_GETCURSEL, 0, 0);
                if row >= 0
                    && self.visible.get(row as usize) == Some(&i)
                    && i < self.store.profiles.len()
                    && msg(
                        self.hwnd,
                        "Удалить выбранный профиль только из sh1n VPN? Happ не изменится.",
                        MB_ICONQUESTION | MB_YESNO | MB_DEFBUTTON2,
                    ) == IDYES
                {
                    let p = self.store.profiles.remove(i);
                    self.store.selected = i.min(self.store.profiles.len().saturating_sub(1));
                    if self.persist() {
                        self.notice = "Профиль удалён.".into();
                    } else {
                        self.store.profiles.insert(i, p);
                        self.store.selected = i;
                    }
                    self.filter();
                    self.layout();
                }
            }
            n if (THEME..THEME + THEMES.len() as u16).contains(&n) => {
                let before = self.store.theme;
                self.store.theme = (n - THEME) as usize;
                if !self.demo && !self.persist() {
                    self.store.theme = before;
                }
                self.brush();
                InvalidateRect(self.handle(SEARCH), null(), 1);
                InvalidateRect(self.handle(INPUT), null(), 1);
            }
            _ => {}
        }
        self.refresh();
    }
    unsafe fn selected(&self) -> Option<usize> {
        let i = SendMessageW(self.handle(LIST), LB_GETCURSEL, 0, 0);
        self.visible.get(i as usize).copied()
    }
    unsafe fn export(&mut self, all: bool) {
        let items = if all {
            &self.store.profiles[..]
        } else {
            let Some(i) = self.selected() else {
                return;
            };
            &self.store.profiles[i..=i]
        };
        if items.is_empty() {
            self.notice = "Нет профилей для экспорта.".into();
            return;
        }
        let Some(mut data) = profile::export(items) else {
            self.notice = "Экспорт превышает 4 МБ. Сохраните профили по одному.".into();
            return;
        };
        if msg(self.hwnd,"JSON будет содержать ключи доступа без шифрования. Храните файл в безопасном месте и не публикуйте его.",MB_ICONWARNING|MB_OKCANCEL|MB_DEFBUTTON2)==IDOK {
            match save_file(self.hwnd,&data) {
                Some(Ok(()))=>self.notice=format!("Экспортировано: {}.",profiles(items.len())),
                Some(Err(()))=>self.notice="Не удалось записать файл экспорта.".into(),
                None=>{},
            }
        }
        data.fill(0);
    }
    unsafe fn connect(&mut self) {
        if self.demo {
            self.notice = "Демонстрационный режим — подключение недоступно.".into();
            return;
        }
        if self.pending.is_some() {
            return;
        }
        if let Some(c) = self.core.take() {
            drop(c);
            KillTimer(self.hwnd, 1);
            self.notice = "Xray остановлен.".into();
            self.label();
            return;
        }
        let row = SendMessageW(self.handle(LIST), LB_GETCURSEL, 0, 0);
        if row < 0 {
            self.notice = "Сначала добавьте и выберите профиль.".into();
            if self.store.profiles.is_empty() {
                self.page = Page::Transfer;
                self.layout();
                SetFocus(self.handle(INPUT));
            }
            return;
        }
        let Some(profile) = self.store.profiles.get(self.store.selected).cloned() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        let mode = self.mode;
        self.pending = Some(rx);
        self.notice = String::new();
        thread::spawn(move || {
            let _ = tx.send(Core::start(&profile, mode));
        });
        SetTimer(self.hwnd, 1, 350, None);
        self.label();
    }
    unsafe fn poll(&mut self) {
        let mut changed = false;
        if let Some(rx) = &self.pending {
            let result = match rx.try_recv() {
                Ok(r) => r,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(_) => Err("Поток запуска Xray неожиданно завершился."),
            };
            self.pending = None;
            changed = true;
            match result {
                Ok(c) => self.core = Some(c),
                Err(e) => self.notice = e.into(),
            }
        }
        if self.core.as_mut().is_some_and(|c| !c.running()) {
            self.core.take();
            changed = true;
            self.notice = "Ядро Xray завершилось.".into();
        }
        if self.core.is_none() && self.pending.is_none() {
            KillTimer(self.hwnd, 1);
        }
        if changed {
            self.label();
            self.refresh();
        }
    }
    unsafe fn label(&self) {
        let text = if self.pending.is_some() {
            "Запуск…"
        } else if self.core.is_some() {
            "Отключить"
        } else {
            "Подключить"
        };
        SetWindowTextW(self.handle(CONNECT), wide(text).as_ptr());
    }
}

impl Drop for App {
    fn drop(&mut self) {
        self.core.take();
        unsafe {
            for f in self.fonts {
                if !f.is_null() {
                    DeleteObject(f.cast());
                }
            }
            if !self.edit_brush.is_null() {
                DeleteObject(self.edit_brush.cast());
            }
            if !self.list_brush.is_null() {
                DeleteObject(self.list_brush.cast());
            }
        }
    }
}

unsafe extern "system" fn proc(hwnd: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if m == WM_DESTROY {
        PostQuitMessage(0);
        return 0;
    }
    if m == WM_ERASEBKGND {
        return 1;
    }
    UI.with(|slot| {
        let Ok(mut slot) = slot.try_borrow_mut() else {
            return DefWindowProcW(hwnd, m, w, l);
        };
        let Some(a) = slot.as_mut() else {
            return DefWindowProcW(hwnd, m, w, l);
        };
        match m {
            WM_COMMAND => {
                a.command(w as u16, (w >> 16) as u16);
                0
            }
            WM_SIZE => {
                a.layout();
                0
            }
            WM_TIMER if w == 1 => {
                a.poll();
                0
            }
            WM_TIMER if w == 2 => {
                a.poll_import();
                0
            }
            WM_DROPFILES => {
                let drop = w as HDROP;
                let count = DragQueryFileW(drop, u32::MAX, null_mut(), 0);
                let n = if count == 1 {
                    DragQueryFileW(drop, 0, null_mut(), 0)
                } else {
                    0
                };
                let mut buf = vec![0u16; n as usize + 1];
                let got = if n > 0 {
                    DragQueryFileW(drop, 0, buf.as_mut_ptr(), n + 1)
                } else {
                    0
                };
                DragFinish(drop);
                if !a.demo {
                    a.page = Page::Transfer;
                    if got == n && got > 0 {
                        match read_path(std::ffi::OsString::from_wide(&buf[..got as usize])) {
                            Ok(s) => a.import(&s),
                            Err(()) => {
                                a.notice = "Поддерживаются текстовые файлы UTF-8 до 4 МБ.".into()
                            }
                        }
                    } else {
                        a.notice = "Перетащите один файл или общий экспорт.".into();
                    }
                    a.layout();
                }
                0
            }
            WM_CLOSE if a.pending.is_some() => {
                a.notice = "Дождитесь запуска Xray перед закрытием окна.".into();
                a.refresh();
                0
            }
            WM_CLOSE => {
                a.core.take();
                DestroyWindow(hwnd);
                0
            }
            WM_GETMINMAXINFO => {
                let p = &mut *(l as *mut MINMAXINFO);
                p.ptMinTrackSize.x = a.s(920);
                p.ptMinTrackSize.y = a.s(680);
                0
            }
            WM_DPICHANGED => {
                a.dpi = (w as u16 as i32).max(96);
                let r = &*(l as *const RECT);
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    r.left,
                    r.top,
                    r.right - r.left,
                    r.bottom - r.top,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                a.resources();
                a.layout();
                0
            }
            WM_CTLCOLOREDIT => {
                SetBkColor(w as HDC, a.t().panel);
                SetTextColor(w as HDC, a.t().text);
                a.edit_brush as isize
            }
            WM_CTLCOLORLISTBOX => {
                SetBkColor(w as HDC, a.t().card);
                SetTextColor(w as HDC, a.t().text);
                a.list_brush as isize
            }
            WM_DRAWITEM => {
                a.draw(&*(l as *const DRAWITEMSTRUCT));
                1
            }
            WM_PAINT => {
                a.paint();
                0
            }
            _ => DefWindowProcW(hwnd, m, w, l),
        }
    })
}

unsafe fn msg(hwnd: HWND, text: &str, flags: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    MessageBoxW(hwnd, wide(text).as_ptr(), wide(BRAND).as_ptr(), flags)
}
unsafe fn edit_input(hwnd: HWND) -> Option<String> {
    let len = GetWindowTextLengthW(hwnd).max(0) as usize;
    if len > profile::MAX_INPUT {
        return None;
    }
    let mut buffer = vec![0u16; len + 1];
    let n = GetWindowTextW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32).max(0) as usize;
    let s = String::from_utf16(&buffer[..n]).ok()?;
    buffer.fill(0);
    (s.len() <= profile::MAX_INPUT).then_some(s)
}
pub unsafe fn clipboard(hwnd: HWND) -> Option<String> {
    if OpenClipboard(hwnd) == 0 {
        return None;
    }
    let out = (|| {
        let h = GetClipboardData(13);
        if h.is_null() {
            return None;
        }
        let n = GlobalSize(h) / 2;
        if n == 0 || n > profile::MAX_INPUT {
            return None;
        }
        let p = GlobalLock(h).cast::<u16>();
        if p.is_null() {
            return None;
        }
        let s = std::slice::from_raw_parts(p, n);
        let end = s.iter().position(|&x| x == 0);
        let out = end
            .and_then(|i| String::from_utf16(&s[..i]).ok())
            .filter(|s| s.len() <= profile::MAX_INPUT);
        GlobalUnlock(h);
        out
    })();
    CloseClipboard();
    out
}
unsafe fn read_file(hwnd: HWND) -> Option<Result<String, ()>> {
    let mut buffer = [0u16; 32768];
    let filter = wide("Профили и подписки\0*.txt;*.json;*.conf;*.b64\0Все файлы\0*.*\0");
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFile: buffer.as_mut_ptr(),
        nMaxFile: buffer.len() as u32,
        lpstrFilter: filter.as_ptr(),
        Flags: OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_HIDEREADONLY,
        ..Default::default()
    };
    if GetOpenFileNameW(&mut ofn) == 0 {
        return None;
    }
    let end = buffer.iter().position(|&c| c == 0)?;
    Some(read_path(std::ffi::OsString::from_wide(&buffer[..end])))
}
fn read_path(path: std::ffi::OsString) -> Result<String, ()> {
    let mut b = Vec::new();
    File::open(path)
        .map_err(|_| ())?
        .take((profile::MAX_INPUT + 1) as u64)
        .read_to_end(&mut b)
        .map_err(|_| ())?;
    if b.len() > profile::MAX_INPUT {
        return Err(());
    }
    if b.starts_with(&[0xff, 0xfe]) && b.len() % 2 == 0 {
        let u: Vec<_> = b[2..]
            .chunks_exact(2)
            .map(|p| u16::from_le_bytes([p[0], p[1]]))
            .collect();
        return String::from_utf16(&u).map_err(|_| ());
    }
    String::from_utf8(b).map_err(|_| ())
}
unsafe fn save_file(hwnd: HWND, data: &[u8]) -> Option<Result<(), ()>> {
    let mut buffer = [0u16; 32768];
    let name = wide("sh1n-profiles.json");
    buffer[..name.len()].copy_from_slice(&name);
    let filter = wide("Профили Xray (JSON)\0*.json\0");
    let extension = wide("json");
    let mut ofn = OPENFILENAMEW {
        lStructSize: size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFile: buffer.as_mut_ptr(),
        nMaxFile: buffer.len() as u32,
        lpstrFilter: filter.as_ptr(),
        lpstrDefExt: extension.as_ptr(),
        Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR | OFN_HIDEREADONLY,
        ..Default::default()
    };
    if GetSaveFileNameW(&mut ofn) == 0 {
        return None;
    }
    let end = buffer.iter().position(|&c| c == 0)?;
    Some(
        File::create(std::ffi::OsString::from_wide(&buffer[..end]))
            .and_then(|mut f| f.write_all(data))
            .map_err(|_| ()),
    )
}

fn rect(l: i32, t: i32, r: i32, b: i32) -> RECT {
    RECT {
        left: l,
        top: t,
        right: r,
        bottom: b,
    }
}
unsafe fn fill(dc: HDC, r: RECT, c: u32) {
    let b = CreateSolidBrush(c);
    FillRect(dc, &r, b);
    DeleteObject(b.cast());
}
unsafe fn rounded(dc: HDC, r: RECT, rad: i32, c: u32, border: u32) {
    let b = CreateSolidBrush(c);
    let p = CreatePen(PS_SOLID, 1, border);
    let oldb = SelectObject(dc, b.cast());
    let oldp = SelectObject(dc, p.cast());
    RoundRect(dc, r.left, r.top, r.right, r.bottom, rad, rad);
    SelectObject(dc, oldb);
    SelectObject(dc, oldp);
    DeleteObject(b.cast());
    DeleteObject(p.cast());
}
unsafe fn text(dc: HDC, mut r: RECT, font: HFONT, c: u32, s: &str, align: u32) {
    let old = SelectObject(dc, font.cast());
    SetBkMode(dc, TRANSPARENT as i32);
    SetTextColor(dc, c);
    let u = wide(s);
    DrawTextW(
        dc,
        u.as_ptr(),
        (u.len() - 1) as i32,
        &mut r,
        DT_SINGLELINE | DT_VCENTER | DT_NOPREFIX | DT_END_ELLIPSIS | align,
    );
    SelectObject(dc, old);
}

impl App {
    unsafe fn paint(&self) {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(self.hwnd, &mut ps);
        let mut client = RECT::default();
        GetClientRect(self.hwnd, &mut client);
        let (w, h) = self.size();
        let s = |v| self.s(v);
        let p = |l, t, r, b| rect(s(l), s(t), s(r), s(b));
        let t = self.t();
        let x = BASE + 34;
        let right = w - 34;
        fill(dc, client, t.bg);
        fill(dc, p(0, 0, BASE, h), t.rail);
        fill(dc, p(BASE - 1, 0, BASE, h), t.line);
        rounded(dc, p(20, 28, 64, 72), s(14), t.accent, t.accent);
        shield(dc, p(29, 36, 55, 64), t.ink, s(2));
        text(dc, p(75, 26, 193, 61), self.fonts[3], t.text, BRAND, 0);
        text(
            dc,
            p(76, 59, 195, 77),
            self.fonts[4],
            t.muted,
            "КЛИЕНТ XRAY",
            0,
        );
        text(dc, p(22, 116, 196, 140), self.fonts[4], t.muted, "МЕНЮ", 0);
        rounded(dc, p(16, h - 99, BASE - 16, h - 19), s(15), t.card, t.line);
        text(
            dc,
            p(30, h - 86, BASE - 28, h - 60),
            self.fonts[1],
            t.text,
            "Beta 01",
            0,
        );
        text(
            dc,
            p(30, h - 58, BASE - 28, h - 36),
            self.fonts[4],
            t.muted,
            "WINDOWS x64",
            0,
        );
        let (title, subtitle) = match self.page {
            Page::Home => ("Подключение", "Ваши профили. Ваш выбор маршрута."),
            Page::Transfer => (
                "Импорт и экспорт",
                "Переносите свои профили между клиентами и устройствами.",
            ),
            Page::Themes => (
                "Темы",
                "Десять палитр: от мятного графита до светлой пастели.",
            ),
        };
        text(dc, p(x, 28, right, 81), self.fonts[2], t.text, title, 0);
        text(
            dc,
            p(x, 82, right, 108),
            self.fonts[0],
            t.muted,
            subtitle,
            0,
        );
        match self.page {
            Page::Themes => {}
            Page::Transfer => {
                rounded(dc, p(x, 121, right, 359), s(22), t.card, t.line);
                text(
                    dc,
                    p(x + 16, 133, right - 12, 161),
                    self.fonts[1],
                    t.text,
                    "Вставьте ссылку, список или JSON",
                    0,
                );
                rounded(dc, p(x + 12, 167, right - 12, 285), s(13), t.panel, t.line);
                rounded(dc, p(x, 398, right, 512), s(22), t.card, t.line);
                text(
                    dc,
                    p(x + 18, 414, right - 210, 446),
                    self.fonts[3],
                    t.text,
                    "Резервная копия",
                    0,
                );
                text(
                    dc,
                    p(x + 18, 449, right - 212, 478),
                    self.fonts[0],
                    t.muted,
                    "Все профили в одном файле Xray JSON.",
                    0,
                );
                text(
                    dc,
                    p(x + 16, 363, right - 10, 389),
                    self.fonts[4],
                    t.muted,
                    "HTTPS ПОДПИСКИ   /   V2RAYN   /   XRAY JSON   /   WIREGUARD .CONF",
                    0,
                );
            }
            Page::Home => {
                rounded(dc, p(x, 121, right, 240), s(22), t.card, t.line);
                let (status, col) = if self.core.is_some() {
                    ("XRAY РАБОТАЕТ", t.accent)
                } else if self.pending.is_some() {
                    ("ЗАПУСК XRAY", t.accent)
                } else {
                    ("НЕ ПОДКЛЮЧЕНО", t.muted)
                };
                rounded(dc, p(x + 20, 137, x + 164, 160), s(13), t.panel, t.panel);
                rounded(dc, p(x + 30, 144, x + 38, 152), s(7), col, col);
                text(
                    dc,
                    p(x + 45, 136, x + 162, 161),
                    self.fonts[4],
                    col,
                    status,
                    0,
                );
                let name = self
                    .store
                    .profiles
                    .get(self.store.selected)
                    .map(|p| p.name.as_str())
                    .unwrap_or("Нет профиля");
                text(
                    dc,
                    p(x + 20, 166, right - 195, 201),
                    self.fonts[3],
                    t.text,
                    name,
                    0,
                );
                let detail = if let Some(c) = &self.core {
                    if c.mode == Mode::Tun {
                        "TUN  ·  системный трафик".into()
                    } else {
                        format!(
                            "SOCKS5  127.0.0.1:{}   /   HTTP  127.0.0.1:{}",
                            c.socks, c.http
                        )
                    }
                } else if self.mode == Mode::Tun {
                    "Нужны права администратора; Happ должен быть закрыт".into()
                } else {
                    "Системные настройки Windows не меняются".into()
                };
                text(
                    dc,
                    p(x + 20, 202, right - 17, 228),
                    self.fonts[0],
                    t.muted,
                    &detail,
                    0,
                );
                text(
                    dc,
                    p(x, 337, right, 370),
                    self.fonts[3],
                    t.text,
                    "Профили",
                    0,
                );
                text(
                    dc,
                    p(x + 128, 341, right, 368),
                    self.fonts[0],
                    t.muted,
                    &profiles(self.store.profiles.len()),
                    0,
                );
                rounded(dc, p(x, 376, right - 247, 420), s(13), t.panel, t.line);
                text(
                    dc,
                    p(x + 13, 380, x + 78, 416),
                    self.fonts[1],
                    t.muted,
                    "Поиск",
                    0,
                );
                rounded(dc, p(x, 432, right, h - 59), s(16), t.card, t.line);
                if self.visible.is_empty() {
                    let (heading, desc) = if self.store.profiles.is_empty() {
                        (
                            "Добавьте первый профиль",
                            "Откройте импорт или перетащите файл сюда.",
                        )
                    } else {
                        (
                            "Ничего не найдено",
                            "Попробуйте другое название или протокол.",
                        )
                    };
                    text(
                        dc,
                        p(x + 12, 447, right - 12, 481),
                        self.fonts[3],
                        t.text,
                        heading,
                        DT_CENTER,
                    );
                    text(
                        dc,
                        p(x + 12, 484, right - 12, 517),
                        self.fonts[0],
                        t.muted,
                        desc,
                        DT_CENTER,
                    );
                }
            }
        }
        fill(dc, p(x, h - 48, right, h - 47), t.line);
        let note = if !self.notice.is_empty() {
            &self.notice
        } else if self.demo {
            "Демонстрационные данные"
        } else if self.page == Page::Themes {
            "Тема сохраняется на этом ПК"
        } else {
            "Профили хранятся локально и защищены Windows DPAPI"
        };
        text(
            dc,
            p(x, h - 39, right, h - 11),
            self.fonts[0],
            t.muted,
            note,
            0,
        );
        EndPaint(self.hwnd, &ps);
    }
    unsafe fn draw(&self, d: &DRAWITEMSTRUCT) {
        let r = d.rcItem;
        let t = self.t();
        let dc = d.hDC;
        let id = d.CtlID as u16;
        let s = |v| self.s(v);
        let focused = d.itemState & ODS_FOCUS != 0;
        if id == LIST {
            if d.itemID == u32::MAX {
                return;
            }
            let Some(p) = self
                .visible
                .get(d.itemID as usize)
                .and_then(|&i| self.store.profiles.get(i))
            else {
                return;
            };
            let sel = d.itemState & ODS_SELECTED != 0;
            let c = if sel { t.panel } else { t.card };
            fill(dc, r, c);
            fill(
                dc,
                rect(r.left + s(14), r.bottom - 1, r.right - s(14), r.bottom),
                t.line,
            );
            rounded(
                dc,
                rect(r.left + s(17), r.top + s(17), r.left + s(30), r.top + s(30)),
                s(9),
                if sel { t.accent } else { t.second },
                c,
            );
            text(
                dc,
                rect(r.left + s(43), r.top, r.right - s(125), r.bottom),
                self.fonts[1],
                t.text,
                &p.name,
                0,
            );
            let badge = rect(
                r.right - s(108),
                r.top + s(11),
                r.right - s(15),
                r.bottom - s(11),
            );
            rounded(dc, badge, s(11), if sel { t.card } else { t.panel }, c);
            text(
                dc,
                badge,
                self.fonts[4],
                if sel { t.accent } else { t.muted },
                &if p.kind == "hysteria" {
                    "HYSTERIA2".into()
                } else {
                    p.kind.to_uppercase()
                },
                DT_CENTER,
            );
            if focused {
                DrawFocusRect(dc, &rect(r.left + 2, r.top + 2, r.right - 2, r.bottom - 2));
            }
            return;
        }
        let chosen = match id {
            HOME => self.page == Page::Home,
            THEMES_NAV => self.page == Page::Themes,
            TRANSFER => self.page == Page::Transfer,
            PROXY => self.mode == Mode::Proxy,
            TUN => self.mode == Mode::Tun,
            _ => false,
        };
        let nav = matches!(id, HOME | THEMES_NAV | TRANSFER);
        let power = (id == CONNECT && self.core.is_none() && self.pending.is_none())
            || id == IMPORT
            || id == HOME_IMPORT;
        let pressed = d.itemState & ODS_SELECTED != 0;
        fill(
            dc,
            r,
            if nav {
                t.rail
            } else if matches!(id, CONNECT | CLIP | FILE | IMPORT | EXPORT_ALL) {
                t.card
            } else {
                t.bg
            },
        );
        if let Some(sample) = id.checked_sub(THEME).and_then(|i| THEMES.get(i as usize)) {
            let on = (id - THEME) as usize == self.store.theme;
            rounded(
                dc,
                r,
                s(18),
                if on { t.panel } else { t.card },
                if on || focused { t.accent } else { t.line },
            );
            rounded(
                dc,
                rect(
                    r.left + s(12),
                    r.top + s(12),
                    r.left + s(76),
                    r.bottom - s(12),
                ),
                s(12),
                sample.bg,
                sample.line,
            );
            fill(
                dc,
                rect(
                    r.left + s(19),
                    r.top + s(19),
                    r.left + s(31),
                    r.bottom - s(19),
                ),
                sample.panel,
            );
            rounded(
                dc,
                rect(r.left + s(36), r.top + s(19), r.left + s(69), r.top + s(30)),
                s(6),
                sample.accent,
                sample.accent,
            );
            rounded(
                dc,
                rect(
                    r.left + s(36),
                    r.top + s(35),
                    r.left + s(69),
                    r.bottom - s(19),
                ),
                s(6),
                sample.card,
                sample.line,
            );
            text(
                dc,
                rect(
                    r.left + s(89),
                    r.top + s(13),
                    r.right - s(14),
                    r.top + s(36),
                ),
                self.fonts[1],
                if on { t.accent } else { t.text },
                sample.name,
                0,
            );
            text(
                dc,
                rect(
                    r.left + s(89),
                    r.top + s(41),
                    r.right - s(12),
                    r.bottom - s(10),
                ),
                self.fonts[4],
                t.muted,
                sample.note,
                0,
            );
            return;
        }
        let bg = if power {
            if pressed {
                t.second
            } else {
                t.accent
            }
        } else if chosen || pressed {
            t.panel
        } else if nav {
            t.rail
        } else {
            t.card
        };
        let border = if focused || chosen {
            t.accent
        } else if nav {
            bg
        } else {
            t.line
        };
        rounded(dc, r, s(13), bg, border);
        let mut buf = [0u16; 100];
        let n = GetWindowTextW(d.hwndItem, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        let caption = String::from_utf16_lossy(&buf[..n]);
        let fg = if power {
            t.ink
        } else if chosen {
            t.accent
        } else {
            t.text
        };
        if nav {
            let index = match id {
                HOME => "01",
                TRANSFER => "02",
                _ => "03",
            };
            text(
                dc,
                rect(r.left + s(12), r.top, r.left + s(35), r.bottom),
                self.fonts[4],
                if chosen { t.accent } else { t.muted },
                index,
                0,
            );
            text(
                dc,
                rect(r.left + s(38), r.top, r.right - s(5), r.bottom),
                self.fonts[1],
                fg,
                &caption,
                0,
            );
        } else if matches!(id, PROXY | TUN) {
            text(
                dc,
                rect(r.left + s(15), r.top + s(7), r.right - s(12), r.top + s(32)),
                self.fonts[1],
                fg,
                &caption,
                0,
            );
            let desc = if id == PROXY {
                "Для отдельных программ · SOCKS5 / HTTP"
            } else {
                "Для системного трафика · Wintun"
            };
            text(
                dc,
                rect(
                    r.left + s(15),
                    r.top + s(32),
                    r.right - s(11),
                    r.bottom - s(7),
                ),
                self.fonts[4],
                t.muted,
                desc,
                0,
            );
            if chosen {
                rounded(
                    dc,
                    rect(
                        r.right - s(24),
                        r.top + s(13),
                        r.right - s(14),
                        r.top + s(23),
                    ),
                    s(9),
                    t.accent,
                    t.accent,
                );
            }
        } else {
            text(
                dc,
                rect(r.left + s(4), r.top, r.right - s(4), r.bottom),
                self.fonts[1],
                fg,
                &caption,
                DT_CENTER,
            );
        }
    }
}

unsafe fn shield(dc: HDC, r: RECT, color: u32, width: i32) {
    let (w, h) = (r.right - r.left, r.bottom - r.top);
    let p = |x: i32, y: i32| POINT {
        x: r.left + w * x / 100,
        y: r.top + h * y / 100,
    };
    let pts = [
        p(50, 4),
        p(90, 18),
        p(85, 62),
        p(50, 94),
        p(15, 62),
        p(10, 18),
        p(50, 4),
    ];
    let mark = [p(30, 52), p(45, 66), p(72, 34)];
    let pen = CreatePen(PS_SOLID, width.max(1), color);
    let old = SelectObject(dc, pen.cast());
    Polyline(dc, pts.as_ptr(), pts.len() as i32);
    Polyline(dc, mark.as_ptr(), mark.len() as i32);
    SelectObject(dc, old);
    DeleteObject(pen.cast());
}

unsafe fn app_icon(instance: HINSTANCE) -> HICON {
    let mut and = [0xffu8; 128];
    let mut pixels = [0u8; 4096];
    let shield = [
        (16.0, 6.0),
        (24.0, 9.0),
        (23.0, 18.5),
        (16.0, 25.0),
        (9.0, 18.5),
        (8.0, 9.0),
        (16.0, 6.0),
    ];
    let check = [(11.5, 15.5), (14.5, 18.0), (20.5, 12.0)];
    let near = |px: f64, py: f64, points: &[(f64, f64)]| {
        points.windows(2).any(|seg| {
            let (x, y) = seg[0];
            let (dx, dy) = (seg[1].0 - x, seg[1].1 - y);
            let t = (((px - x) * dx + (py - y) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
            (px - x - dx * t).powi(2) + (py - y - dy * t).powi(2) <= 0.9
        })
    };
    for row in 0..32 {
        for x in 0..32 {
            let y = 31 - row;
            let edge_x = (x as f64 - 15.5).abs() - 9.0;
            let edge_y = (y as f64 - 15.5).abs() - 9.0;
            if edge_x.max(0.0).powi(2) + edge_y.max(0.0).powi(2) > 5.2f64.powi(2) {
                continue;
            }
            and[row * 4 + x / 8] &= !(0x80 >> (x % 8));
            let dark = near(x as f64, y as f64, &shield) || near(x as f64, y as f64, &check);
            let i = (row * 32 + x) * 4;
            pixels[i..i + 4].copy_from_slice(if dark {
                &[0x27, 0x29, 0x10, 0xff]
            } else {
                &[0xd4, 0xeb, 0x86, 0xff]
            });
        }
    }
    CreateIcon(instance, 32, 32, 1, 32, and.as_ptr(), pixels.as_ptr())
}
