use std::{
    fs::File,
    io::{Cursor, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::{DynamicImage, ImageBuffer, ImageFormat, ImageReader, Rgba};
use serde::Serialize;

const MAX_INPUT_BYTES: u64 = 24 * 1024 * 1024;
const MAX_IMAGE_PIXELS: u64 = 20_000_000;
const MAX_IMAGE_SIDE: u32 = 12_000;
const MAX_PREVIEW_SIDE: u32 = 1_600;
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(45);
const CAPTURE_POLL_INTERVAL: Duration = Duration::from_millis(80);
static CAPTURE_ACTIVE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QrResult {
    pub texts: Vec<String>,
    pub image_data_url: String,
}

#[tauri::command]
pub async fn decode_clipboard() -> Result<QrResult, String> {
    tauri::async_runtime::spawn_blocking(decode_clipboard_blocking)
        .await
        .map_err(|_| "读取剪贴板时发生内部错误。".to_string())?
}

#[tauri::command]
pub async fn decode_image(data: Vec<u8>) -> Result<QrResult, String> {
    tauri::async_runtime::spawn_blocking(move || decode_bytes(&data))
        .await
        .map_err(|_| "处理图片时发生内部错误。".to_string())?
}

#[tauri::command]
pub async fn decode_image_path(path: String) -> Result<QrResult, String> {
    tauri::async_runtime::spawn_blocking(move || decode_path_blocking(&path))
        .await
        .map_err(|_| "读取图片时发生内部错误。".to_string())?
}

#[tauri::command]
pub fn copy_text(text: String) -> Result<(), String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|_| "无法访问系统剪贴板。".to_string())?;
    clipboard
        .set_text(text)
        .map_err(|_| "复制失败，请检查系统剪贴板是否可用。".to_string())
}

#[tauri::command]
#[cfg(windows)]
pub async fn capture_qr(window: tauri::WebviewWindow) -> Result<QrResult, String> {
    let _capture_guard = CaptureGuard::acquire()?;
    let hide_result = window
        .hide()
        .map_err(|_| "无法暂时隐藏窗口以开始截图。".to_string());
    if let Err(error) = hide_result {
        restore_window(&window);
        return Err(error);
    }

    let result = tauri::async_runtime::spawn_blocking(capture_from_clipboard_blocking)
        .await
        .map_err(|_| "截图处理时发生内部错误。".to_string())
        .and_then(|result| result);

    restore_window(&window);
    result
}

#[tauri::command]
#[cfg(not(windows))]
pub async fn capture_qr(_window: tauri::WebviewWindow) -> Result<QrResult, String> {
    Err("截图识别目前仅支持 Windows。".to_string())
}

#[cfg(windows)]
fn restore_window(window: &tauri::WebviewWindow) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    let webview: &tauri::Webview = window.as_ref();
    let _ = webview.set_focus();
}

#[cfg(windows)]
struct CaptureGuard;

#[cfg(windows)]
impl CaptureGuard {
    fn acquire() -> Result<Self, String> {
        CAPTURE_ACTIVE
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map(|_| Self)
            .map_err(|_| "截图正在进行，请稍后再试。".to_string())
    }
}

#[cfg(windows)]
impl Drop for CaptureGuard {
    fn drop(&mut self) {
        CAPTURE_ACTIVE.store(false, Ordering::Release);
    }
}

fn decode_clipboard_blocking() -> Result<QrResult, String> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|_| "无法访问系统剪贴板。".to_string())?;
    let image = clipboard
        .get_image()
        .map_err(|_| "剪贴板中没有图片，请先复制一张二维码图片。".to_string())?;
    let width = u32::try_from(image.width).map_err(|_| "图片尺寸超出限制。".to_string())?;
    let height = u32::try_from(image.height).map_err(|_| "图片尺寸超出限制。".to_string())?;
    let pixels = checked_pixel_count(width, height)?;
    if image.bytes.len() as u64 != pixels * 4 {
        return Err("剪贴板图片数据不完整，无法读取。".to_string());
    }
    let buffer =
        ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width, height, image.bytes.into_owned())
            .ok_or_else(|| "无法读取剪贴板图片。".to_string())?;
    decode_dynamic_image(DynamicImage::ImageRgba8(buffer))
}

fn decode_path_blocking(path: &str) -> Result<QrResult, String> {
    if path.contains("://") || path.starts_with("\\\\") || path.starts_with("//") {
        return Err("请提供本机图片文件路径。".to_string());
    }

    let path = PathBuf::from(path);
    let canonical = path
        .canonicalize()
        .map_err(|_| "找不到这张图片，请检查文件路径。".to_string())?;
    if is_network_path(&canonical) {
        return Err("仅支持本机上的图片文件。".to_string());
    }
    let metadata = canonical
        .metadata()
        .map_err(|_| "无法读取图片文件信息。".to_string())?;
    if !metadata.is_file() {
        return Err("所选路径不是图片文件。".to_string());
    }
    if metadata.len() > MAX_INPUT_BYTES {
        return Err("图片文件过大，请选择小于 24 MB 的图片。".to_string());
    }

    let file = File::open(canonical).map_err(|_| "无法打开图片文件。".to_string())?;
    let mut data = Vec::with_capacity(metadata.len() as usize);
    file.take(MAX_INPUT_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|_| "读取图片文件失败。".to_string())?;
    if data.len() as u64 > MAX_INPUT_BYTES {
        return Err("图片文件过大，请选择小于 24 MB 的图片。".to_string());
    }
    decode_bytes(&data)
}

#[cfg(windows)]
fn is_network_path(path: &Path) -> bool {
    use std::path::{Component, Prefix};
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;

    let normalized = path.to_string_lossy().to_ascii_lowercase();
    if normalized.starts_with("\\\\?\\unc\\")
        || (normalized.starts_with("\\\\") && !normalized.starts_with("\\\\?\\"))
    {
        return true;
    }

    for component in path.components() {
        let Component::Prefix(prefix) = component else {
            continue;
        };
        let drive = match prefix.kind() {
            Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
            _ => continue,
        };
        let root = [u16::from(drive), u16::from(b':'), u16::from(b'\\'), 0];
        // DRIVE_REMOTE is the Windows value for a mapped network drive.
        if unsafe { GetDriveTypeW(root.as_ptr()) } == 4 {
            return true;
        }
    }
    false
}

#[cfg(not(windows))]
fn is_network_path(_path: &Path) -> bool {
    false
}

fn decode_bytes(data: &[u8]) -> Result<QrResult, String> {
    if data.is_empty() {
        return Err("图片内容为空。".to_string());
    }
    if data.len() as u64 > MAX_INPUT_BYTES {
        return Err("图片过大，请选择小于 24 MB 的图片。".to_string());
    }

    let mut reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|_| "无法识别图片格式。支持 PNG、JPEG、GIF、BMP 和 WebP。".to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_SIDE);
    limits.max_image_height = Some(MAX_IMAGE_SIDE);
    limits.max_alloc = Some(MAX_IMAGE_PIXELS * 8);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| "图片无法解码，请确认文件完整且格式受支持。".to_string())?;
    checked_pixel_count(image.width(), image.height())?;
    decode_dynamic_image(image)
}

fn decode_dynamic_image(image: DynamicImage) -> Result<QrResult, String> {
    checked_pixel_count(image.width(), image.height())?;
    let mut prepared = rqrr::PreparedImage::prepare(image.to_luma8());
    let grids = prepared.detect_grids();
    let mut texts = Vec::new();
    for grid in grids {
        if let Ok((_metadata, text)) = grid.decode() {
            if !text.trim().is_empty() && !texts.iter().any(|existing| existing == &text) {
                texts.push(text);
            }
        }
    }
    if texts.is_empty() {
        return Err("图片中没有可读取的二维码，请调整清晰度或重新截取。".to_string());
    }

    let preview = image.thumbnail(MAX_PREVIEW_SIDE, MAX_PREVIEW_SIDE);
    let mut png = Cursor::new(Vec::new());
    preview
        .write_to(&mut png, ImageFormat::Png)
        .map_err(|_| "生成图片预览失败。".to_string())?;
    let encoded = STANDARD.encode(png.into_inner());
    Ok(QrResult {
        texts,
        image_data_url: format!("data:image/png;base64,{encoded}"),
    })
}

fn checked_pixel_count(width: u32, height: u32) -> Result<u64, String> {
    if width == 0 || height == 0 || width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
        return Err("图片尺寸超出限制，请选择较小的图片。".to_string());
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| "图片尺寸无效。".to_string())?;
    if pixels > MAX_IMAGE_PIXELS {
        return Err("图片分辨率过高，请缩小图片后重试。".to_string());
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_a_generated_qr_image() {
        let expected = "https://example.test/reelink-qr-fixture";
        let generated = qrcode::QrCode::new(expected.as_bytes()).expect("valid QR content");
        let image = generated.render::<image::Luma<u8>>().build();
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(image)
            .write_to(&mut png, ImageFormat::Png)
            .expect("encode QR fixture");

        let result = decode_bytes(png.get_ref()).expect("decode QR fixture");
        assert_eq!(result.texts, vec![expected]);
        assert!(result.image_data_url.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn decodes_multiple_codes_and_deduplicates_repeated_text() {
        let first = qrcode::QrCode::new("重复内容".as_bytes())
            .expect("valid first QR content")
            .render::<image::Luma<u8>>()
            .build();
        let second = qrcode::QrCode::new("第二个二维码".as_bytes())
            .expect("valid second QR content")
            .render::<image::Luma<u8>>()
            .build();
        let width = first.width();
        let height = first.height();
        let mut canvas = ImageBuffer::from_pixel(width * 3 + 64, height, image::Luma([255]));
        image::imageops::overlay(&mut canvas, &first, 0, 0);
        image::imageops::overlay(&mut canvas, &first, i64::from(width + 32), 0);
        image::imageops::overlay(&mut canvas, &second, i64::from((width + 32) * 2), 0);

        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(canvas)
            .write_to(&mut png, ImageFormat::Png)
            .expect("encode multiple QR fixture");
        let result = decode_bytes(png.get_ref()).expect("decode multiple QR fixture");

        assert_eq!(result.texts.len(), 2);
        assert!(result.texts.contains(&"重复内容".to_string()));
        assert!(result.texts.contains(&"第二个二维码".to_string()));
    }

    #[test]
    fn decodes_a_real_local_image_path() {
        let directory = tempfile::tempdir().expect("create temporary directory");
        let path = directory.path().join("fixture.png");
        let png = generated_qr_png("local image fixture");
        std::fs::write(&path, png).expect("write QR fixture");

        let result = decode_path_blocking(path.to_str().expect("UTF-8 temporary path"))
            .expect("decode local QR path");
        assert_eq!(result.texts, vec!["local image fixture"]);
    }

    #[test]
    fn rejects_oversized_input_before_decoding() {
        let data = vec![0; (MAX_INPUT_BYTES + 1) as usize];
        let error = decode_bytes(&data).expect_err("oversized bytes should be rejected");
        assert!(error.contains("过大"));
    }

    fn generated_qr_png(text: &str) -> Vec<u8> {
        let generated = qrcode::QrCode::new(text.as_bytes()).expect("valid QR content");
        let image = generated.render::<image::Luma<u8>>().build();
        let mut png = Cursor::new(Vec::new());
        DynamicImage::ImageLuma8(image)
            .write_to(&mut png, ImageFormat::Png)
            .expect("encode QR fixture");
        png.into_inner()
    }
}

#[cfg(windows)]
fn capture_from_clipboard_blocking() -> Result<QrResult, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::HWND,
        System::DataExchange::GetClipboardSequenceNumber,
        UI::{
            Input::KeyboardAndMouse::{GetAsyncKeyState, VK_ESCAPE},
            Shell::ShellExecuteW,
            WindowsAndMessaging::SW_SHOWNORMAL,
        },
    };

    let mut clipboard =
        arboard::Clipboard::new().map_err(|_| "无法访问系统剪贴板。".to_string())?;
    let previous_sequence = unsafe { GetClipboardSequenceNumber() };
    // Allow the compositor to remove our window before Windows captures the screen.
    std::thread::sleep(Duration::from_millis(180));
    // Clear the old transition bit; a new Escape press cancels this capture.
    unsafe { GetAsyncKeyState(VK_ESCAPE as i32) };

    let verb: Vec<u16> = std::ffi::OsStr::new("open")
        .encode_wide()
        .chain(Some(0))
        .collect();
    let uri: Vec<u16> = std::ffi::OsStr::new("ms-screenclip:")
        .encode_wide()
        .chain(Some(0))
        .collect();
    let launched = unsafe {
        ShellExecuteW(
            std::ptr::null_mut::<std::ffi::c_void>() as HWND,
            verb.as_ptr(),
            uri.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if (launched as isize) <= 32 {
        return Err("无法启动 Windows 截图工具。".to_string());
    }

    let start = Instant::now();
    while start.elapsed() < CAPTURE_TIMEOUT {
        std::thread::sleep(CAPTURE_POLL_INTERVAL);
        if unsafe { GetAsyncKeyState(VK_ESCAPE as i32) } != 0 {
            return Err("已取消屏幕截图。".into());
        }
        if unsafe { GetClipboardSequenceNumber() } != previous_sequence {
            if let Some((width, height, bytes)) = read_clipboard_image(&mut clipboard) {
                let pixels = checked_pixel_count(width, height)?;
                if bytes.len() as u64 != pixels * 4 {
                    return Err("截图图片数据不完整，无法读取。".to_string());
                }
                let buffer = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(width, height, bytes)
                    .ok_or_else(|| "无法读取截图图片。".to_string())?;
                return decode_dynamic_image(DynamicImage::ImageRgba8(buffer));
            }
        }
    }
    Err("截图已取消或等待超时，请重新尝试。".to_string())
}

#[cfg(windows)]
fn read_clipboard_image(clipboard: &mut arboard::Clipboard) -> Option<(u32, u32, Vec<u8>)> {
    let image = clipboard.get_image().ok()?;
    let width = u32::try_from(image.width).ok()?;
    let height = u32::try_from(image.height).ok()?;
    checked_pixel_count(width, height).ok()?;
    Some((width, height, image.bytes.into_owned()))
}
