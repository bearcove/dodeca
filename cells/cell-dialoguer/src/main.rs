//! Dodeca dialoguer cell
//!
//! Provides interactive terminal prompts using the dialoguer crate.

use cell_dialoguer_proto::{ConfirmResult, Dialoguer, DialoguerDispatcher, SelectResult};
use dialoguer::{Confirm, Select, theme::ColorfulTheme};
use roam_shm::driver::establish_guest;
use roam_shm::guest::ShmGuest;
use roam_shm::spawn::SpawnArgs;
use roam_shm::transport::ShmGuestTransport;

/// Dialoguer service implementation
#[derive(Clone)]
pub struct DialoguerImpl;

impl Dialoguer for DialoguerImpl {
    async fn select(&self, prompt: String, items: Vec<String>) -> SelectResult {
        // Run the blocking dialoguer call in a blocking task
        let result = tokio::task::spawn_blocking(move || {
            Select::with_theme(&ColorfulTheme::default())
                .with_prompt(&prompt)
                .items(&items)
                .default(0)
                .interact_opt()
        })
        .await;

        match result {
            Ok(Ok(Some(index))) => SelectResult::Selected { index },
            Ok(Ok(None)) => SelectResult::Cancelled,
            Ok(Err(_)) | Err(_) => SelectResult::Cancelled,
        }
    }

    async fn confirm(&self, prompt: String, default: bool) -> ConfirmResult {
        let result = tokio::task::spawn_blocking(move || {
            Confirm::with_theme(&ColorfulTheme::default())
                .with_prompt(&prompt)
                .default(default)
                .interact_opt()
        })
        .await;

        match result {
            Ok(Ok(Some(true))) => ConfirmResult::Yes,
            Ok(Ok(Some(false))) => ConfirmResult::No,
            Ok(Ok(None)) => ConfirmResult::Cancelled,
            Ok(Err(_)) | Err(_) => ConfirmResult::Cancelled,
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = SpawnArgs::from_env()?;
    let guest = ShmGuest::attach_with_ticket(&args)?;
    let transport = ShmGuestTransport::new(guest);
    let dispatcher = DialoguerDispatcher::new(DialoguerImpl);
    let (_handle, driver) = establish_guest(transport, dispatcher);
    driver.run().await?;
    Ok(())
}
