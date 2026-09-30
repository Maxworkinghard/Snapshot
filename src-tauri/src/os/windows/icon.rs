//! Windows 可执行文件图标提取，用于窗口列表和上一应用显示。

use image::{Rgba, RgbaImage};
use std::{
    ffi::c_void,
    mem::{size_of, zeroed},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    sync::mpsc::{self, Sender},
    thread,
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, SelectObject, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    },
    System::{
        Com::{CoInitializeEx, COINIT_APARTMENTTHREADED},
        Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION},
    },
    UI::{
        Shell::{SHDefExtractIconW, SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON},
        WindowsAndMessaging::{DestroyIcon, DrawIconEx, PrivateExtractIconsW, DI_NORMAL, HICON},
    },
};

/// 进程图标，编码成 PNG（媒体协议 icon/<pid>/<边长> 用）。
pub(crate) fn app_icon_png(pid: u32, size: u32) -> Option<Vec<u8>> {
    crate::capture::encode_png(&icon_for_process(pid, size)?).ok()
}

/// `size` 是想要的像素边长：窗口列表只显示 24px，桌宠最大 68px，没必要按 256px 取。
fn icon_for_process(pid: u32, size: u32) -> Option<RgbaImage> {
    let size = size as i32;
    unsafe {
        let process: HANDLE = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return None;
        }
        let mut path = vec![0u16; 32768];
        let mut len = path.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut len);
        CloseHandle(process);
        if ok == 0 {
            return None;
        }
        path.truncate(len as usize);
        path.push(0);

        let mut extracted_icon = null_mut();
        let mut icon_id = 0u32;
        let extracted = PrivateExtractIconsW(
            path.as_ptr(),
            0,
            size,
            size,
            &mut extracted_icon,
            &mut icon_id,
            1,
            0,
        );
        let icon = if extracted > 0 && extracted != u32::MAX && !extracted_icon.is_null() {
            extracted_icon
        } else {
            let mut info: SHFILEINFOW = zeroed();
            let result = SHGetFileInfoW(
                path.as_ptr(),
                0,
                &mut info,
                size_of::<SHFILEINFOW>() as u32,
                SHGFI_ICON | SHGFI_LARGEICON,
            );
            if result == 0 || info.hIcon.is_null() {
                return None;
            }
            info.hIcon
        };

        rasterize_icon(icon, size)
    }
}

struct IconRequest {
    path: PathBuf,
    size: i32,
    reply: Sender<Option<RgbaImage>>,
}

/// 壳层图标不能从线程池并行取：调用线程得一直是公寓模型，而且不能同时取。
fn icon_requests() -> Sender<IconRequest> {
    static WORKER: std::sync::OnceLock<Sender<IconRequest>> = std::sync::OnceLock::new();
    WORKER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<IconRequest>();
            thread::Builder::new()
                .name("shortcut-icon".into())
                .spawn(move || {
                    unsafe {
                        let _ = CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
                    }
                    while let Ok(job) = rx.recv() {
                        let (source, index) = shortcut_icon_source(&job.path)
                            .filter(|(candidate, _)| candidate.is_file())
                            .unwrap_or((job.path.clone(), 0));
                        let mut wide: Vec<u16> = source.as_os_str().encode_wide().collect();
                        wide.push(0);
                        let image = unsafe { hicon_for_path(wide.as_ptr(), index, job.size) }
                            .and_then(|icon| rasterize_icon(icon, job.size));
                        let _ = job.reply.send(image);
                    }
                })
                .expect("shortcut icon thread");
            tx
        })
        .clone()
}

/// 启动台要用快捷方式自己的图标，不经过进程路径。
pub(crate) fn png_for_path(path: &Path, size: u32) -> Option<Vec<u8>> {
    let (reply, rx) = mpsc::channel();
    icon_requests()
        .send(IconRequest {
            path: path.to_path_buf(),
            size: size.clamp(16, 256) as i32,
            reply,
        })
        .ok()?;
    crate::capture::encode_png(&rx.recv().ok()??).ok()
}

/// 快捷方式文件上的图标自带一个小箭头。能解析出真正的图标文件时，用那个，箭头就没了。
fn shortcut_icon_source(path: &Path) -> Option<(PathBuf, i32)> {
    let bytes = std::fs::read(path).ok()?;
    let (raw, index) = parse_shortcut_icon(&bytes)?;
    let expanded = expand_shortcut_path(&raw);
    if expanded.as_os_str().is_empty() {
        None
    } else {
        Some((expanded, index))
    }
}

/// [Shell Link 二进制格式](https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-shllink/) 里的图标路径和索引。
fn parse_shortcut_icon(bytes: &[u8]) -> Option<(String, i32)> {
    if bytes.len() < 0x4c || u32::from_le_bytes(bytes[0..4].try_into().ok()?) < 0x4c {
        return None;
    }
    let flags = u32::from_le_bytes(bytes[0x14..0x18].try_into().ok()?);
    let icon_index = i32::from_le_bytes(bytes[0x38..0x3c].try_into().ok()?);
    let mut offset = 0x4c;
    if flags & 0x01 != 0 {
        let size = u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?) as usize;
        offset = offset.checked_add(2 + size)?;
    }
    if flags & 0x02 != 0 {
        let size = u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?) as usize;
        if size < 4 {
            return None;
        }
        let info = bytes.get(offset..offset + size)?;
        if flags & 0x40 == 0 {
            if let Some(target) = link_info_target(info) {
                return Some((target, icon_index));
            }
        }
        offset = offset.checked_add(size)?;
    }
    for bit in [0x04u32, 0x08, 0x10, 0x20, 0x40] {
        if flags & bit == 0 {
            continue;
        }
        let count = u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?) as usize;
        offset = offset.checked_add(2)?;
        let end = offset.checked_add(count.checked_mul(2)?)?;
        let chars = bytes.get(offset..end)?;
        if bit == 0x40 {
            let units: Vec<u16> = chars
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .collect();
            let text = String::from_utf16_lossy(&units)
                .trim_matches('\0')
                .trim()
                .to_string();
            if text.is_empty() {
                return None;
            }
            return Some((text, icon_index));
        }
        offset = end;
    }
    None
}

fn link_info_target(info: &[u8]) -> Option<String> {
    if info.len() < 0x1c {
        return None;
    }
    let header = u32::from_le_bytes(info[4..8].try_into().ok()?) as usize;
    let flags = u32::from_le_bytes(info[8..12].try_into().ok()?);
    if flags & 1 == 0 {
        return None;
    }
    let offset = if header >= 0x24 && info.len() >= 0x24 {
        u32::from_le_bytes(info[28..32].try_into().ok()?) as usize
    } else {
        0
    };
    if offset > 0 {
        return utf16z(info, offset);
    }
    let local = u32::from_le_bytes(info[16..20].try_into().ok()?) as usize;
    let end = info.get(local..)?.iter().position(|byte| *byte == 0)?;
    let text = String::from_utf8_lossy(&info[local..local + end])
        .trim()
        .to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn utf16z(bytes: &[u8], offset: usize) -> Option<String> {
    let mut units = Vec::new();
    let mut index = offset;
    while index + 1 < bytes.len() {
        let unit = u16::from_le_bytes(bytes[index..index + 2].try_into().ok()?);
        if unit == 0 {
            break;
        }
        units.push(unit);
        index += 2;
    }
    let text = String::from_utf16_lossy(&units).trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn expand_shortcut_path(raw: &str) -> PathBuf {
    let mut out = String::new();
    let mut rest = raw;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        if let Some(end) = after.find('%') {
            if let Ok(value) = std::env::var(&after[..end]) {
                out.push_str(&value);
                rest = &after[end + 1..];
                continue;
            }
        }
        out.push('%');
        rest = after;
    }
    out.push_str(rest);
    PathBuf::from(out.trim())
}

unsafe fn hicon_for_path(path: *const u16, index: i32, size: i32) -> Option<HICON> {
    let mut large = null_mut();
    let extracted = SHDefExtractIconW(path, index, 0, &mut large, null_mut(), size as u32);
    if extracted == 0 && !large.is_null() {
        return Some(large);
    }
    if !large.is_null() {
        DestroyIcon(large);
    }
    let mut info: SHFILEINFOW = zeroed();
    let result = SHGetFileInfoW(
        path,
        0,
        &mut info,
        size_of::<SHFILEINFOW>() as u32,
        SHGFI_ICON | SHGFI_LARGEICON,
    );
    if result == 0 || info.hIcon.is_null() {
        None
    } else {
        Some(info.hIcon)
    }
}

fn rasterize_icon(icon: HICON, size: i32) -> Option<RgbaImage> {
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            DestroyIcon(icon);
            return None;
        }
        let mut bitmap_info: BITMAPINFO = zeroed();
        bitmap_info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bitmap_info.bmiHeader.biWidth = size;
        bitmap_info.bmiHeader.biHeight = -size;
        bitmap_info.bmiHeader.biPlanes = 1;
        bitmap_info.bmiHeader.biBitCount = 32;
        bitmap_info.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut c_void = null_mut();
        let bitmap = CreateDIBSection(dc, &bitmap_info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() || bits.is_null() {
            DeleteDC(dc);
            DestroyIcon(icon);
            return None;
        }
        let old = SelectObject(dc, bitmap);
        let _ = DrawIconEx(dc, 0, 0, icon, size, size, 0, null_mut(), DI_NORMAL);
        let raw = std::slice::from_raw_parts(bits as *const u8, (size * size * 4) as usize);
        let mut image = RgbaImage::new(size as u32, size as u32);
        let pixels = raw.as_chunks::<4>().0;
        let has_alpha = pixels.iter().any(|pixel| pixel[3] != 0);
        for (index, pixel) in pixels.iter().enumerate() {
            let alpha = if has_alpha {
                pixel[3]
            } else if pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0 {
                0
            } else {
                255
            };
            let x = (index as u32) % size as u32;
            let y = (index as u32) / size as u32;
            image.put_pixel(x, y, Rgba([pixel[2], pixel[1], pixel[0], alpha]));
        }
        SelectObject(dc, old);
        DeleteObject(bitmap);
        DeleteDC(dc);
        DestroyIcon(icon);
        Some(image)
    }
}

#[cfg(test)]
mod tests {
    use super::{expand_shortcut_path, parse_shortcut_icon};

    fn header(flags: u32, index: i32) -> Vec<u8> {
        let mut bytes = vec![0u8; 0x4c];
        bytes[0..4].copy_from_slice(&0x4cu32.to_le_bytes());
        bytes[0x14..0x18].copy_from_slice(&flags.to_le_bytes());
        bytes[0x38..0x3c].copy_from_slice(&index.to_le_bytes());
        bytes
    }

    fn push_utf16(bytes: &mut Vec<u8>, text: &str) {
        let units: Vec<u16> = text.encode_utf16().collect();
        bytes.extend_from_slice(&(units.len() as u16).to_le_bytes());
        for unit in units {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
    }

    #[test]
    fn icon_location_keeps_its_index() {
        let mut bytes = header(0x40, 3);
        push_utf16(&mut bytes, r"C:\Windows\System32\shell32.dll");
        let (path, index) = parse_shortcut_icon(&bytes).unwrap();
        assert_eq!(path, r"C:\Windows\System32\shell32.dll");
        assert_eq!(index, 3);
    }

    #[test]
    fn name_comes_before_the_icon_path() {
        let mut bytes = header(0x04 | 0x40, 0);
        push_utf16(&mut bytes, "Blender");
        push_utf16(&mut bytes, r"%SystemRoot%\system32\imageres.dll");
        let (path, _) = parse_shortcut_icon(&bytes).unwrap();
        assert_eq!(path, r"%SystemRoot%\system32\imageres.dll");
        let expanded = expand_shortcut_path(&path);
        assert!(expanded.ends_with(r"system32\imageres.dll"));
    }

    #[test]
    fn installed_blender_shortcut_points_at_its_exe() {
        let Some(data) = dirs::data_dir() else {
            return;
        };
        let path = data.join(r"Microsoft\Windows\Start Menu\Programs\Blender\Blender 5.2.lnk");
        if !path.exists() {
            return;
        }
        let (icon, _) = super::shortcut_icon_source(&path).expect("解析快捷方式图标");
        assert!(icon.is_file(), "{}", icon.display());
    }
}
