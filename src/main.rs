// src/main.rs
mod database;
mod error;
mod file_handlers;
mod handlers;
mod logger;
mod models;
mod vessel_client;
mod websocket;
use actix_cors::Cors;
use actix_files::Files;
use actix_web::{middleware::Logger, web, App, HttpServer};
use dotenvy::dotenv;
use std::env;
use tokio::sync::Mutex;

use crate::logger::init_logger;

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // env_logger::init();
    dotenv().ok();
    init_logger(); // 初始化日志

    // 读取配置
    let db_config = database::DatabaseConfig {
        host: env::var("DB_HOST").unwrap_or_else(|_| "localhost".to_string()),
        port: env::var("DB_PORT")
            .unwrap_or_else(|_| "3306".to_string())
            .parse()
            .unwrap_or(3306),
        user: env::var("DB_USER").unwrap_or_else(|_| "mysql".to_string()),
        password: env::var("DB_PASSWORD").unwrap_or_else(|_| "password".to_string()),
        database: env::var("DB_NAME").unwrap_or_else(|_| "event_db".to_string()),
    };
    // 初始化数据库连接
    let db_manager = database::DatabaseManager::new(&db_config)
        .await
        .expect("Failed to connect to database");

    // 初始化WebSocket管理器
    let ws_manager = web::Data::new(Mutex::new(websocket::EventWebSocketManager::new()));
    // 初始化船只WebSocket管理器
    let mut vessel_manager = vessel_client::VesselWebSocketManager::new(db_manager.clone());
    // 设置WebSocket管理器用于广播位置更新
    vessel_manager.set_ws_manager(ws_manager.clone());

    let vessel_manager_data = web::Data::new(vessel_manager);

    // 启动船只连接
    vessel_manager_data
        .start()
        .await
        .expect("Failed to start vessel manager");
    // 创建上传目录
    tokio::fs::create_dir_all("./event_uploads").await?;

    println!("--- 事件接收服务已启动 (Rust + Actix-web) ---");
    println!("服务地址: http://0.0.0.0:8010");
    println!("WebSocket连接: ws://localhost:8010/ws");
    println!("事件列表API: http://localhost:8010/api/v1/events");
    println!("--------------------------");

    HttpServer::new(move || {
        App::new()
            .app_data(web::Data::new(db_manager.clone()))
            .app_data(ws_manager.clone())
            .app_data(vessel_manager_data.clone())
            .wrap(Logger::default())
            .wrap(
                Cors::default()
                    .allow_any_origin()
                    .allow_any_method()
                    .allow_any_header()
                    .max_age(3600),
            )
            .service(
                web::scope("/api/v1")
                    .route(
                        "/event-upload",
                        web::post().to(handlers::handle_event_upload),
                    )
                    .route(
                        "/tasks/{task_id}/events",
                        web::get().to(handlers::get_events_by_task_id),
                    )
                    .route(
                        "/events/{event_id}/package",
                        web::get().to(handlers::download_event_package_stream),
                    )
                    .route(
                        "/events/package",
                        web::get().to(handlers::download_events_package),
                    )
                    .route("/ws", web::get().to(websocket::event_websocket_route)),
            )
            .service(Files::new("/files", "./event_uploads").show_files_listing())
            .service(
                web::scope(""), // .route(
                                //     "/files/{event_id}/{file_name}",
                                //     web::get().to(handlers::get_event_file),
                                // )
            )
        // .route("/ws", web::get().to(websocket::websocket_endpoint))
    })
    .bind("0.0.0.0:8010")?
    .run()
    .await
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
