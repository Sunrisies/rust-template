use crate::{
    error::AppError,
    file_handlers::FileInfo,
    models::{Event, EventWithLocation},
};
use chrono::{DateTime, Utc};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{MySql, MySqlPool, Pool};
use std::collections::HashMap;
#[derive(Clone)]
pub struct DatabaseManager {
    pool: Pool<MySql>,
}
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Device {
    pub id: i64,
    pub device_id: String,
    pub device_name: Option<String>,
    pub device_type: Option<String>,
    pub vessel_tcp_host: Option<String>,
    pub vessel_tcp_port: Option<i32>,
    pub vessel_name: Option<String>,
    pub vessel_id: Option<String>,
    pub status: i8,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub parameters: Option<String>,
}
// 扩展事件记录结构体以包含文件链接
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventWithFiles {
    pub id: i64,
    pub event_id: Option<String>,
    pub event_state: Option<i32>,
    pub device_name: Option<String>,
    pub device_id: Option<String>,
    pub task_name: Option<String>,
    pub task_id: Option<String>,
    pub app_name: Option<String>,
    pub app_id: Option<String>,
    pub src_name: Option<String>,
    pub src_id: Option<String>,
    pub details: Option<String>,
    pub raw_json: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub location_created_at: Option<chrono::DateTime<chrono::Utc>>,
    pub preview_url: Option<String>,
    pub meta_url: Option<String>,
}

// 事件文件结构体
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EventFile {
    pub id: i64,
    pub event_id: Option<String>,
    pub file_name: String,
    pub file_path: String,
    pub file_size: i64,
    pub mime_type: Option<String>,
}
impl DatabaseManager {
    pub async fn new(config: &DatabaseConfig) -> Result<Self, sqlx::Error> {
        let url = format!(
            "mysql://{}:{}@{}:{}/{}",
            config.user, config.password, config.host, config.port, config.database
        );

        let pool = MySqlPool::connect(&url).await?;
        Ok(Self { pool })
    }

    pub async fn save_event(&self, event_data: &Event) -> Result<bool, sqlx::Error> {
        let raw_json = serde_json::to_string(event_data).unwrap();
        let details = event_data
            .details
            .as_ref()
            .and_then(|d| serde_json::to_string(d).ok());

        let query = r#"
            INSERT INTO events (
                event_id, event_state, device_name, device_id, 
                task_name, task_id, app_name, app_id, 
                src_name, src_id, details, raw_json, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, NOW(), NOW())
            ON DUPLICATE KEY UPDATE
                event_state = VALUES(event_state),
                device_name = VALUES(device_name),
                device_id = VALUES(device_id),
                task_name = VALUES(task_name),
                task_id = VALUES(task_id),
                app_name = VALUES(app_name),
                app_id = VALUES(app_id),
                src_name = VALUES(src_name),
                src_id = VALUES(src_id),
                details = VALUES(details),
                raw_json = VALUES(raw_json),
                updated_at = NOW()
        "#;

        sqlx::query(query)
            .bind(&event_data.event_id)
            .bind(&event_data.event_state)
            .bind(&event_data.device_name)
            .bind(&event_data.device_id)
            .bind(&event_data.task_name)
            .bind(&event_data.task_id)
            .bind(&event_data.app_name)
            .bind(&event_data.app_id)
            .bind(&event_data.src_name)
            .bind(&event_data.src_id)
            .bind(&details)
            .bind(&raw_json)
            .execute(&self.pool)
            .await?;
        log::info!(
            "事件数据保存成功: {}",
            event_data.event_id.as_deref().unwrap_or("unknown")
        );
        Ok(true)
    }

    pub async fn save_event_file(
        &self,
        event_id: &str,
        file_info: &FileInfo,
    ) -> Result<bool, AppError> {
        // 获取当前时间戳（秒）
        let timestamp = Utc::now().timestamp() as f64;

        let query = r#"
            INSERT INTO event_files (event_id, file_name, file_path, file_size, mime_type, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
        "#;

        sqlx::query(query)
            .bind(event_id)
            .bind(&file_info.file_name)
            .bind(&file_info.file_path)
            .bind(file_info.file_size as i64)
            .bind(&file_info.mime_type)
            .bind(timestamp)
            .execute(&self.pool)
            .await?;

        log::info!("文件信息保存成功: {}", file_info.file_name);
        Ok(true)
    }

    pub async fn get_events_by_task_id(
        &self,
        task_id: &str,
        limit: i32,
        page: i32,
        name: Option<&str>,
    ) -> Result<(Vec<EventWithFiles>, i64), sqlx::Error> {
        let offset = (page - 1) * limit;

        // 1. 先查满足条件的总数
        let count_sql = r#"
            SELECT COUNT(*) AS total
            FROM events e
            WHERE e.task_id = ?
            AND (? IS NULL OR e.app_name LIKE CONCAT('%', ?, '%'))
        "#;

        let total: i64 = sqlx::query_scalar(count_sql)
            .bind(task_id)
            .bind(name)
            .bind(name)
            .fetch_one(&self.pool)
            .await?;
        // 2. 分页查询事件
        let events_sql = r#"
            SELECT
                e.*,
                el.latitude,
                el.longitude,
                el.created_at as location_created_at
            FROM events e
            LEFT JOIN event_location el ON e.event_id = el.event_id
            WHERE e.task_id = ?
            AND (? IS NULL OR e.app_name LIKE CONCAT('%', ?, '%'))
            ORDER BY e.created_at DESC
            LIMIT ? OFFSET ?
        "#;
        let event_records = sqlx::query_as::<_, EventWithLocation>(events_sql)
            .bind(task_id)
            .bind(name)
            .bind(name)
            .bind(limit)
            .bind(offset)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| {
                log::error!("查询事件记录失败: {}", e);
                e
            })?;
        if event_records.is_empty() {
            return Ok((Vec::new(), total));
        }
        // 3. 组装文件链接
        let event_ids: Vec<&str> = event_records
            .iter()
            .filter_map(|e| Some(e.event_id.as_ref())) // 正确处理 Option
            .collect();
        let files_by_event = self.get_files_by_event_ids(&event_ids).await?;

        // 4. 组合数据
        let mut items = Vec::new();
        for event_record in event_records {
            let event_id_clone = event_record.event_id.clone();

            let mut row = EventWithFiles {
                id: event_record.id,
                event_id: Some(event_id_clone.to_string()),
                event_state: event_record.event_state,
                device_name: event_record.device_name.clone(),
                device_id: event_record.device_id.clone(),
                task_name: event_record.task_name.clone(),
                task_id: event_record.task_id.clone(),
                app_name: event_record.app_name.clone(),
                app_id: event_record.app_id.clone(),
                src_name: event_record.src_name.clone(),
                src_id: event_record.src_id.clone(),
                details: event_record.details.clone(),
                raw_json: event_record.raw_json.clone(),
                created_at: event_record.created_at,
                updated_at: event_record.updated_at,
                latitude: event_record.latitude,
                longitude: event_record.longitude,
                location_created_at: event_record.location_created_at,
                preview_url: None,
                meta_url: None,
            };

            // 处理文件链接
            if let Some(files) = files_by_event.get(&event_id_clone) {
                for file in files {
                    let file_name_lower = file.file_name.to_lowercase();
                    if file_name_lower.ends_with(".jpg")
                        || file_name_lower.ends_with(".jpeg")
                        || file_name_lower.ends_with(".png")
                        || file_name_lower.ends_with(".gif")
                        || file_name_lower.ends_with(".bmp")
                    {
                        row.preview_url =
                            Some(format!("/files/{}/{}", event_id_clone, file.file_name));
                    } else if file_name_lower.ends_with(".json") {
                        row.meta_url =
                            Some(format!("/files/{}/{}", event_id_clone, file.file_name));
                    }
                }
            }

            items.push(row);
        }

        Ok((items, total))
    }
    // 根据事件ID列表获取文件信息
    async fn get_files_by_event_ids(
        &self,
        event_ids: &[&str],
    ) -> Result<HashMap<String, Vec<EventFile>>, sqlx::Error> {
        if event_ids.is_empty() {
            return Ok(HashMap::new());
        }
        // 过滤掉空字符串
        let valid_ids: Vec<&str> = event_ids
            .iter()
            .filter(|&&id| !id.is_empty())
            .copied()
            .collect();

        if valid_ids.is_empty() {
            return Ok(HashMap::new());
        }
        // 构建IN查询的占位符
        let placeholders = (0..valid_ids.len())
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT * FROM event_files WHERE event_id IN ({}) ORDER BY created_at DESC",
            placeholders
        );
        let mut query = sqlx::query_as::<_, EventFile>(&sql);
        for event_id in &valid_ids {
            query = query.bind(event_id);
        }
        match query.fetch_all(&self.pool).await {
            Ok(files) => {
                // 按事件ID分组
                let mut files_by_event: HashMap<String, Vec<EventFile>> = HashMap::new();
                for file in files {
                    if let Some(event_id) = &file.event_id {
                        if !event_id.is_empty() {
                            files_by_event
                                .entry(event_id.clone())
                                .or_insert_with(Vec::new)
                                .push(file);
                        }
                    }
                }
                Ok(files_by_event)
            }
            Err(e) => {
                log::error!("查询文件信息失败: {}", e);
                Err(e)
            }
        }
    }
    // 获取所有设备信息
    pub async fn get_all_devices(&self) -> Result<Vec<Device>, AppError> {
        let query = "SELECT * FROM devices WHERE status = 1";
        log::info!("执行获取所有设备信息的查询");
        let devices = sqlx::query_as::<_, Device>(query)
            .fetch_all(&self.pool)
            .await?;
        Ok(devices)
    }

    // 保存事件位置信息
    pub async fn save_event_location(
        &self,
        event_id: &str,
        task_id: &str,
        vessel_manager: &crate::vessel_client::VesselWebSocketManager,
    ) -> Result<bool, AppError> {
        // 从船只WebSocket缓存中获取最新位置数据
        if let Some(position_data) = vessel_manager.get_position_data(task_id).await {
            // 提取经纬度
            let (latitude, longitude) = extract_lat_lon(&position_data.data);

            // 验证经纬度是否有效（排除0值）
            if let (Some(lat), Some(lon)) = (latitude, longitude) {
                // 检查是否为无效的(0,0)坐标
                if lat != 0.0 || lon != 0.0 {
                    let query = r#"
                        INSERT INTO event_location (event_id, latitude, longitude, created_at)
                        VALUES (?, ?, ?, NOW())
                        ON DUPLICATE KEY UPDATE
                            latitude = VALUES(latitude),
                            longitude = VALUES(longitude),
                            created_at = VALUES(created_at)
                    "#;

                    sqlx::query(query)
                        .bind(event_id)
                        .bind(lat)
                        .bind(lon)
                        .execute(&self.pool)
                        .await?;

                    info!(
                        "事件位置信息保存成功: {} (设备: {}) - 位置: ({}, {})",
                        event_id, task_id, lat, lon
                    );
                    return Ok(true);
                } else {
                    warn!(
                        "船只位置数据为(0,0)，可能是无效数据: {:?}",
                        position_data.data
                    );
                }
            } else {
                warn!(
                    "无法从船只数据中提取有效的经纬度信息: {:?}",
                    position_data.data
                );
            }
        } else {
            warn!("没有找到设备 {} 的位置数据", task_id);
        }

        Ok(false)
    }

    pub async fn get_task_by_id(&self, task_id: &str) -> Result<Option<Device>, AppError> {
        let query = "SELECT * FROM devices WHERE status = 1 AND task_id = $1";
        let devices = sqlx::query_as::<_, Device>(query)
            .bind(task_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(devices)
    }
}

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: String,
    pub database: String,
}
// 提取经纬度
fn extract_lat_lon(data: &Value) -> (Option<f64>, Option<f64>) {
    let mut latitude = None;
    let mut longitude = None;

    // 从多种可能的数据结构中提取经纬度
    if let (Some(lat), Some(lon)) = (
        data.get("Lat").and_then(|v| v.as_f64()),
        data.get("Lon").and_then(|v| v.as_f64()),
    ) {
        latitude = Some(lat);
        longitude = Some(lon);
    } else if let (Some(lat), Some(lon)) = (
        data.get("latitude").and_then(|v| v.as_f64()),
        data.get("longitude").and_then(|v| v.as_f64()),
    ) {
        latitude = Some(lat);
        longitude = Some(lon);
    } else if let (Some(lat), Some(lng)) = (
        data.get("lat").and_then(|v| v.as_f64()),
        data.get("lng").and_then(|v| v.as_f64()),
    ) {
        latitude = Some(lat);
        longitude = Some(lng);
    } else if let (Some(lat), Some(lon)) = (
        data.get("lat").and_then(|v| v.as_f64()),
        data.get("lon").and_then(|v| v.as_f64()),
    ) {
        latitude = Some(lat);
        longitude = Some(lon);
    } else if let Some(location) = data.get("location") {
        if let Some(loc_obj) = location.as_object() {
            latitude = loc_obj
                .get("Lat")
                .or_else(|| loc_obj.get("latitude"))
                .or_else(|| loc_obj.get("lat"))
                .and_then(|v| v.as_f64());

            longitude = loc_obj
                .get("Lon")
                .or_else(|| loc_obj.get("longitude"))
                .or_else(|| loc_obj.get("lng"))
                .or_else(|| loc_obj.get("lon"))
                .and_then(|v| v.as_f64());
        }
    }

    // 验证经纬度是否有效（排除0值）
    if let (Some(lat), Some(lon)) = (latitude, longitude) {
        if lat == 0.0 && lon == 0.0 {
            (None, None)
        } else {
            (Some(lat), Some(lon))
        }
    } else {
        (None, None)
    }
}
