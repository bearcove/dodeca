//! Dodeca dialoguer cell
//!
//! Provides interactive terminal prompts using the dialoguer crate.

use cell_dialoguer_proto::{ConfirmResult, Dialoguer, DialoguerDispatcher, SelectResult};
use dialoguer::{Confirm, Select, theme::ColorfulTheme};
use dodeca_cell_runtime::run_cell;

/// Dialoguer service implementation
#[derive(Clone)]
pub struct DialoguerImpl;

impl Dialoguer for DialoguerImpl {
    async fn select(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        prompt: String,
        items: Vec<String>,
    ) -> SelectResult {
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

    async fn confirm(
        &self,
        _cx: &dodeca_cell_runtime::Context,
        prompt: String,
        default: bool,
    ) -> ConfirmResult {
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_cell!("dialoguer", |_handle| DialoguerDispatcher::new(
        DialoguerImpl
    ))
}
