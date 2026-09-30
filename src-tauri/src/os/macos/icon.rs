//! macOS：NSRunningApplication.icon → 指定边长的 PNG。
//! 拿到的直接就是 PNG 字节，不必像 Windows 那样走 RgbaImage。

use objc2::AnyThread;
use objc2_app_kit::{
    NSBitmapImageFileType, NSBitmapImageRep, NSDeviceRGBColorSpace, NSGraphicsContext, NSImage,
    NSRunningApplication,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize};

pub fn png_for_pid(pid: u32, size: u32) -> Option<Vec<u8>> {
    unsafe {
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(pid as i32)?;
        let icon = app.icon()?;
        downscale_to_png(&icon, size)
    }
}

pub(crate) unsafe fn png_of(icon: &NSImage, size: u32) -> Option<Vec<u8>> {
    downscale_to_png(icon, size)
}

/// 图标的 TIFF 里带全套尺寸（最大 1024×1024）；setSize 只改逻辑尺寸、
/// 动不了 TIFFRepresentation 里的像素。要真压到目标边长，得把它画进
/// 一个新的同尺寸 bitmap rep 再导出。
unsafe fn downscale_to_png(icon: &NSImage, size: u32) -> Option<Vec<u8>> {
    let rep = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
        NSBitmapImageRep::alloc(),
        std::ptr::null_mut(),
        size as isize,
        size as isize,
        8,
        4,
        true,
        false,
        NSDeviceRGBColorSpace,
        0,
        0,
    )?;
    let context = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&context));
    let edge = f64::from(size);
    icon.drawInRect(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(edge, edge)));
    NSGraphicsContext::restoreGraphicsState_class();
    let png =
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())?;
    Some(png.to_vec())
}

#[cfg(test)]
mod tests {
    #[test]
    fn finder_icon_encodes_as_64px_png() {
        let Ok(output) = std::process::Command::new("pgrep")
            .args(["-x", "Finder"])
            .output()
        else {
            return;
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let Some(pid) = text
            .lines()
            .next()
            .and_then(|line| line.trim().parse::<u32>().ok())
        else {
            return;
        };
        let png = super::png_for_pid(pid, 64).expect("Finder 应当能取到图标");
        // 图标 TIFF 里有 1024×1024 原图；不真压尺寸的话这里会得到六百 KB 的大图
        let decoded = image::load_from_memory(&png).expect("导出的应为合法 PNG");
        assert_eq!(
            (decoded.width(), decoded.height()),
            (64, 64),
            "图标应压到 64×64"
        );
        assert!(
            png.len() < 20 * 1024,
            "64×64 图标 PNG 应远小于 20KB，实际 {} 字节",
            png.len()
        );
        // drawInRect 必须真的把图标画进去，而不是导出一张空白图
        let opaque = decoded
            .to_rgba8()
            .pixels()
            .filter(|pixel| pixel[3] > 0)
            .count();
        assert!(
            opaque > 64,
            "64×64 图标应有可见内容，实际只有 {opaque} 个非透明像素"
        );
    }
}
