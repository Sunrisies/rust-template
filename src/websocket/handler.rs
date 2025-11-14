// src/websocket/handler.rs
use actix_web::{web, Error, HttpRequest, HttpResponse};
use actix_ws::{Message, Session};
use futures_util::StreamExt;
use log::{info, warn};
use serde_json;
use tokio::sync::Mutex;

use super::manager::EventWebSocketManager;
use super::models::{ClientMessage, EventNotification, SystemMessage};

// WebSocket路由处理函数
pub async fn event_websocket_route(
    req: HttpRequest,
    stream: web::Payload,
    ws_manager: web::Data<Mutex<EventWebSocketManager>>,
) -> Result<HttpResponse, Error> {
    info!("新的WebSocket连接请求");

    // 建立WebSocket连接
    let (response, mut session, msg_stream) = actix_ws::handle(&req, stream)?;
    // 添加到WebSocket管理器
    let session_id = {
        let manager = ws_manager.lock().await;
        manager.add_session(session.clone()).await
    }; // 锁在这里被释放

    // 发送欢迎消息
    let welcome_msg = SystemMessage {
        message_type: "system".to_string(),
        content: "已连接到事件接收服务WebSocket".to_string(),
        timestamp: chrono::Utc::now(),
    };

    if let Ok(message_json) = serde_json::to_string(&welcome_msg) {
        let _ = session.text(message_json).await;
    }

    // 处理消息流
    actix_rt::spawn(handle_websocket_messages(
        session, msg_stream, ws_manager, session_id,
    ));

    Ok(response)
}

// 处理WebSocket消息
async fn handle_websocket_messages(
    mut session: Session,
    mut msg_stream: actix_ws::MessageStream,
    ws_manager: web::Data<Mutex<EventWebSocketManager>>,
    session_id: String,
) {
    info!("开始处理WebSocket消息流: {}", session_id);

    while let Some(Ok(msg)) = msg_stream.next().await {
        match msg {
            Message::Text(text) => {
                info!("收到文本消息 from {}: {}", session_id, text);

                // 解析客户端消息
                match serde_json::from_str::<ClientMessage>(&text) {
                    Ok(_client_msg) => {}
                    Err(e) => {
                        warn!("解析客户端消息失败: {}", e);

                        // 发送错误消息回客户端
                        let error_msg = SystemMessage {
                            message_type: "error".to_string(),
                            content: format!("消息格式错误: {}", e),
                            timestamp: chrono::Utc::now(),
                        };

                        if let Ok(error_text) = serde_json::to_string(&error_msg) {
                            let _ = session.text(error_text).await;
                        }
                    }
                }
            }
            Message::Binary(bin) => {
                info!("收到二进制消息 from {}: {} bytes", session_id, bin.len());

                // 可以处理二进制消息，如图片/文件
                // 这里简单回显
                let _ = session.binary(bin).await;
            }
            Message::Ping(bytes) => {
                info!("收到Ping消息 from {}", session_id);
                let _ = session.pong(&bytes).await;

                // 发送pong响应
                let pong_msg = SystemMessage {
                    message_type: "pong".to_string(),
                    content: "pong".to_string(),
                    timestamp: chrono::Utc::now(),
                };

                if let Ok(pong_text) = serde_json::to_string(&pong_msg) {
                    let _ = session.text(pong_text).await;
                }
            }
            Message::Pong(_) => {
                // 忽略pong消息
            }
            Message::Close(reason) => {
                info!("WebSocket连接关闭 by {}: {:?}", session_id, reason);
                break;
            }
            Message::Continuation(_) => {
                // 处理continuation帧
                info!("收到Continuation帧 from {}", session_id);
            }
            Message::Nop => {
                // 无操作
            }
        }
    }

    // 连接断开，从管理器移除
    info!("WebSocket连接断开: {}", session_id);
    let ws_manager = ws_manager.lock().await;
    ws_manager.remove_session(&session_id).await;
}
