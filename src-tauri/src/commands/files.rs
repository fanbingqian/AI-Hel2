use std::path::PathBuf;
use tauri::Emitter;

#[tauri::command]
pub async fn read_text_file(path: String) -> Result<String, String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err("文件不存在".into());
    }
    if !p.is_file() {
        return Err("路径不是文件".into());
    }
    std::fs::read_to_string(&p).map_err(|e| format!("读取文件失败: {}", e))
}

#[tauri::command]
pub async fn read_file_base64(path: String) -> Result<String, String> {
    use base64::Engine;
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err("文件不存在".into());
    }
    let buf = std::fs::read(&p).map_err(|e| format!("读取文件失败: {e}"))?;
    let mime = mime_guess::from_path(&p).first_or_octet_stream();
    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf);
    Ok(format!("data:{};base64,{}", mime, encoded))
}

#[tauri::command]
pub async fn save_avatar(app: tauri::AppHandle, base64_data: String) -> Result<(), String> {
    use base64::Engine;
    use crate::services::config_service::ConfigService;
    let cs = ConfigService::new();
    let avatar_path = cs.hermes_home().join("avatar");
    // Decode base64 to raw bytes
    if let Some(comma) = base64_data.find(',') {
        let raw = &base64_data[comma + 1..];
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(raw)
            .map_err(|e| format!("Base64 decode failed: {e}"))?;
        std::fs::write(&avatar_path, &bytes)
            .map_err(|e| format!("写入头像文件失败: {e}"))?;
    } else {
        return Err("Invalid base64 data URL".into());
    }
    // Notify frontend to refresh avatar
    let _ = app.emit("user:avatar-updated", &base64_data);
    Ok(())
}

#[tauri::command]
pub async fn get_avatar() -> Result<String, String> {
    use base64::Engine;
    use crate::services::config_service::ConfigService;
    let cs = ConfigService::new();
    let avatar_path = cs.hermes_home().join("avatar");
    if !avatar_path.exists() {
        return Ok(String::new());
    }
    let buf = std::fs::read(&avatar_path)
        .map_err(|e| format!("读取头像文件失败: {e}"))?;
    let mime = mime_guess::from_path(&avatar_path).first_or_octet_stream();
    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf);
    Ok(format!("data:{};base64,{}", mime, encoded))
}

#[tauri::command]
pub async fn capture_screen() -> Result<String, String> {
    use base64::Engine;
    use screenshots::Screen;
    use screenshots::image::{DynamicImage, ImageFormat};
    use std::io::Cursor;
    let screens = Screen::all().map_err(|e| format!("获取屏幕失败: {}", e))?;
    let screen = screens.first().ok_or("未检测到屏幕")?;
    let image = screen.capture().map_err(|e| format!("截屏失败: {}", e))?;
    let dynamic = DynamicImage::ImageRgba8(image);
    let mut buf = Vec::new();
    dynamic.write_to(&mut Cursor::new(&mut buf), ImageFormat::Png)
        .map_err(|e| format!("编码失败: {}", e))?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&buf);
    Ok(format!("data:image/png;base64,{}", encoded))
}
