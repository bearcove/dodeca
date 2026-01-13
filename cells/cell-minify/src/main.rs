//! Dodeca minify cell (cell-minify)
//!
//! This cell handles HTML minification.

use roam_shm::driver::establish_guest;
use roam_shm::guest::ShmGuest;
use roam_shm::spawn::SpawnArgs;
use roam_shm::transport::ShmGuestTransport;

use cell_minify_proto::{Minifier, MinifierDispatcher, MinifyResult};

/// Minifier implementation
#[derive(Clone)]
pub struct MinifierImpl;

impl Minifier for MinifierImpl {
    async fn minify_html(&self, html: String) -> MinifyResult {
        // TODO: Use facet-html for minification instead
        // For now, just return the input unchanged (no-op)
        MinifyResult::Success { content: html }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = SpawnArgs::from_env()?;
    let guest = ShmGuest::attach_with_ticket(&args)?;
    let transport = ShmGuestTransport::new(guest);
    let dispatcher = MinifierDispatcher::new(MinifierImpl);
    let (_handle, driver) = establish_guest(transport, dispatcher);
    driver.run().await;
    Ok(())
}
