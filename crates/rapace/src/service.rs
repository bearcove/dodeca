//! Service macro for defining typed RPC interfaces
//!
//! # Example
//!
//! ```ignore
//! rapace::service! {
//!     pub trait MyService {
//!         async fn greet(name: String) -> String;
//!         async fn add(a: i32, b: i32) -> i32;
//!     }
//! }
//! ```
//!
//! This generates:
//! - `MyServiceRequest` enum with variants for each method
//! - `MyServiceResponse` enum with variants for each method
//! - `MyServiceClient` struct with async methods that send requests
//! - `MyService` trait that you implement to handle requests

/// Define a service interface with typed request/response
///
/// Each method becomes a request/response pair, serialized with Facet.
#[macro_export]
macro_rules! service {
    (
        $(#[$trait_attr:meta])*
        $vis:vis trait $name:ident {
            $(
                $(#[$method_attr:meta])*
                async fn $method:ident($($arg:ident: $arg_ty:ty),* $(,)?) -> $ret:ty;
            )*
        }
    ) => {
        $crate::service!(@paste
            $(#[$trait_attr])*
            $vis trait $name {
                $(
                    $(#[$method_attr])*
                    async fn $method($($arg: $arg_ty),*) -> $ret;
                )*
            }
        );
    };

    (@paste
        $(#[$trait_attr:meta])*
        $vis:vis trait $name:ident {
            $(
                $(#[$method_attr:meta])*
                async fn $method:ident($($arg:ident: $arg_ty:ty),*) -> $ret:ty;
            )*
        }
    ) => {
        $crate::paste::paste! {
            // Request enum
            #[derive(::facet::Facet, Debug)]
            #[repr(u8)]
            $vis enum [<$name Request>] {
                $(
                    [<$method:camel>] { $($arg: $arg_ty),* },
                )*
            }

            // Response enum
            #[derive(::facet::Facet, Debug)]
            #[repr(u8)]
            $vis enum [<$name Response>] {
                $(
                    [<$method:camel>]($ret),
                )*
            }

            // Service trait
            $(#[$trait_attr])*
            $vis trait $name {
                $(
                    $(#[$method_attr])*
                    fn $method(&self, $($arg: $arg_ty),*) -> impl std::future::Future<Output = $ret> + Send;
                )*
            }

            // Client struct
            $vis struct [<$name Client>] {
                conn: $crate::Connection,
            }

            impl [<$name Client>] {
                /// Create a new client wrapping a connection
                pub fn new(conn: $crate::Connection) -> Self {
                    Self { conn }
                }

                $(
                    $(#[$method_attr])*
                    pub async fn $method(&self, $($arg: $arg_ty),*) -> Result<$ret, $crate::RequestError> {
                        let request = [<$name Request>]::[<$method:camel>] { $($arg),* };
                        let request_bytes = $crate::facet_postcard::to_vec(&request)
                            .map_err(|_| $crate::RequestError::SendFailed)?;
                        let response_bytes = self.conn.request(request_bytes).await?;
                        let response: [<$name Response>] = $crate::facet_postcard::from_bytes(&response_bytes)
                            .map_err(|_| $crate::RequestError::Cancelled)?;
                        match response {
                            [<$name Response>]::[<$method:camel>](v) => Ok(v),
                            #[allow(unreachable_patterns)]
                            _ => Err($crate::RequestError::Cancelled),
                        }
                    }
                )*
            }

            // Server dispatch function
            $vis async fn [<dispatch_ $name:snake>]<S: $name>(
                service: &S,
                request_bytes: &[u8],
            ) -> Result<Vec<u8>, $crate::RequestError> {
                let request: [<$name Request>] = $crate::facet_postcard::from_bytes(request_bytes)
                    .map_err(|_| $crate::RequestError::Cancelled)?;
                let response = match request {
                    $(
                        [<$name Request>]::[<$method:camel>] { $($arg),* } => {
                            let result = service.$method($($arg),*).await;
                            [<$name Response>]::[<$method:camel>](result)
                        }
                    )*
                };
                $crate::facet_postcard::to_vec(&response)
                    .map_err(|_| $crate::RequestError::SendFailed)
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use tokio::io::duplex;

    // Define a test service
    crate::service! {
        pub trait Calculator {
            async fn add(a: i32, b: i32) -> i32;
            async fn greet(name: String) -> String;
        }
    }

    // Implement the service
    struct CalculatorImpl;

    impl Calculator for CalculatorImpl {
        async fn add(&self, a: i32, b: i32) -> i32 {
            a + b
        }

        async fn greet(&self, name: String) -> String {
            format!("Hello, {}!", name)
        }
    }

    #[tokio::test]
    async fn test_service_macro() {
        // Create a bidirectional pipe
        let (client_stream, server_stream) = duplex(64 * 1024);
        let (client_read, client_write) = tokio::io::split(client_stream);
        let (server_read, server_write) = tokio::io::split(server_stream);

        // Set up client side
        let (client_conn, _) = crate::socket::run(client_read, client_write).await.unwrap();
        let client = CalculatorClient::new(client_conn);

        // Set up server side
        let (server_conn, mut server_incoming) =
            crate::socket::run(server_read, server_write).await.unwrap();
        let service = CalculatorImpl;

        // Spawn server handler
        tokio::spawn(async move {
            while let Some((id, payload)) = server_incoming.recv().await {
                let response = dispatch_calculator(&service, &payload).await.unwrap();
                let _ = server_conn.respond(id, response).await;
            }
        });

        // Test add
        let result = client.add(2, 3).await.unwrap();
        assert_eq!(result, 5);

        // Test greet
        let result = client.greet("World".to_string()).await.unwrap();
        assert_eq!(result, "Hello, World!");
    }

    // =========================================================================
    // Test bidirectional services - like host/plugin communication
    // =========================================================================

    // Host service - provides data to plugin
    crate::service! {
        pub trait HostService {
            async fn load_template(name: String) -> Option<String>;
            async fn resolve_data(path: String) -> Option<String>;
        }
    }

    // Plugin service - renders templates
    crate::service! {
        pub trait PluginService {
            async fn render(template_name: String) -> String;
        }
    }

    // Host implementation
    struct Host;

    impl HostService for Host {
        async fn load_template(&self, name: String) -> Option<String> {
            match name.as_str() {
                "greeting.html" => Some("Hello, {{ name }}!".to_string()),
                "base.html" => Some("<html>{% block content %}{% endblock %}</html>".to_string()),
                _ => None,
            }
        }

        async fn resolve_data(&self, path: String) -> Option<String> {
            match path.as_str() {
                "user.name" => Some("Alice".to_string()),
                "user.email" => Some("alice@example.com".to_string()),
                _ => None,
            }
        }
    }

    // Plugin implementation - calls BACK to host to get templates/data
    struct Plugin {
        host_client: HostServiceClient,
    }

    impl PluginService for Plugin {
        async fn render(&self, template_name: String) -> String {
            // Call back to host to load the template
            let template = self
                .host_client
                .load_template(template_name.clone())
                .await
                .unwrap();

            match template {
                Some(t) => {
                    // Call back to host to resolve data
                    let name = self
                        .host_client
                        .resolve_data("user.name".to_string())
                        .await
                        .unwrap()
                        .unwrap_or_else(|| "World".to_string());

                    // "Render" by simple replacement
                    t.replace("{{ name }}", &name)
                }
                None => format!("Template not found: {}", template_name),
            }
        }
    }

    #[tokio::test]
    async fn test_bidirectional_services() {
        // Create bidirectional pipe
        let (host_stream, plugin_stream) = duplex(64 * 1024);
        let (host_read, host_write) = tokio::io::split(host_stream);
        let (plugin_read, plugin_write) = tokio::io::split(plugin_stream);

        // Set up host side connection
        let (host_conn, mut host_incoming) =
            crate::socket::run(host_read, host_write).await.unwrap();

        // Set up plugin side connection
        let (plugin_conn, mut plugin_incoming) =
            crate::socket::run(plugin_read, plugin_write).await.unwrap();

        // Host is a client to PluginService
        let plugin_client = PluginServiceClient::new(host_conn.clone());

        // Plugin is a client to HostService
        let host_client = HostServiceClient::new(plugin_conn.clone());

        // Host service implementation
        let host_service = Host;

        // Plugin service implementation (with reference back to host)
        let plugin_service = Plugin { host_client };

        // Spawn host's request handler (handles HostService requests from plugin)
        let host_conn_for_handler = host_conn.clone();
        tokio::spawn(async move {
            while let Some((id, payload)) = host_incoming.recv().await {
                let response = dispatch_host_service(&host_service, &payload).await.unwrap();
                let _ = host_conn_for_handler.respond(id, response).await;
            }
        });

        // Spawn plugin's request handler (handles PluginService requests from host)
        let plugin_conn_for_handler = plugin_conn.clone();
        tokio::spawn(async move {
            while let Some((id, payload)) = plugin_incoming.recv().await {
                let response = dispatch_plugin_service(&plugin_service, &payload)
                    .await
                    .unwrap();
                let _ = plugin_conn_for_handler.respond(id, response).await;
            }
        });

        // Now the host can call the plugin to render
        // The plugin will call BACK to the host to get template and data
        let result = plugin_client.render("greeting.html".to_string()).await.unwrap();
        assert_eq!(result, "Hello, Alice!");

        // Try with missing template
        let result = plugin_client.render("missing.html".to_string()).await.unwrap();
        assert_eq!(result, "Template not found: missing.html");
    }
}
