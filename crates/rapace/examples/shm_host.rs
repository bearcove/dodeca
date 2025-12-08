//! Host side of the shared memory example
//!
//! Run this first, it will print the shared memory name.
//! Then run shm_plugin with that name as an argument.
//!
//! Usage: cargo run --example shm_host

mod shm_common;

use rapace::shm::{SharedMemoryChannel, DEFAULT_RING_CAPACITY};
use shm_common::{dispatch_host_service, HostService, PluginServiceClient};
use std::time::Duration;

struct Host;

impl HostService for Host {
    async fn load_template(&self, name: String) -> Option<String> {
        println!("[host] load_template({:?})", name);
        match name.as_str() {
            "greeting.html" => Some("Hello, {{ name }}!".to_string()),
            "base.html" => Some("<html>{% block content %}{% endblock %}</html>".to_string()),
            _ => None,
        }
    }

    async fn resolve_data(&self, path: String) -> Option<String> {
        println!("[host] resolve_data({:?})", path);
        match path.as_str() {
            "user.name" => Some("Alice".to_string()),
            "user.email" => Some("alice@example.com".to_string()),
            _ => None,
        }
    }
}

#[tokio::main]
async fn main() {
    // Create shared memory
    let channel = SharedMemoryChannel::new(DEFAULT_RING_CAPACITY).expect("Failed to create shm");
    let shm_name = channel.name().to_string();
    println!("Shared memory created: {}", shm_name);
    println!("Run: cargo run --example shm_plugin -- {}", shm_name);

    // Host: writes to ring_a, reads from ring_b
    let (conn, mut incoming) = rapace::shm::run(channel.ring_a(), channel.ring_b()).await;

    // Create client to call plugin
    let plugin_client = PluginServiceClient::new(conn.clone());

    // Spawn handler for incoming HostService requests from plugin
    let host_service = Host;
    let conn_for_handler = conn.clone();
    tokio::spawn(async move {
        while let Some((id, payload)) = incoming.recv().await {
            println!("[host] Received request id={}", id);
            match dispatch_host_service(&host_service, &payload).await {
                Ok(response) => {
                    let _ = conn_for_handler.respond(id, response).await;
                }
                Err(e) => {
                    eprintln!("[host] Error dispatching: {:?}", e);
                }
            }
        }
    });

    // Wait for plugin to connect
    println!("[host] Waiting for plugin (10 seconds)...");
    tokio::time::sleep(Duration::from_secs(10)).await;

    // Call the plugin
    println!("[host] Calling plugin.render(\"greeting.html\")...");
    match tokio::time::timeout(
        Duration::from_secs(10),
        plugin_client.render("greeting.html".to_string()),
    )
    .await
    {
        Ok(Ok(result)) => {
            println!("[host] Result: {}", result);
        }
        Ok(Err(e)) => {
            eprintln!("[host] Error: {:?}", e);
        }
        Err(_) => {
            eprintln!("[host] Timeout waiting for response");
        }
    }

    println!("[host] Done");
}
