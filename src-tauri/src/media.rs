//! 应用自己的媒体协议 `media://`：页面里的 `<img>` / `<audio>` 直接向后端要文件字节。
//!
//! 原先是把文件读出来转成 base64 字符串，再经 IPC 塞给页面：历史页一打开就把每张快照的
//! 原图全转一遍（上限 200 张，全攥在页面内存里），桌宠每换一个动作就重新解一次压缩包、
//! 重新编码一遍。走协议后，字节直接进 WebView 的图片管线，懒加载、缓存、解码都交给浏览器。
//!
//! 只认下面几种路径，每种都先在设置 / 索引里按 id 查到真实文件，页面拿不到任意文件的读权限：
//!   snapshot/<快照 id>
//!   thumb/<快照 id>              缩略图（没有就现做）
//!   pet/<形象 id>[/<压缩包内的动作路径>]
//!   pet-walk/<形象 id>/<动作路径>  拖动时播的走路动作（横穿画布的改成原地走）
//!   pet-thumb/<形象 id>          默认动作的第一帧
//!   icon/<pid>/<边长>
//!   sound/<设置里选定的自定义音效路径>

use super::*;
use percent_encoding::percent_decode_str;
use tauri::http::{header, Request, Response, StatusCode};

pub(crate) const SCHEME: &str = "media";

struct Media {
    mime: &'static str,
    /// 内容永不变（快照文件按 id 落盘后不再改写），可以让 WebView 长期缓存
    immutable: bool,
    bytes: Vec<u8>,
}

pub(crate) fn handle(
    context: tauri::UriSchemeContext<'_, tauri::Wry>,
    request: Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    let app = context.app_handle().clone();
    // 读盘、解压、取图标都会阻塞，挪到阻塞线程池，别占住 WebView 发请求的线程
    tauri::async_runtime::spawn_blocking(move || {
        let path = percent_decode_str(request.uri().path().trim_start_matches('/'))
            .decode_utf8_lossy()
            .into_owned();
        responder.respond(into_response(serve(&app, &path)));
    });
}

fn into_response(result: Result<Media, String>) -> Response<Vec<u8>> {
    let builder = Response::builder();
    let built = match result {
        Ok(media) => builder
            .header(header::CONTENT_TYPE, media.mime)
            .header(
                header::CACHE_CONTROL,
                if media.immutable {
                    "max-age=31536000, immutable"
                } else {
                    "no-cache"
                },
            )
            .body(media.bytes),
        Err(message) => builder
            .status(StatusCode::NOT_FOUND)
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(message.into_bytes()),
    };
    built.unwrap_or_default()
}

fn serve(app: &AppHandle, path: &str) -> Result<Media, String> {
    let state = app.state::<AppState>();
    let (kind, rest) = path.split_once('/').ok_or("资源路径不完整")?;
    match kind {
        "snapshot" => {
            let (mime, bytes) = snapshots::read_snapshot(&state, rest)?;
            Ok(Media {
                mime,
                immutable: true,
                bytes,
            })
        }
        "thumb" => Ok(Media {
            mime: "image/jpeg",
            immutable: true,
            bytes: snapshots::read_thumbnail(&state, rest)?,
        }),
        "pet-thumb" => Ok(Media {
            mime: "image/png",
            immutable: false,
            bytes: pet::read_thumbnail(&state, rest)?,
        }),
        "pet" => {
            let (id, entry) = match rest.split_once('/') {
                Some((id, entry)) => (id, Some(entry)),
                None => (rest, None),
            };
            Ok(Media {
                mime: "image/gif",
                immutable: false,
                bytes: pet::read_animation(&state, id, entry)?,
            })
        }
        "pet-walk" => {
            let (id, entry) = rest.split_once('/').ok_or("走路动作路径不完整")?;
            Ok(Media {
                mime: "image/gif",
                immutable: false,
                bytes: pet::read_walk(&state, id, entry)?,
            })
        }
        "launch-icon" => {
            let (id, size) = rest.rsplit_once('/').ok_or("图标路径不完整")?;
            let size = size
                .parse::<u32>()
                .map_err(|_| "图标尺寸无效".to_string())?
                .clamp(16, 256);
            Ok(Media {
                mime: "image/png",
                immutable: false,
                bytes: launchpad::icon_bytes(&state, id, size)?,
            })
        }
        "icon" => {
            let (pid, size) = rest.split_once('/').ok_or("图标路径不完整")?;
            let pid = pid.parse().map_err(|_| "pid 无效".to_string())?;
            let size = size
                .parse::<u32>()
                .map_err(|_| "图标尺寸无效".to_string())?
                .clamp(16, 256);
            let bytes = os::app_icon_png(pid, size).ok_or("这个进程没有可用的图标")?;
            Ok(Media {
                mime: "image/png",
                immutable: false,
                bytes,
            })
        }
        "sound" => {
            // 只放行设置里选定的那一个文件，不能借这个口子读别的路径
            let configured = state.settings.lock().custom_sound_path.clone();
            if configured.as_deref() != Some(rest) {
                return Err("只能读取设置里选定的音效文件".into());
            }
            let bytes = fs::read(rest).map_err(|_| "音效文件已被移动或删除".to_string())?;
            Ok(Media {
                mime: audio_mime(rest),
                immutable: false,
                bytes,
            })
        }
        _ => Err(format!("未知的资源类型：{kind}")),
    }
}

fn audio_mime(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".mp3") {
        "audio/mpeg"
    } else if lower.ends_with(".wav") {
        "audio/wav"
    } else if lower.ends_with(".ogg") {
        "audio/ogg"
    } else if lower.ends_with(".m4a") {
        "audio/mp4"
    } else {
        "application/octet-stream"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_types_follow_the_import_filter() {
        // 与设置页「导入音效」的扩展名过滤一一对应
        assert_eq!(audio_mime("C:/a/Shutter.MP3"), "audio/mpeg");
        assert_eq!(audio_mime("/a/b.wav"), "audio/wav");
        assert_eq!(audio_mime("/a/b.ogg"), "audio/ogg");
        assert_eq!(audio_mime("/a/b.m4a"), "audio/mp4");
    }

    #[test]
    fn failures_become_404_with_a_readable_reason() {
        let response = into_response(Err("找不到这条快照".into()));
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            String::from_utf8(response.body().clone()).unwrap(),
            "找不到这条快照"
        );
    }

    #[test]
    fn only_snapshots_are_cached_long_term() {
        let snapshot = into_response(Ok(Media {
            mime: "image/png",
            immutable: true,
            bytes: vec![1],
        }));
        assert_eq!(
            snapshot.headers()[header::CACHE_CONTROL],
            "max-age=31536000, immutable"
        );
        let pet = into_response(Ok(Media {
            mime: "image/gif",
            immutable: false,
            bytes: vec![1],
        }));
        assert_eq!(pet.headers()[header::CACHE_CONTROL], "no-cache");
    }
}
