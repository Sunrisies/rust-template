use actix_web::web;
use futures_util::StreamExt;
// src/vessel_client.rs
use log::{error, info, warn};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use lru::LruCache;

use crate::{database::Device, error::AppError, websocket::EventWebSocketManager};

// 位置数据结构
#[derive(Debug, Clone)]
pub struct PositionData {
    pub data: Value,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

// 船只WebSocket管理器
pub struct VesselWebSocketManager {
    db_manager: crate::database::DatabaseManager,
    connections: Arc<RwLock<HashMap<String, VesselConnection>>>,
    position_cache: Arc<RwLock<LruCache<String, PositionData>>>, // 使用LRU缓存
    running: Arc<RwLock<bool>>,
    ws_manager: Option<web::Data<Mutex<EventWebSocketManager>>>, // 用于WebSocket广播
    cache_size: usize, // 缓存大小限制
}

#[derive(Debug)]
struct VesselConnection {
    device_id: String,
    // 可以添加更多连接状态信息
}

impl VesselWebSocketManager {
    pub fn new(db_manager: crate::database::DatabaseManager) -> Self {
        Self::new_with_cache_size(db_manager, 1000) // 默认缓存1000个位置数据
    }

    pub fn new_with_cache_size(db_manager: crate::database::DatabaseManager, cache_size: usize) -> Self {
        Self {
            db_manager,
            connections: Arc::new(RwLock::new(HashMap::new())),
            position_cache: Arc::new(RwLock::new(LruCache::new(
                std::num::NonZeroUsize::new(cache_size).unwrap_or_else(|| std::num::NonZeroUsize::new(1000).unwrap())
            ))),
            running: Arc::new(RwLock::new(false)),
            ws_manager: None,
            cache_size,
        }
    }

    // 设置WebSocket管理器用于广播
    pub fn set_ws_manager(&mut self, ws_manager: web::Data<Mutex<EventWebSocketManager>>) {
        self.ws_manager = Some(ws_manager);
    }

    pub async fn start(&self) -> Result<(), Box<dyn std::error::Error>> {
        {
            let mut running = self.running.write().await;
            *running = true;
        }

        info!("启动船只WebSocket管理器");

        // 获取所有设备
        let devices = self.get_all_devices().await?;

        // 为每个设备启动连接
        for device in devices {
            if device.status == 1 {
                self.connect_to_vessel(device).await;
            }
        }

        Ok(())
    }

    pub async fn stop(&self) {
        {
            let mut running = self.running.write().await;
            *running = false;
        }

        let mut connections = self.connections.write().await;
        connections.clear();

        info!("停止船只WebSocket管理器");
    }

    async fn get_all_devices(&self) -> Result<Vec<VesselDevice>, Box<dyn std::error::Error>> {
        // 从数据库获取设备列表
        let db_devices = self
            .db_manager
            .get_all_devices()
            .await
            .map_err(|e| format!("获取设备列表失败: {}", e))?;

        let vessel_devices: Vec<VesselDevice> = db_devices
            .into_iter()
            .filter_map(|device| {
                // // 检查设备是否有必要的连接信息
                if let (Some(host), Some(port)) = (&device.vessel_tcp_host, device.vessel_tcp_port)
                {
                    Some(VesselDevice {
                        device_id: device.device_id,
                        vessel_tcp_host: host.clone(),
                        vessel_tcp_port: port as u16,
                        status: device.status,
                    })
                } else {
                    warn!("设备 {} 缺少TCP主机或端口信息，跳过连接", device.device_id);
                    None
                }
            })
            .collect();

        info!("从数据库获取到 {} 个设备", vessel_devices.len());
        Ok(vessel_devices)
    }

    async fn connect_to_vessel(&self, device: VesselDevice) {
        let device_id = device.device_id.clone();
        let host = device.vessel_tcp_host.clone();
        let port = device.vessel_tcp_port;
        let max_retries = 5; // 增加重试次数
        let mut retry_count = 0;
        let mut consecutive_failures = 0; // 连续失败计数

        let connections = self.connections.clone();
        let position_cache = self.position_cache.clone();
        let running = self.running.clone();
        let ws_manager = self.ws_manager.clone();

        tokio::spawn(async move {
            let uri = format!("ws://{}/{}", host, port);

            loop {
                // 检查是否还在运行
                {
                    let is_running = running.read().await;
                    if !*is_running {
                        break;
                    }
                }

                // 检查重试次数
                if retry_count >= max_retries {
                    error!("设备 {} 连接失败，已达到最大重试次数", device_id);
                    // 实施指数退避策略
                    let backoff_seconds = std::cmp::min(300, 30 * (1 << consecutive_failures)); // 最大5分钟
                    info!("设备 {} 将在 {} 秒后重试", device_id, backoff_seconds);
                    tokio::time::sleep(tokio::time::Duration::from_secs(backoff_seconds)).await;
                    consecutive_failures += 1;
                    retry_count = 0; // 重置短时重试计数
                    continue;
                }

                match tokio_tungstenite::connect_async(&uri).await {
                    Ok((ws_stream, _)) => {
                        info!("成功连接到船只WebSocket: {} (设备: {})", uri, device_id);
                        retry_count = 0; // 重置重试计数器
                        consecutive_failures = 0; // 重置连续失败计数

                        let (_, mut read) = ws_stream.split();

                        // 添加到连接管理器
                        {
                            let mut conns = connections.write().await;
                            conns.insert(
                                device_id.clone(),
                                VesselConnection {
                                    device_id: device_id.clone(),
                                },
                            );
                        }

                        // 处理消息
                        while let Some(message) = read.next().await {
                            // 检查是否还在运行
                            {
                                let is_running = running.read().await;
                                if !*is_running {
                                    break;
                                }
                            }

                            match message {
                                Ok(tokio_tungstenite::tungstenite::Message::Text(text)) => {
                                    // 检查是否是设备关闭消息
                                    if let Ok(data) = serde_json::from_str::<Value>(&text) {
                                        // 检查设备是否返回关闭状态
                                        if let (Some(code), Some(msg)) = (
                                            data.get("code").and_then(|v| v.as_i64()),
                                            data.get("msg").and_then(|v| v.as_str())
                                        ) {
                                            if code == 0 && msg.contains("断开") {
                                                warn!("设备 {} 返回关闭状态: {}", device_id, msg);
                                                break;
                                            }
                                        }

                                        // 只有MsgID等于0的数据才保存
                                        if data.get("MsgID").and_then(|v| v.as_i64()) == Some(0) {
                                            let position_data = PositionData {
                                                data: data.clone(),
                                                timestamp: chrono::Utc::now(),
                                            };

                                            // 更新位置缓存
                                            {
                                                let mut cache = position_cache.write().await;
                                                cache.put(
                                                    device_id.clone(),
                                                    position_data.clone(),
                                                );
                                            }
                                        }
                                    } else {
                                        warn!("无法解析船只WebSocket消息: {}", text);
                                    }
                                }
                                Ok(tokio_tungstenite::tungstenite::Message::Close(_)) => {
                                    info!("船只WebSocket连接正常关闭: {} (设备: {})", uri, device_id);
                                    break;
                                }
                                Err(tokio_tungstenite::tungstenite::Error::ConnectionClosed) => {
                                    info!("船只WebSocket连接已关闭: {} (设备: {})", uri, device_id);
                                    break;
                                }
                                Err(tokio_tungstenite::tungstenite::Error::AlreadyClosed) => {
                                    warn!("船只WebSocket连接已经关闭: {} (设备: {})", uri, device_id);
                                    break;
                                }
                                Err(tokio_tungstenite::tungstenite::Error::Io(ref e)) 
                                    if e.kind() == std::io::ErrorKind::ConnectionReset => 
                                {
                                    // 特殊处理连接重置错误
                                    warn!("船只WebSocket连接被重置: {} (设备: {})", uri, device_id);
                                    // 立即重试，但不计入连续失败
                                    retry_count += 1;
                                    break;
                                }
                                Err(e) => {
                                    error!(
                                        "船只WebSocket接收错误: {} (设备: {}): {}",
                                        uri, device_id, e
                                    );
                                    // 对于其他错误，计入重试次数
                                    retry_count += 1;
                                    break;
                                }
                                _ => {}
                            }
                        }

                        // 连接断开，从管理器移除
                        {
                            let mut conns = connections.write().await;
                            conns.remove(&device_id);
                        }
                    }
                    Err(e) => {
                        error!(
                            "连接船只WebSocket失败: {} (设备: {}): {}",
                            uri, device_id, e
                        );
                        retry_count += 1;
                    }
                }

                // 根据连续失败次数调整等待时间
                let wait_seconds = if consecutive_failures > 0 {
                    std::cmp::min(60, 5 * consecutive_failures) // 最大1分钟
                } else {
                    5 // 默认5秒
                };

                info!("设备 {} 将在 {} 秒后重试连接 (重试次数: {}/{})", 
                      device_id, wait_seconds, retry_count, max_retries);
                tokio::time::sleep(tokio::time::Duration::from_secs(wait_seconds)).await;
            }
        });
    }

    pub async fn get_position_data(&self, device_id: &str) -> Option<PositionData> {
        let mut cache = self.position_cache.write().await;
        // LruCache的get方法会更新访问顺序，所以需要可变引用
        cache.get(device_id).cloned()
    }

    pub async fn ensure_connection(&self, task_id: &str) -> Result<(), AppError> {
        // 检查是否已有连接
        {
            let conns = self.connections.read().await;
            if conns.contains_key(task_id) {
                return Ok(());
            }
        }

        // 获取设备信息
        if let Some(device) = self.get_task_by_id(task_id).await? {
            // 建立新连接
            self.connect_to_vessel(device).await;
            Ok(())
        } else {
            Err(AppError::NotFound(format!("设备 {} 未找到", task_id)))
        }
    }
    pub async fn get_task_by_id(&self, device_id: &str) -> Result<Option<VesselDevice>, AppError> {
        if let Some(device) = self.db_manager.get_task_by_id(device_id).await? {
            // 转换Device为VesselDevice
            if let (Some(host), Some(port)) = (&device.vessel_tcp_host, device.vessel_tcp_port) {
                Ok(Some(VesselDevice {
                    device_id: device.device_id,
                    vessel_tcp_host: host.clone(),
                    vessel_tcp_port: port as u16,
                    status: device.status,
                }))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }
    // 保存事件及其位置信息
    pub async fn save_event_with_position(
        &self,
        event_data: &crate::models::Event,
    ) -> Result<bool, Box<dyn std::error::Error>> {
        // 保存事件数据
        // self.db_manager.save_event(event_data).await?;

        // 保存位置信息
        if let (Some(event_id), Some(task_id)) = (&event_data.event_id, &event_data.task_id) {
            self.db_manager
                .save_event_location(event_id, task_id, self)
                .await?;
        }

        Ok(true)
    }

    // 检查设备是否在线
    pub async fn check_device_status(&self, device_id: &str) -> Result<bool, AppError> {
        // 检查连接管理器中是否有该设备的连接
        {
            let conns = self.connections.read().await;
            if conns.contains_key(device_id) {
                return Ok(true);
            }
        }

        // 检查缓存中是否有最近的位置数据（表示最近连接过）
        {
            let mut cache = self.position_cache.write().await;
            // LruCache的get方法会更新访问顺序，所以需要可变引用
            if let Some(pos_data) = cache.get(device_id) {
                // 如果位置数据是5分钟内的，认为设备可能在线
                let now = chrono::Utc::now();
                let diff = now.signed_duration_since(pos_data.timestamp);
                if diff.num_minutes() < 5 {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }
}

// 设备信息结构体
#[derive(Debug, Clone)]
pub struct VesselDevice {
    pub device_id: String,
    pub vessel_tcp_host: String,
    pub vessel_tcp_port: u16,
    pub status: i8,
}
