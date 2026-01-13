//! Dodeca code execution cell (cell-code-execution)
//!
//! This cell handles extracting and executing code samples from markdown.

use roam_shm::driver::establish_guest;
use roam_shm::guest::ShmGuest;
use roam_shm::spawn::SpawnArgs;
use roam_shm::transport::ShmGuestTransport;

use cell_code_execution_proto::{CodeExecutionResult, CodeExecutor, CodeExecutorDispatcher};

// Include implementation code directly
include!("impl.rs");

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = SpawnArgs::from_env()?;
    let guest = ShmGuest::attach_with_ticket(&args)?;
    let transport = ShmGuestTransport::new(guest);
    let dispatcher = CodeExecutorDispatcher::new(CodeExecutorImpl);
    let (_handle, driver) = establish_guest(transport, dispatcher);
    driver.run().await;
    Ok(())
}
