use actix_ws::Session;
use log::{error, info};
use std::collections::HashMap;
use tokio::sync::Mutex;
use uuid::Uuid;

// WebSocket会话管理器
#[derive(Default)]
pub struct EventWebSocketManager {
    sessions: Mutex<HashMap<String, Session>>,
    device_subscriptions: Mutex<HashMap<String, Vec<String>>>, // device_id -> session_ids
    task_subscriptions: Mutex<HashMap<String, Vec<String>>>,   // task_id -> session_ids
}

impl EventWebSocketManager {
    pub fn new() -> Self {
        info!("Event WebSocket管理器已启动");
        Self {
            sessions: Mutex::new(HashMap::new()),
            device_subscriptions: Mutex::new(HashMap::new()),
            task_subscriptions: Mutex::new(HashMap::new()),
        }
    }

    // 添加新会话
    pub async fn add_session(&self, session: Session) -> String {
        let session_id = Uuid::new_v4().to_string();
        let mut sessions = self.sessions.lock().await;
        sessions.insert(session_id.clone(), session);

        info!(
            "新的WebSocket会话连接: {}, 当前会话数: {}",
            session_id,
            sessions.len()
        );
        session_id
    }

    pub async fn broadcast_string(&self, message: &str) -> Result<(), Box<dyn std::error::Error>> {
        let sessions_snapshot: Vec<(String, Session)> = {
            let sessions = self.sessions.lock().await;
            sessions
                .iter()
                .map(|(id, session)| (id.clone(), session.clone()))
                .collect()
        };

        let mut tasks = Vec::new();
        for (session_id, mut session) in sessions_snapshot {
            let message = message.to_string();
            tasks.push(tokio::spawn(async move {
                match session.text(message).await {
                    Ok(_) => {
                        info!("消息发送到会话 {} 成功", session_id);
                        Ok(())
                    }
                    Err(e) => {
                        error!("发送消息到会话 {} 失败: {}", session_id, e);
                        Err(e)
                    }
                }
            }));
        }

        for task in tasks {
            let _ = task.await;
        }

        Ok(())
    }

    // 移除会话
    pub async fn remove_session(&self, session_id: &str) {
        let mut sessions = self.sessions.lock().await;
        sessions.remove(session_id);

        // 清理订阅
        self.cleanup_subscriptions(session_id).await;

        info!(
            "WebSocket会话断开: {}, 当前会话数: {}",
            session_id,
            sessions.len()
        );
    }
    // 清理会话的订阅
    async fn cleanup_subscriptions(&self, session_id: &str) {
        let mut device_subs = self.device_subscriptions.lock().await;
        let mut task_subs = self.task_subscriptions.lock().await;

        // 清理设备订阅
        for sessions in device_subs.values_mut() {
            sessions.retain(|id| id != session_id);
        }

        // 清理任务订阅
        for sessions in task_subs.values_mut() {
            sessions.retain(|id| id != session_id);
        }

        // 移除空的订阅列表
        device_subs.retain(|_, sessions| !sessions.is_empty());
        task_subs.retain(|_, sessions| !sessions.is_empty());
    }
}
