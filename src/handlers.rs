use actix_web::http::header::{ContentDisposition, DispositionType};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::Path;
use tokio::fs;
use tokio::io::AsyncReadExt;
// src/handlers.rs
use crate::{database, error::AppError, models, vessel_client, websocket};
use actix_web::{web, HttpRequest, HttpResponse, Result};
use log::{info, warn};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;
const FIELDS_TO_REMOVE: &[&str] = &[
    "device_name",
    "device_id",
    "task_name",
    "task_id",
    "app_id",
    "src_name",
    "src_id",
];
// 辅助函数：根据文件名获取文件类型
fn get_file_type(filename: &str) -> String {
    if filename.ends_with(".jpg") || filename.ends_with(".jpeg") {
        "image".to_string()
    } else if filename.ends_with(".png") {
        "image".to_string()
    } else if filename.ends_with(".gif") {
        "image".to_string()
    } else if filename.ends_with(".mp4") || filename.ends_with(".avi") {
        "video".to_string()
    } else if filename.ends_with(".json") {
        "json".to_string()
    } else {
        "unknown".to_string()
    }
}
pub async fn handle_event_upload(
    db: web::Data<database::DatabaseManager>,
    ws_manager: web::Data<Mutex<websocket::EventWebSocketManager>>,
    vessel_manager: web::Data<vessel_client::VesselWebSocketManager>,
    multipart: actix_multipart::Multipart,
) -> Result<HttpResponse> {
    // 处理文件上传
    let saved_files =
        crate::file_handlers::save_uploaded_file(multipart, "./event_uploads").await?;
    // 解析事件数据
    for file in &saved_files {
        if file.file_name.ends_with(".json") {
            if let Ok(content) = tokio::fs::read_to_string(&file.file_path).await {
                if let Ok(event_data) = serde_json::from_str::<models::Event>(&content) {
                    let _ = db.save_event(&event_data).await;
                    // 发送 raw_json 消息
                    let raw_json_message = serde_json::json!({
                        "type": "raw_json",
                        "event_id": &event_data.event_id,
                        "json_data": event_data
                    });
                    if let Ok(message_json) = serde_json::to_string(&raw_json_message) {
                        let server = ws_manager.lock().await;
                        if let Err(e) = server.broadcast_string(&message_json).await {
                            log::error!("广播原始JSON数据失败: {}", e);
                        }
                    }
                    if let Some(task_id) = &event_data.task_id {
                        // 确保设备连接
                        if let Err(e) = vessel_manager.ensure_connection(task_id).await {
                            log::warn!("无法建立设备连接 {}: {}", task_id, e);
                            // 继续处理，但记录警告
                        }
                    }

                    vessel_manager
                        .save_event_with_position(&event_data)
                        .await
                        .map_err(|e| AppError::EventProcessing(e.to_string()))?;
                }
            }
        } else {
            // 对于非JSON文件，发送 event_received 消息
            let simple_notification = serde_json::json!({
                "type": "event_received",
                "event_id": file.event_id,
                "device_name": None::<String>, // 非JSON文件没有这些信息
                "task_name": None::<String>,
                "timestamp": chrono::Utc::now().timestamp(),
                "file_count": 1,
                "file_type": get_file_type(&file.file_name)
            });

            if let Ok(message_json) = serde_json::to_string(&simple_notification) {
                let server = ws_manager.lock().await;

                if let Err(e) = server.broadcast_string(&message_json).await {
                    log::error!("广播简单事件通知失败: {}", e);
                }
            }
        }

        let _ = db.save_event_file(&file.event_id, file).await;
    }

    Ok(HttpResponse::Ok().json(json!({
        "status": "success",
        "message": "Files received and saved to database successfully.",
        "file_count": saved_files.len(),
        "database_saved": true
    })))
}

pub async fn get_events_by_task_id(
    db: web::Data<database::DatabaseManager>,
    path: web::Path<(String,)>,
    query: web::Query<EventQuery>,
) -> Result<HttpResponse, AppError> {
    let task_id = &path.0;
    // 处理查询参数
    let limit = query.limit.unwrap_or(10); // 默认每页10条
    let page = query.page.unwrap_or(1); // 默认第1页
    let name = query.name.as_deref(); // 可选的app_name搜索条件
    let (events, total) = db
        .get_events_by_task_id(task_id, limit, page, name.as_deref())
        .await?;

    // 转换时间格式
    let items: Vec<serde_json::Value> = events
        .into_iter()
        .map(|event| {
            let mut value = serde_json::to_value(&event).unwrap_or_default();

            // 手动处理时间字段
            if let serde_json::Value::Object(ref mut map) = value {
                // 转换 created_at
                if let Some(created_at) = map.get("created_at").and_then(|v| v.as_str()) {
                    map.insert(
                        "created_at".to_string(),
                        serde_json::Value::String(created_at.to_string()),
                    );
                }

                // 转换 updated_at
                if let Some(updated_at) = map.get("updated_at").and_then(|v| v.as_str()) {
                    map.insert(
                        "updated_at".to_string(),
                        serde_json::Value::String(updated_at.to_string()),
                    );
                }

                // 转换 location_created_at
                if let Some(location_created_at) =
                    map.get("location_created_at").and_then(|v| v.as_str())
                {
                    map.insert(
                        "location_created_at".to_string(),
                        serde_json::Value::String(location_created_at.to_string()),
                    );
                }
            }

            value
        })
        .collect();

    Ok(HttpResponse::Ok().json(json!({
        "code": 10001,
        "status": 200,
        "message": "请求成功",
        "data": {
            "items": items,
            "total": total
        }
    })))
}

#[derive(serde::Deserialize)]
pub struct EventQuery {
    pub limit: Option<i32>, // 改为 Option<i32>
    pub page: Option<i32>,  // 改为 Option<i32>
    pub name: Option<String>,
}
#[derive(Deserialize)]
pub struct MultiEventQuery {
    pub event_ids: String, // 逗号分隔的事件ID列表
}
// 多事件文件夹打包下载
pub async fn download_events_package(
    query: web::Query<MultiEventQuery>,
) -> Result<HttpResponse, AppError> {
    let event_ids_str = &query.event_ids;

    // 解析事件ID列表
    let event_ids: Vec<&str> = event_ids_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect();

    if event_ids.is_empty() {
        return Err(AppError::EventProcessing("未提供有效的事件ID".to_string()));
    }

    info!("开始打包多个事件目录: {:?}", event_ids);

    // 安全检查所有事件ID
    for event_id in &event_ids {
        if event_id.contains("..") || event_id.contains('/') || event_id.contains('\\') {
            return Err(AppError::NotFound(format!("无效的事件ID: {}", event_id)));
        }
    }

    // 检查所有目录是否存在
    let mut valid_events = Vec::new();
    for event_id in &event_ids {
        let event_dir = format!("./event_uploads/{}", event_id);
        let dir_path = Path::new(&event_dir);

        if dir_path.exists() && dir_path.is_dir() {
            valid_events.push((event_id.to_string(), event_dir));
        } else {
            warn!("事件目录不存在: {}", event_dir);
        }
    }

    if valid_events.is_empty() {
        return Err(AppError::NotFound("未找到任何有效的事件目录".to_string()));
    }

    info!("找到 {} 个有效事件目录", valid_events.len());

    // 创建ZIP文件
    let zip_data = create_multi_event_zip(&valid_events).await?;

    // 设置响应头
    let filename = if valid_events.len() == 1 {
        format!("{}.zip", valid_events[0].0)
    } else {
        format!("{}_events.zip", valid_events.len())
    };

    let content_disposition = ContentDisposition {
        disposition: DispositionType::Attachment,
        parameters: vec![],
    };

    Ok(HttpResponse::Ok()
        .content_type("application/zip")
        .append_header(("Content-Disposition", content_disposition))
        .append_header(("Content-Type", "application/zip"))
        .append_header(("X-Filename", filename.clone()))
        .append_header(("X-Total-Events", valid_events.len().to_string()))
        .body(zip_data))
}

// 创建多事件ZIP包
async fn create_multi_event_zip(events: &[(String, String)]) -> Result<Vec<u8>, AppError> {
    let mut zip_data = Vec::new();
    let mut zip_writer = ZipWriter::new(std::io::Cursor::new(&mut zip_data));

    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);

    let mut total_files = 0;
    let mut processed_events = 0;

    for (event_id, event_dir) in events {
        // 读取目录中的所有文件
        let mut entries = match fs::read_dir(event_dir).await {
            Ok(entries) => entries,
            Err(e) => {
                warn!("无法读取目录 {}: {}", event_dir, e);
                continue;
            }
        };

        let mut event_file_count = 0;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();

            if path.is_file() {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    if file_name.to_lowercase().ends_with(".json") {
                        match process_file_content(&path, file_name, &FIELDS_TO_REMOVE).await {
                            Ok(processed_content) => {
                                // 在ZIP中创建新文件
                                let zip_path = format!("{}/{}", event_id, file_name);
                                if let Err(e) = zip_writer.start_file(zip_path, options) {
                                    warn!("无法在ZIP中创建文件: {} - {}", file_name, e);
                                    continue;
                                }

                                // 将处理后的内容写入ZIP
                                if let Err(e) = zip_writer.write_all(&processed_content) {
                                    warn!("无法写入处理后的JSON文件到ZIP: {} - {}", file_name, e);
                                    continue;
                                }

                                event_file_count += 1;
                                total_files += 1;
                            }
                            Err(e) => {
                                log::warn!("无法处理JSON文件: {} - {}", file_name, e);
                            }
                        }
                    } else {
                        // // 在ZIP中使用相对路径，包含事件ID作为子目录
                        let zip_path = format!("{}/{}", event_id, file_name);

                        // 开始写入ZIP文件
                        if let Err(e) = zip_writer.start_file(zip_path, options) {
                            warn!("无法添加文件到ZIP: {} - {}", file_name, e);
                            continue;
                        }

                        // 读取文件内容并写入ZIP

                        match fs::File::open(&path).await {
                            Ok(mut file) => {
                                let mut buffer = Vec::new();
                                if let Err(e) = file.read_to_end(&mut buffer).await {
                                    warn!("无法读取文件 {}: {}", path.display(), e);
                                    continue;
                                }

                                if let Err(e) = zip_writer.write_all(&buffer) {
                                    warn!("无法写入ZIP文件 {}: {}", file_name, e);
                                    continue;
                                }

                                event_file_count += 1;
                                total_files += 1;
                            }
                            Err(e) => {
                                warn!("无法打开文件 {}: {}", path.display(), e);
                            }
                        }
                    }
                }
            }
        }

        processed_events += 1;
        info!(
            "事件 {} 处理完成，包含 {} 个文件",
            event_id, event_file_count
        );
    }

    // 完成ZIP写入
    zip_writer.finish()?;

    info!(
        "成功创建多事件ZIP包，包含 {} 个事件的 {} 个文件",
        processed_events, total_files
    );

    Ok(zip_data)
}

// 处理文件内容 - 对JSON文件进行特殊处理
async fn process_file_content(
    file_path: &Path,
    file_name: &str,
    fields_to_remove: &[&str],
) -> Result<Vec<u8>, AppError> {
    let mut file = fs::File::open(file_path).await?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).await?;

    // 如果是JSON文件，进行特殊处理
    if file_name.to_lowercase().ends_with(".json") {
        // 解析JSON
        let json_content = String::from_utf8(buffer)
            .map_err(|e| AppError::EventProcessing(format!("无效的UTF-8内容: {}", e)))?;

        let mut json_value: Value = serde_json::from_str(&json_content)
            .map_err(|e| AppError::EventProcessing(format!("无效的JSON格式: {}", e)))?;

        // 移除指定字段
        remove_json_fields(&mut json_value, fields_to_remove);

        // 重新序列化为JSON字符串
        let processed_json = serde_json::to_string_pretty(&json_value)
            .map_err(|e| AppError::EventProcessing(format!("JSON序列化失败: {}", e)))?;
        Ok(processed_json.into_bytes())
    } else {
        // 非JSON文件，直接返回原始内容
        Ok(buffer)
    }
}

// 递归移除JSON中的指定字段
fn remove_json_fields(value: &mut Value, fields_to_remove: &[&str]) {
    match value {
        Value::Object(obj) => {
            // 移除当前层级的指定字段
            for field in fields_to_remove {
                obj.remove(*field);
            }

            // 递归处理所有子对象
            for (_, child_value) in obj.iter_mut() {
                remove_json_fields(child_value, fields_to_remove);
            }
        }
        Value::Array(arr) => {
            // 处理数组中的每个元素
            for item in arr.iter_mut() {
                remove_json_fields(item, fields_to_remove);
            }
        }
        _ => {} // 其他类型不需要处理
    }
}

// 流式打包下载
pub async fn download_event_package_stream(
    path: web::Path<String>,
    req: HttpRequest,
) -> Result<HttpResponse, AppError> {
    let event_id = path.into_inner();

    // 安全检查
    if event_id.contains("..") || event_id.contains('/') || event_id.contains('\\') {
        return Err(AppError::NotFound("Invalid event ID".to_string()));
    }

    let event_dir = format!("./event_uploads/{}", event_id);
    let dir_path = Path::new(&event_dir);

    if !dir_path.exists() || !dir_path.is_dir() {
        return Err(AppError::NotFound(format!(
            "Event directory not found: {}",
            event_id
        )));
    }

    log::info!("开始流式打包事件目录: {}", event_dir);

    // 创建临时ZIP文件
    let temp_dir = std::env::temp_dir();
    let zip_path = temp_dir.join(format!("{}.zip", event_id));

    create_event_zip_to_file(&event_dir, &event_id, &zip_path).await?;

    // 读取ZIP文件并创建流式响应
    let file = actix_files::NamedFile::open(&zip_path)?;
    let response = file
        .set_content_type("application/zip".parse().unwrap())
        .set_content_disposition(actix_web::http::header::ContentDisposition {
            disposition: actix_web::http::header::DispositionType::Attachment,
            parameters: vec![],
        })
        .into_response(&req);

    // 异步清理临时文件
    let zip_path_clone = zip_path.clone();
    tokio::spawn(async move {
        tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
        let _ = tokio::fs::remove_file(zip_path_clone).await;
    });

    Ok(response)
}

async fn create_event_zip_to_file(
    event_dir: &str,
    event_id: &str,
    output_path: &Path,
) -> Result<(), AppError> {
    let file = std::fs::File::create(output_path)?;
    let mut zip_writer = ZipWriter::new(file);

    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut entries = fs::read_dir(event_dir).await?;
    let mut file_count = 0;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();

        if path.is_file() {
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                if file_name.to_lowercase().ends_with(".json") {
                    match process_file_content(&path, file_name, FIELDS_TO_REMOVE).await {
                        Ok(processed_content) => {
                            // 在ZIP中创建新文件
                            let zip_path = format!("{}/{}", event_id, file_name);
                            if let Err(e) = zip_writer.start_file(zip_path, options) {
                                warn!("无法在ZIP中创建文件: {} - {}", file_name, e);
                                continue;
                            }

                            // 将处理后的内容写入ZIP
                            if let Err(e) = zip_writer.write_all(&processed_content) {
                                warn!("无法写入处理后的JSON文件到ZIP: {} - {}", file_name, e);
                                continue;
                            }
                        }
                        Err(e) => {
                            log::warn!("无法处理JSON文件: {} - {}", file_name, e);
                        }
                    }
                } else {
                    // // 在ZIP中使用相对路径，包含事件ID作为子目录
                    let zip_path = format!("{}/{}", event_id, file_name);

                    // 开始写入ZIP文件
                    if let Err(e) = zip_writer.start_file(zip_path, options) {
                        warn!("无法添加文件到ZIP: {} - {}", file_name, e);
                        continue;
                    }

                    // 读取文件内容并写入ZIP

                    match fs::File::open(&path).await {
                        Ok(mut file) => {
                            let mut buffer = Vec::new();
                            if let Err(e) = file.read_to_end(&mut buffer).await {
                                warn!("无法读取文件 {}: {}", path.display(), e);
                                continue;
                            }

                            if let Err(e) = zip_writer.write_all(&buffer) {
                                warn!("无法写入ZIP文件 {}: {}", file_name, e);
                                continue;
                            }
                        }
                        Err(e) => {
                            warn!("无法打开文件 {}: {}", path.display(), e);
                        }
                    }
                }
            }
        }
    }

    zip_writer.finish()?;
    info!("成功创建ZIP文件，包含 {} 个文件", file_count);

    Ok(())
}
