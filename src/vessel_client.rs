use actix_web::web;
use futures_util::StreamExt;
// src/vessel_client.rs
use log::{error, info, warn};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

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
    position_cache: Arc<RwLock<HashMap<String, PositionData>>>,
    running: Arc<RwLock<bool>>,
    ws_manager: Option<web::Data<Mutex<EventWebSocketManager>>>, // 用于WebSocket广播
}

#[derive(Debug)]
struct VesselConnection {
    device_id: String,
    // 可以添加更多连接状态信息
}

impl VesselWebSocketManager {
    pub fn new(db_manager: crate::database::DatabaseManager) -> Self {
        Self {
            db_manager,
            connections: Arc::new(RwLock::new(HashMap::new())),
            position_cache: Arc::new(RwLock::new(HashMap::new())),
            running: Arc::new(RwLock::new(false)),
            ws_manager: None,
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
        let max_retries = 3;
        let mut retry_count = 0;

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
                    break;
                }

                match tokio_tungstenite::connect_async(&uri).await {
                    Ok((ws_stream, _)) => {
                        info!("成功连接到船只WebSocket: {} (设备: {})", uri, device_id);
                        retry_count = 0; // 重置重试计数器

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
                                    // 处理船只数据
                                    if let Ok(data) = serde_json::from_str::<Value>(&text) {
                                        // 只有MsgID等于0的数据才保存
                                        if data.get("MsgID").and_then(|v| v.as_i64()) == Some(0) {
                                            let position_data = PositionData {
                                                data: data.clone(),
                                                timestamp: chrono::Utc::now(),
                                            };

                                            // 更新位置缓存
                                            {
                                                let mut cache = position_cache.write().await;
                                                cache.insert(
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
                                    info!("船只WebSocket连接关闭: {} (设备: {})", uri, device_id);
                                    break;
                                }
                                Err(e) => {
                                    error!(
                                        "船只WebSocket接收错误: {} (设备: {}): {}",
                                        uri, device_id, e
                                    );
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

                // 等待5秒后重试
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
            }
        });
    }

    pub async fn get_position_data(&self, device_id: &str) -> Option<PositionData> {
        let cache = self.position_cache.read().await;
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
}

// 设备信息结构体
#[derive(Debug, Clone)]
pub struct VesselDevice {
    pub device_id: String,
    pub vessel_tcp_host: String,
    pub vessel_tcp_port: u16,
    pub status: i8,
}
