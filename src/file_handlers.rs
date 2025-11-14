use actix_multipart::Multipart;
use actix_web::Error;
use futures_util::StreamExt as _;
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncWriteExt;

pub async fn save_uploaded_file(
    mut payload: Multipart,
    upload_dir: &str,
) -> Result<Vec<FileInfo>, Error> {
    let mut saved_files = Vec::new();

    while let Some(item) = payload.next().await {
        let mut field = item?;
        let content_type = field.content_disposition().clone();
        let filename = content_type
            .as_ref()
            .and_then(|cd| cd.get_filename())
            .map(|f| f.to_string());
        if let Some(filename_str) = filename.as_ref() {
            let event_id = Path::new(&filename_str)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            let filepath = format!("{}/{}", upload_dir, event_id);
            fs::create_dir_all(&filepath).await?;

            let full_path = format!("{}/{}", filepath, filename_str);
            let mut file = fs::File::create(&full_path).await?;
            let mut file_size = 0;

            while let Some(chunk) = field.next().await {
                let data = chunk?;
                file_size += data.len();
                file.write_all(&data).await?;
            }

            saved_files.push(FileInfo {
                file_name: filename_str.to_string(),
                file_path: full_path,
                file_size,
                mime_type: field.content_type().map(|m| m.to_string()),
                event_id,
            });
        } else {
            continue;
        }
    }

    Ok(saved_files)
}

// src/file_handlers.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub file_name: String,
    pub file_path: String,
    pub file_size: usize,
    pub mime_type: Option<String>,
    pub event_id: String,
    // pub field_name: String,
}

// 为FileInfo添加一些便利方法
impl FileInfo {
    pub fn new(
        file_name: String,
        file_path: String,
        file_size: usize,
        mime_type: Option<String>,
        event_id: String,
        // field_name: String,
    ) -> Self {
        Self {
            file_name,
            file_path,
            file_size,
            mime_type,
            event_id,
            // field_name,
        }
    }

    // 从HashMap创建FileInfo（用于与Python代码兼容）
    pub fn from_hashmap(map: &std::collections::HashMap<String, String>) -> Option<Self> {
        Some(Self {
            file_name: map.get("file_name")?.clone(),
            file_path: map.get("file_path")?.clone(),
            file_size: map.get("file_size")?.parse().ok()?,
            mime_type: map.get("mime_type").cloned(),
            event_id: map.get("event_id")?.clone(),
            // field_name: map.get("field_name")?.clone(),
        })
    }
}
