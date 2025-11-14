// src/models.rs
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Box {
    pub left_top_x: Option<i32>,
    pub left_top_y: Option<i32>,
    pub right_bottom_x: Option<i32>,
    pub right_bottom_y: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Target {
    pub id: Option<i32>,
    pub r#box: Option<Box>,
    pub color: Option<Vec<i32>>,
    pub label: Option<String>,
    pub prob: Option<f32>,
    pub moving_state: Option<bool>,
    pub moving_direction: Option<i32>,
    pub trajectory: Option<Vec<std::collections::HashMap<String, i32>>>,
    pub region_label: Option<String>,
    pub image_base64: Option<String>,
    pub extras: Option<std::collections::HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Detail {
    pub frame_id: Option<i32>,
    pub model_id: Option<String>,
    pub model_name: Option<String>,
    pub model_thres: Option<f32>,
    pub model_type: Option<i32>,
    pub regions_with_label: Option<std::collections::HashMap<String, serde_json::Value>>,
    pub targets: Option<Vec<Target>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
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
    pub created: Option<i64>,
    pub details: Option<Vec<Detail>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct EventRecord {
    pub id: i64,
    pub event_id: String,
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EventWithLocation {
    pub id: i64,
    pub event_id: String,
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
    pub latitude: Option<f64>,  // 位置信息
    pub longitude: Option<f64>, // 位置信息
    pub location_created_at: Option<chrono::DateTime<chrono::Utc>>, // 位置信息
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    pub code: i32,
    pub status: i32,
    pub message: String,
    pub data: T,
}
