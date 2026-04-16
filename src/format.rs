use serde_json::Value;
use std::path::PathBuf;
use base64::{Engine as _, engine::general_purpose::STANDARD};

pub fn process_files_in_json(json_val: &mut Value, root_dir: &str, trigger: &str) {
    match json_val {
        Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                process_files_in_json(v, root_dir, trigger);
            }
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                process_files_in_json(v, root_dir, trigger);
            }
        }
        Value::String(s) => {
            if s.starts_with(trigger) {
                let mut path = PathBuf::from(root_dir);
                let clean_s = s.trim_start_matches('/');
                path.push(clean_s);
                
                if let Ok(bytes) = std::fs::read(&path) {
                    let b64 = STANDARD.encode(&bytes);
                    let ext = path.extension().and_then(|ex| ex.to_str()).unwrap_or("");
                    let mime = match ext.to_lowercase().as_str() {
                        "jpg" | "jpeg" => "image/jpeg",
                        "png" => "image/png",
                        "gif" => "image/gif",
                        "webp" => "image/webp",
                        "pdf" => "application/pdf",
                        "svg" => "image/svg+xml",
                        "mp4" => "video/mp4",
                        _ => "application/octet-stream",
                    };
                    *s = format!("data:{};base64,{}", mime, b64);
                }
            }
        }
        _ => {}
    }
}
