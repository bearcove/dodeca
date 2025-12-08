//! Shared service definitions for the shm examples
//!
//! This module defines the HostService and PluginService interfaces
//! that are used by both shm_host and shm_plugin examples.

// Host service - provides data to plugin
rapace::service! {
    pub trait HostService {
        async fn load_template(name: String) -> Option<String>;
        async fn resolve_data(path: String) -> Option<String>;
    }
}

// Plugin service - renders templates
rapace::service! {
    pub trait PluginService {
        async fn render(template_name: String) -> String;
    }
}
