//! In-memory native desktop selection. No clipboard, filesystem or webview access.
use image::RgbaImage;
use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::atomic::{AtomicU64, Ordering},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::*,
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus},
        WindowsAndMessaging::*,
    },
};

const MAX_DESKTOP_PIXELS: u64 = 32_000_000; // Two BGRA DIBs, at most 256 MB together.
const MAX_REGION_PIXELS: u64 = 20_000_000;
const TIMEOUT_ID: usize = 1;
static CLASS_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Region {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}
fn region(a: POINT, b: POINT, width: i32, height: i32) -> Option<Region> {
    let x1 = a.x.clamp(0, width);
    let x2 = b.x.clamp(0, width);
    let y1 = a.y.clamp(0, height);
    let y2 = b.y.clamp(0, height);
    let r = Region {
        x: x1.min(x2),
        y: y1.min(y2),
        width: (x2 - x1).abs(),
        height: (y2 - y1).abs(),
    };
    (r.width > 0 && r.height > 0).then_some(r)
}
fn valid_region(r: Region) -> bool {
    r.width > 0
        && r.height > 0
        && r.width <= 12_000
        && r.height <= 12_000
        && r.width as u64 * r.height as u64 <= MAX_REGION_PIXELS
}

struct DesktopDc(HDC);
impl Drop for DesktopDc {
    fn drop(&mut self) {
        unsafe {
            ReleaseDC(null_mut(), self.0);
        }
    }
}
struct Dib {
    dc: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
}
impl Dib {
    unsafe fn new(source: HDC, width: i32, height: i32) -> Result<Self, String> {
        let dc = CreateCompatibleDC(source);
        if dc.is_null() {
            return Err("无法创建截图绘图环境".into());
        }
        let mut info: BITMAPINFO = zeroed();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB;
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(source, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(dc);
            return Err("截图内存分配失败".into());
        }
        let old = SelectObject(dc, bitmap);
        if old.is_null() || old == (-1isize) as HGDIOBJ {
            DeleteObject(bitmap);
            DeleteDC(dc);
            return Err("无法准备截图画面".into());
        }
        Ok(Self {
            dc,
            bitmap,
            old,
            bits: bits.cast(),
        })
    }
}
impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}
struct DpiGuard(DPI_AWARENESS_CONTEXT);
impl Drop for DpiGuard {
    fn drop(&mut self) {
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}
struct WindowGuard(HWND);
impl Drop for WindowGuard {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() && IsWindow(self.0) != 0 {
                DestroyWindow(self.0);
            }
        }
    }
}
struct ClassGuard {
    name: Vec<u16>,
    instance: HINSTANCE,
}
impl Drop for ClassGuard {
    fn drop(&mut self) {
        unsafe {
            UnregisterClassW(self.name.as_ptr(), self.instance);
        }
    }
}
struct Selection {
    original: Dib,
    dimmed: Dib,
    width: i32,
    height: i32,
    start: Option<POINT>,
    current: POINT,
    selected: Option<Region>,
    done: bool,
    failure: Option<&'static str>,
    instruction: Vec<u16>,
    instruction_origin: POINT,
}
impl Selection {
    fn cancel(&mut self, reason: &'static str) {
        self.failure = Some(reason);
        self.done = true;
    }
}
unsafe fn cursor(hwnd: HWND, width: i32, height: i32) -> POINT {
    let mut p: POINT = zeroed();
    GetCursorPos(&mut p);
    ScreenToClient(hwnd, &mut p);
    p.x = p.x.clamp(0, width);
    p.y = p.y.clamp(0, height);
    p
}
unsafe extern "system" fn window_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // Never let a Rust panic cross the system callback boundary.
    match std::panic::catch_unwind(|| dispatch(hwnd, msg, wp, lp)) {
        Ok(value) => value,
        Err(_) => {
            let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Selection;
            if let Some(s) = p.as_mut() {
                s.cancel("截图窗口发生错误，请重试");
            }
            0
        }
    }
}
unsafe fn dispatch(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_NCCREATE {
        let create = &*(lp as *const CREATESTRUCTW);
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
    }
    let p = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Selection;
    if msg == WM_NCDESTROY {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    if p.is_null() {
        return DefWindowProcW(hwnd, msg, wp, lp);
    }
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            let mut ps: PAINTSTRUCT = zeroed();
            let dc = BeginPaint(hwnd, &mut ps);
            let s = &*p;
            BitBlt(dc, 0, 0, s.width, s.height, s.dimmed.dc, 0, 0, SRCCOPY);
            if let Some(r) = s
                .start
                .and_then(|a| region(a, s.current, s.width, s.height))
            {
                BitBlt(
                    dc,
                    r.x,
                    r.y,
                    r.width,
                    r.height,
                    s.original.dc,
                    r.x,
                    r.y,
                    SRCCOPY,
                );
                let rect = RECT {
                    left: r.x,
                    top: r.y,
                    right: r.x + r.width,
                    bottom: r.y + r.height,
                };
                FrameRect(dc, &rect, GetStockObject(WHITE_BRUSH) as HBRUSH);
            }
            SetBkMode(dc, OPAQUE as i32);
            SetBkColor(dc, 0x00282828);
            SetTextColor(dc, 0x00ffffff);
            TextOutW(
                dc,
                s.instruction_origin.x,
                s.instruction_origin.y,
                s.instruction.as_ptr(),
                s.instruction.len() as i32,
            );
            EndPaint(hwnd, &ps);
            0
        }
        WM_LBUTTONDOWN => {
            if !(*p).done {
                let point = cursor(hwnd, (*p).width, (*p).height);
                (*p).start = Some(point);
                (*p).current = point;
                SetCapture(hwnd);
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_MOUSEMOVE => {
            if (*p).start.is_some() && !(*p).done {
                (*p).current = cursor(hwnd, (*p).width, (*p).height);
                InvalidateRect(hwnd, null(), 0);
            }
            0
        }
        WM_LBUTTONUP => {
            if let Some(a) = (*p).start {
                if !(*p).done {
                    (*p).current = cursor(hwnd, (*p).width, (*p).height);
                    let r = region(a, (*p).current, (*p).width, (*p).height);
                    if let Some(r) = r {
                        if valid_region(r) {
                            (*p).selected = Some(r);
                            (*p).done = true;
                        } else {
                            (*p).cancel("选区过大，请缩小到二维码附近后重试");
                        }
                    } else {
                        (*p).cancel("已取消截图");
                    }
                    ReleaseCapture();
                }
            }
            0
        }
        WM_KEYDOWN | WM_SYSKEYDOWN if wp == 27 => {
            (*p).cancel("已取消截图");
            0
        }
        WM_RBUTTONDOWN | WM_CLOSE => {
            (*p).cancel("已取消截图");
            0
        }
        WM_ACTIVATE if wp & 0xffff == WA_INACTIVE as usize => {
            if !(*p).done {
                (*p).cancel("已取消截图");
            }
            0
        }
        WM_CAPTURECHANGED if (*p).start.is_some() && !(*p).done => {
            (*p).cancel("截图已取消，请重试");
            0
        }
        WM_DISPLAYCHANGE => {
            (*p).cancel("显示器配置已变化，请重新截图");
            0
        }
        WM_TIMER if wp == TIMEOUT_ID => {
            (*p).cancel("截图已超时，请重试");
            0
        }
        WM_DESTROY => {
            if !(*p).done {
                (*p).cancel("已取消截图");
            }
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Must run on a dedicated blocking worker, never on the application's UI thread.
pub fn capture_region() -> Result<RgbaImage, String> {
    unsafe {
        let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if previous.is_null() {
            return Err("无法设置截图坐标环境".into());
        }
        let _dpi = DpiGuard(previous);
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let width = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let height = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        if width <= 0 || height <= 0 || width as u64 * height as u64 > MAX_DESKTOP_PIXELS {
            return Err("桌面尺寸过大，暂不支持截图；可改为粘贴二维码图片".into());
        }
        let source = DesktopDc(GetDC(null_mut()));
        if source.0.is_null() {
            return Err("无法读取当前桌面".into());
        }
        let original = Dib::new(source.0, width, height)?;
        if BitBlt(
            original.dc,
            0,
            0,
            width,
            height,
            source.0,
            x,
            y,
            SRCCOPY | CAPTUREBLT,
        ) == 0
        {
            return Err("无法截取当前桌面".into());
        }
        GdiFlush();
        let dimmed = Dib::new(source.0, width, height)?;
        let length = width as usize * height as usize * 4;
        let original_bytes = std::slice::from_raw_parts(original.bits, length);
        let dimmed_bytes = std::slice::from_raw_parts_mut(dimmed.bits, length);
        for (a, b) in original_bytes
            .chunks_exact(4)
            .zip(dimmed_bytes.chunks_exact_mut(4))
        {
            b.copy_from_slice(&[a[0] / 2, a[1] / 2, a[2] / 2, 255]);
        }
        let mut pos: POINT = zeroed();
        GetCursorPos(&mut pos);
        // Place the hint on the current monitor, including monitors at negative origins.
        let monitor = MonitorFromPoint(pos, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        let origin = if GetMonitorInfoW(monitor, &mut info) != 0 {
            POINT {
                x: info.rcMonitor.left - x + 24,
                y: info.rcMonitor.top - y + 24,
            }
        } else {
            POINT { x: 24, y: 24 }
        };
        let mut state = Box::new(Selection {
            original,
            dimmed,
            width,
            height,
            start: None,
            current: POINT { x: 0, y: 0 },
            selected: None,
            done: false,
            failure: None,
            instruction: "  拖动框选二维码 · Esc 或右键取消  "
                .encode_utf16()
                .collect(),
            instruction_origin: origin,
        });
        let instance = GetModuleHandleW(null());
        let name: Vec<u16> = format!(
            "ReelinkNativeCapture{}",
            CLASS_COUNTER.fetch_add(1, Ordering::Relaxed)
        )
        .encode_utf16()
        .chain(Some(0))
        .collect();
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: name.as_ptr(),
            hCursor: LoadCursorW(null_mut(), IDC_CROSS),
            ..zeroed()
        };
        if RegisterClassW(&class) == 0 {
            return Err("无法创建截图窗口".into());
        }
        let _class = ClassGuard { name, instance };
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            _class.name.as_ptr(),
            _class.name.as_ptr(),
            WS_POPUP,
            x,
            y,
            width,
            height,
            null_mut(),
            null_mut(),
            instance,
            (&mut *state as *mut Selection).cast::<c_void>(),
        );
        if hwnd.is_null() {
            return Err("无法打开截图窗口".into());
        }
        // Drop order keeps state alive until HWND destruction clears its userdata.
        let window = WindowGuard(hwnd);
        if SetTimer(hwnd, TIMEOUT_ID, 120_000, None) == 0 {
            return Err("无法启动截图计时器".into());
        }
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd);
        SetFocus(hwnd);
        UpdateWindow(hwnd);
        if GetForegroundWindow() != hwnd {
            return Err("无法聚焦截图窗口，请重试".into());
        }
        let mut msg: MSG = zeroed();
        while !state.done {
            let result = GetMessageW(&mut msg, null_mut(), 0, 0);
            if result <= 0 {
                state.cancel("截图窗口已关闭，请重试");
                if result == 0 {
                    PostQuitMessage(msg.wParam as i32);
                }
                break;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        KillTimer(hwnd, TIMEOUT_ID);
        drop(window);
        if let Some(reason) = state.failure {
            return Err(reason.into());
        }
        let r = state.selected.ok_or("已取消截图")?;
        let bytes = std::slice::from_raw_parts(state.original.bits, length);
        crop_bgra(bytes, width, height, r)
    }
}

fn crop_bgra(bytes: &[u8], width: i32, height: i32, r: Region) -> Result<RgbaImage, String> {
    if !valid_region(r)
        || width <= 0
        || height <= 0
        || r.x < 0
        || r.y < 0
        || i64::from(r.x) + i64::from(r.width) > i64::from(width)
        || i64::from(r.y) + i64::from(r.height) > i64::from(height)
        || width as u64 * height as u64 * 4 != bytes.len() as u64
    {
        return Err("截图选区无效".into());
    }
    let mut pixels = Vec::new();
    let count = r.width as usize * r.height as usize * 4;
    pixels
        .try_reserve_exact(count)
        .map_err(|_| "截图内存不足，请缩小选区")?;
    for row in r.y..r.y + r.height {
        let offset = (row as usize * width as usize + r.x as usize) * 4;
        for bgra in bytes[offset..offset + r.width as usize * 4].chunks_exact(4) {
            pixels.extend_from_slice(&[bgra[2], bgra[1], bgra[0], 255]);
        }
    }
    RgbaImage::from_raw(r.width as u32, r.height as u32, pixels)
        .ok_or_else(|| "截图数据无效".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_preserves_last_row_and_column_and_converts_channels() {
        let bytes = [1, 2, 3, 0, 4, 5, 6, 0, 7, 8, 9, 0, 10, 11, 12, 0];
        let image = crop_bgra(
            &bytes,
            2,
            2,
            Region {
                x: 1,
                y: 0,
                width: 1,
                height: 2,
            },
        )
        .unwrap();
        assert_eq!(image.into_raw(), [6, 5, 4, 255, 12, 11, 10, 255]);
        assert!(crop_bgra(
            &bytes,
            2,
            2,
            Region {
                x: 2,
                y: 0,
                width: 1,
                height: 1
            }
        )
        .is_err());
        assert!(crop_bgra(
            &bytes[..12],
            2,
            2,
            Region {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            }
        )
        .is_err());
    }
    #[test]
    fn reverse_drag_and_outside_edges_are_clamped() {
        assert_eq!(
            region(POINT { x: 110, y: 90 }, POINT { x: -30, y: -20 }, 100, 80),
            Some(Region {
                x: 0,
                y: 0,
                width: 100,
                height: 80
            })
        );
        assert_eq!(
            region(POINT { x: 15, y: 10 }, POINT { x: 5, y: 20 }, 100, 80),
            Some(Region {
                x: 5,
                y: 10,
                width: 10,
                height: 10
            })
        );
    }
    #[test]
    fn empty_drag_is_not_an_image() {
        assert_eq!(
            region(POINT { x: 5, y: 5 }, POINT { x: 5, y: 9 }, 100, 80),
            None
        );
    }
    #[test]
    fn region_limits_are_enforced() {
        assert!(valid_region(Region {
            x: 0,
            y: 0,
            width: 5000,
            height: 4000
        }));
        assert!(!valid_region(Region {
            x: 0,
            y: 0,
            width: 5001,
            height: 4000
        }));
        assert!(!valid_region(Region {
            x: 0,
            y: 0,
            width: 12001,
            height: 1
        }));
        assert!(!valid_region(Region {
            x: 0,
            y: 0,
            width: 0,
            height: 1
        }));
    }
}
