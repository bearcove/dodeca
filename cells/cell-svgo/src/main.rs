//! Dodeca SVGO cell (cell-svgo)
//!
//! This cell handles SVG optimization.

use roam_shm::driver::establish_guest;
use roam_shm::guest::ShmGuest;
use roam_shm::spawn::SpawnArgs;
use roam_shm::transport::ShmGuestTransport;

use cell_svgo_proto::{SvgoOptimizer, SvgoOptimizerDispatcher, SvgoResult};

/// SVGO optimizer implementation
#[derive(Clone)]
pub struct SvgoOptimizerImpl;

impl SvgoOptimizer for SvgoOptimizerImpl {
    async fn optimize_svg(&self, svg: String) -> SvgoResult {
        match svag::minify(&svg) {
            Ok(optimized) => SvgoResult::Success { svg: optimized },
            Err(e) => SvgoResult::Error {
                message: format!("SVG optimization failed: {}", e),
            },
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = SpawnArgs::from_env()?;
    let guest = ShmGuest::attach_with_ticket(&args)?;
    let transport = ShmGuestTransport::new(guest);
    let dispatcher = SvgoOptimizerDispatcher::new(SvgoOptimizerImpl);
    let (_handle, driver) = establish_guest(transport, dispatcher);
    driver.run().await.ok();
    Ok(())
}
