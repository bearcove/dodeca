//! Plugin side of the shared memory example
//!
//! Run shm_host first to create the shared memory, then run this with the shm name.
//!
//! Usage: cargo run --example shm_plugin -- /rapace-12345-67890

mod shm_common;

use rapace::shm::{SharedMemoryChannel, DEFAULT_RING_CAPACITY};
use shm_common::{dispatch_plugin_service, HostServiceClient, PluginService};

struct Plugin {
    host_client: HostServiceClient,
}

impl PluginService for Plugin {
    async fn render(&self, template_name: String) -> String {
        println!("[plugin] render({:?})", template_name);

        // Call back to host to load the template
        println!("[plugin] Calling host.load_template...");
        let template = match self.host_client.load_template(template_name.clone()).await {
            Ok(t) => t,
            Err(e) => {
                eprintln!("[plugin] Error loading template: {:?}", e);
                return format!("Error: {:?}", e);
            }
        };

        match template {
            Some(t) => {
                // Call back to host to resolve data
                println!("[plugin] Calling host.resolve_data...");
                let name = self
                    .host_client
                    .resolve_data("user.name".to_string())
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_else(|| "World".to_string());

                // Simple "render"
                t.replace("{{ name }}", &name)
            }
            None => format!("Template not found: {}", template_name),
        }
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: {} <shm_name>", args[0]);
        eprintln!("Example: {} /rapace-12345-67890", args[0]);
        std::process::exit(1);
    }

    let shm_name = &args[1];
    println!("[plugin] Opening shared memory: {}", shm_name);

    // Open existing shared memory
    let channel =
        SharedMemoryChannel::open(shm_name, DEFAULT_RING_CAPACITY).expect("Failed to open shm");

    // Plugin: reads from ring_a, writes to ring_b (opposite of host!)
    let (conn, mut incoming) = rapace::shm::run(channel.ring_b(), channel.ring_a()).await;

    // Create client to call host
    let host_client = HostServiceClient::new(conn.clone());

    // Create plugin service
    let plugin_service = Plugin { host_client };

    // Handle incoming PluginService requests from host
    let conn_for_handler = conn.clone();
    println!("[plugin] Ready, waiting for requests...");

    while let Some((id, payload)) = incoming.recv().await {
        println!("[plugin] Received request id={}", id);
        match dispatch_plugin_service(&plugin_service, &payload).await {
            Ok(response) => {
                println!("[plugin] Sending response for id={}", id);
                let _ = conn_for_handler.respond(id, response).await;
            }
            Err(e) => {
                eprintln!("[plugin] Error dispatching: {:?}", e);
            }
        }
    }

    println!("[plugin] Connection closed");
}
