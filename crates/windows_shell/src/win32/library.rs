use super::*;
use windows_shell::{
    library_command, LibraryCommand, ID_LIBRARY_CATEGORY, ID_LIBRARY_CLOSE_BUTTON,
    ID_LIBRARY_DETAILS, ID_LIBRARY_LIST, ID_LIBRARY_OPEN_BUTTON, ID_LIBRARY_REMOVE_BUTTON,
    ID_LIBRARY_SEARCH,
};

const LIBRARY_CLASS: &str = "SpecChumWindowsLibrary\0";

impl AppState {
    pub(super) fn open_library(&mut self, app_ptr: *mut AppState) {
        if let Some(hwnd) = self.library_hwnd {
            unsafe { ShowWindow(hwnd, SW_SHOW) };
            return;
        }
        let Ok(instance) = (unsafe { GetModuleHandleW(None) }) else {
            return;
        };
        let Ok(cursor) = (unsafe { LoadCursorW(None, IDC_ARROW) }) else {
            return;
        };
        let class_name: Vec<u16> = LIBRARY_CLASS.encode_utf16().collect();
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(library_wnd_proc),
            hInstance: instance.into(),
            hCursor: cursor,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..unsafe { zeroed() }
        };
        unsafe {
            let _ = RegisterClassExW(&wc);
        }
        let title: Vec<u16> = "Spec Chum — Library\0".encode_utf16().collect();
        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                760,
                540,
                self.main_hwnd,
                None,
                Some(instance.into()),
                Some(app_ptr.cast()),
            )
        };
        if hwnd.is_err() {
            self.report_err(
                "Library",
                &HostError::Message("could not create Library window".into()),
            );
        }
    }

    fn refresh_library(&mut self) {
        let (Some(search_hwnd), Some(category_hwnd), Some(list_hwnd)) = (
            self.library_search,
            self.library_category,
            self.library_list,
        ) else {
            return;
        };
        let search = window_text(search_hwnd);
        let category_index = send_message(category_hwnd, 0x0147, 0, 0).0;
        let category = match category_index {
            1 => Some(MediaCategory::Tape),
            2 => Some(MediaCategory::Snapshot),
            3 => Some(MediaCategory::Recording),
            4 => Some(MediaCategory::Disk),
            _ => None,
        };
        let recent = self.prefs.recent_files.clone();
        self.library_entries = self
            .host
            .with_mut(|session| query_recent_media(&recent, &search, category, session));
        {
            let _ = send_message(list_hwnd, 0x0184, 0, 0);
            for entry in &self.library_entries {
                let available = if entry.available { "" } else { " [missing]" };
                let label = format!("{} ({:?}){available}", entry.name, entry.format);
                let label: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
                let _ = send_message(list_hwnd, 0x0180, 0, label.as_ptr() as isize);
            }
            if !self.library_entries.is_empty() {
                let _ = send_message(list_hwnd, 0x0186, 0, 0);
            }
        }
        self.refresh_library_details(0);
    }

    fn refresh_library_details(&self, index: usize) {
        let Some(details) = self.library_details else {
            return;
        };
        let text = self.library_entries.get(index).map_or_else(
            || "Select a supported recent item to see details.".to_owned(),
            |entry| {
                let availability = if entry.available {
                    "Available"
                } else {
                    "File missing"
                };
                let compatibility = match entry.compatibility {
                    MediaCompatibility::Ready => "Compatible with current hardware",
                    MediaCompatibility::MayRequireMachine => "May open without a machine",
                    MediaCompatibility::RequiresMachine => "Requires a machine",
                    MediaCompatibility::RequiresPlus3 => "Requires a +3 or +3e",
                    MediaCompatibility::RequiresBetaModel => "Requires a Beta-compatible model",
                    MediaCompatibility::RequiresBeta => "Requires an attached Beta Disk",
                    MediaCompatibility::RequiresTrdosRom => "Requires a loaded TR-DOS ROM",
                    MediaCompatibility::UnsupportedOnNext => "Unsupported on ZX Spectrum Next",
                };
                format!(
                    "{}\r\n{}\r\n{}\r\n{}",
                    entry.name, entry.path, availability, compatibility
                )
            },
        );
        let text: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let _ = SetWindowTextW(details, PCWSTR(text.as_ptr()));
        }
    }

    fn open_selected_library_entry(&mut self) {
        let Some(list) = self.library_list else {
            return;
        };
        let selected = send_message(list, 0x0188, 0, 0).0;
        let Some(entry) = usize::try_from(selected)
            .ok()
            .and_then(|i| self.library_entries.get(i))
        else {
            return;
        };
        let path = PathBuf::from(&entry.path);
        match self.host.with_mut(|session| session.open_media_path(&path)) {
            Ok(()) => {
                self.prefs.push_recent(&path);
                self.persist_prefs();
                self.refresh_recent_menu_entries();
                self.refresh_library();
            }
            Err(error) => self.report_err("Open media", &error),
        }
    }

    fn remove_selected_library_entry(&mut self) {
        let Some(list) = self.library_list else {
            return;
        };
        let selected = send_message(list, 0x0188, 0, 0).0;
        let Some(entry) = usize::try_from(selected)
            .ok()
            .and_then(|i| self.library_entries.get(i))
        else {
            return;
        };
        self.prefs.remove_recent(Path::new(&entry.path));
        self.persist_prefs();
        self.refresh_recent_menu_entries();
        self.refresh_library();
    }
}

fn window_text(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) }.max(0) as usize;
    let mut wide = vec![0_u16; len + 1];
    let copied = unsafe { GetWindowTextW(hwnd, &mut wide) }.max(0) as usize;
    String::from_utf16_lossy(&wide[..copied])
}

fn send_message(hwnd: HWND, message: u32, wparam: usize, lparam: isize) -> LRESULT {
    unsafe { SendMessageW(hwnd, message, Some(WPARAM(wparam)), Some(LPARAM(lparam))) }
}

fn layout_library(app: &AppState, hwnd: HWND) {
    let mut client = RECT::default();
    if unsafe { GetClientRect(hwnd, &mut client) }.is_err() {
        return;
    }
    let width = client.right - client.left;
    let height = client.bottom - client.top;
    let left_width = ((width - 48) / 2).max(220);
    let right_x = left_width + 32;
    let right_width = (width - right_x - 16).max(200);
    let list_height = (height - 70).max(180);
    let details_height = (height - 160).max(100);
    unsafe {
        if let Some(search) = app.library_search {
            let _ = MoveWindow(search, 16, 16, left_width - 8, 26, true);
        }
        if let Some(category) = app.library_category {
            let _ = MoveWindow(category, right_x, 16, right_width, 200, true);
        }
        if let Some(list) = app.library_list {
            let _ = MoveWindow(list, 16, 54, left_width, list_height, true);
        }
        if let Some(details) = app.library_details {
            let _ = MoveWindow(details, right_x, 54, right_width, details_height, true);
        }
        if let Ok(button) = GetDlgItem(Some(hwnd), ID_LIBRARY_OPEN_BUTTON) {
            let _ = MoveWindow(button, right_x, height - 82, 100, 32, true);
        }
        if let Ok(button) = GetDlgItem(Some(hwnd), ID_LIBRARY_REMOVE_BUTTON) {
            let _ = MoveWindow(button, right_x + 108, height - 82, 175, 32, true);
        }
        if let Ok(button) = GetDlgItem(Some(hwnd), ID_LIBRARY_CLOSE_BUTTON) {
            let _ = MoveWindow(button, width - 116, height - 48, 100, 32, true);
        }
    }
}

fn create_library_control(
    instance: windows::Win32::Foundation::HINSTANCE,
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: i32,
) -> Option<HWND> {
    let class: Vec<u16> = class.encode_utf16().chain(std::iter::once(0)).collect();
    let text: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class.as_ptr()),
            PCWSTR(text.as_ptr()),
            WINDOW_STYLE(style),
            x,
            y,
            width,
            height,
            Some(parent),
            Some(HMENU(id as isize as *mut core::ffi::c_void)),
            Some(instance),
            None,
        )
        .ok()
    }
}

extern "system" fn library_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let app_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut AppState;
    match msg {
        WM_CREATE => {
            let cs = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            let app_ptr = cs.lpCreateParams as *mut AppState;
            if app_ptr.is_null() {
                return LRESULT(-1);
            }
            unsafe { SetWindowLongPtrW(hwnd, GWLP_USERDATA, app_ptr as isize) };
            let Ok(instance) = (unsafe { GetModuleHandleW(None) }) else {
                return LRESULT(-1);
            };
            let base = 0x4000_0000_u32 | 0x1000_0000 | 0x0001_0000;
            let border = base | 0x0080_0000;
            let app = unsafe { &mut *app_ptr };
            app.library_hwnd = Some(hwnd);
            app.library_search = create_library_control(
                instance.into(),
                hwnd,
                "EDIT",
                "",
                base | border | 0x0080,
                16,
                16,
                330,
                26,
                ID_LIBRARY_SEARCH,
            );
            app.library_category = create_library_control(
                instance.into(),
                hwnd,
                "COMBOBOX",
                "All types",
                base | 0x0002_0000 | 0x0000_0003 | 0x0000_0200 | 0x0020_0000,
                360,
                16,
                220,
                200,
                ID_LIBRARY_CATEGORY,
            );
            app.library_list = create_library_control(
                instance.into(),
                hwnd,
                "LISTBOX",
                "",
                base | 0x0080_0000 | 0x0020_0000 | 1,
                16,
                54,
                350,
                360,
                ID_LIBRARY_LIST,
            );
            app.library_details = create_library_control(
                instance.into(),
                hwnd,
                "STATIC",
                "",
                base | border,
                382,
                54,
                340,
                250,
                ID_LIBRARY_DETAILS,
            );
            let _ = create_library_control(
                instance.into(),
                hwnd,
                "BUTTON",
                "Open",
                base | 0x0000_0001,
                382,
                326,
                100,
                32,
                ID_LIBRARY_OPEN_BUTTON,
            );
            let _ = create_library_control(
                instance.into(),
                hwnd,
                "BUTTON",
                "Remove from Library",
                base | 0x0000_0001,
                490,
                326,
                175,
                32,
                ID_LIBRARY_REMOVE_BUTTON,
            );
            let _ = create_library_control(
                instance.into(),
                hwnd,
                "BUTTON",
                "Close",
                base | 0x0000_0001,
                622,
                420,
                100,
                32,
                ID_LIBRARY_CLOSE_BUTTON,
            );
            if let Some(combo) = app.library_category {
                for label in ["All types", "Tape", "Snapshot", "Recording", "Disk"] {
                    let label: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
                    let _ = send_message(combo, 0x0143, 0, label.as_ptr() as isize);
                }
                let _ = send_message(combo, 0x014E, 0, 0);
            }
            layout_library(app, hwnd);
            app.refresh_library();
            LRESULT(0)
        }
        WM_SIZE if !app_ptr.is_null() => {
            layout_library(unsafe { &*app_ptr }, hwnd);
            LRESULT(0)
        }
        WM_COMMAND if !app_ptr.is_null() => {
            let id = (wparam.0 & 0xffff) as i32;
            let notification = (wparam.0 >> 16) & 0xffff;
            let app = unsafe { &mut *app_ptr };
            // Only `BN_CLICKED` may open, remove, or close. Focus notifications
            // arrive first on a click and would act on the wrong list row.
            match library_command(id, notification) {
                Some(LibraryCommand::Refresh) => app.refresh_library(),
                Some(LibraryCommand::SelectionChanged) => {
                    let selected = app
                        .library_list
                        .map_or(-1, |list| send_message(list, 0x0188, 0, 0).0);
                    if selected >= 0 {
                        app.refresh_library_details(selected as usize);
                    }
                }
                Some(LibraryCommand::Open) => app.open_selected_library_entry(),
                Some(LibraryCommand::Remove) => app.remove_selected_library_entry(),
                Some(LibraryCommand::Close) => unsafe {
                    let _ = DestroyWindow(hwnd);
                },
                None => {}
            }
            LRESULT(0)
        }
        WM_DESTROY if !app_ptr.is_null() => {
            let app = unsafe { &mut *app_ptr };
            app.library_hwnd = None;
            app.library_search = None;
            app.library_category = None;
            app.library_list = None;
            app.library_details = None;
            app.library_entries.clear();
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
