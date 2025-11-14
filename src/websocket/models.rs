use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
// 客户端发送的消息格式
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClientMessage {
    pub message_type: String, // "subscribe", "unsubscribe", "ping", "get_position", etc.
    pub device_id: Option<String>,
    pub task_id: Option<String>,
    pub content: Option<String>,
}

// 服务器广播的消息格式 - 事件通知
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct EventNotification {
    pub message_type: String, // "event_received", "file_received", "position_updated"
    pub event_id: Option<String>,
    pub device_name: Option<String>,
    pub task_name: Option<String>,
    pub device_id: Option<String>,
    pub task_id: Option<String>,
    pub timestamp: Option<i64>,
    pub file_count: Option<usize>,
    pub file_type: Option<String>,
    pub position_data: Option<PositionData>,
    pub content: Option<String>,
}

// 位置数据格式
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct PositionData {
    pub device_id: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub timestamp: DateTime<Utc>,
    pub raw_data: Option<serde_json::Value>,
}

// 服务器广播的消息格式 - 系统消息
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SystemMessage {
    pub message_type: String, // "system", "error", "info"
    pub content: String,
    pub timestamp: DateTime<Utc>,
}
