//! Project initialization for `ddc init`

use camino::{Utf8Path, Utf8PathBuf};
use cell_dialoguer_proto::SelectResult;
use eyre::{Result, eyre};
use include_dir::{Dir, include_dir};
use owo_colors::OwoColorize;
use std::fs;

use crate::cells::dialoguer_client;

/// Embedded project templates
static TEMPLATES: Dir = include_dir!("$CARGO_MANIFEST_DIR/templates");

/// A project template
pub struct Template {
    pub name: &'static str,
    pub description: &'static str,
}

/// Available templates
pub fn available_templates() -> Vec<Template> {
    vec![
        Template {
            name: "minimal",
            description: "Clean slate with basic structure",
        },
        Template {
            name: "blog",
            description: "Blog with posts section",
        },
    ]
}

/// Run the init command
pub async fn run_init(name: String, template_name: Option<String>) -> Result<()> {
    let target_dir = Utf8PathBuf::from(&name);

    // Check if directory already exists
    if target_dir.exists() {
        return Err(eyre!("Directory '{}' already exists", name));
    }

    let templates = available_templates();

    // Select template
    let template = if let Some(ref tpl_name) = template_name {
        // Use provided template
        templates
            .iter()
            .find(|t| t.name == tpl_name)
            .ok_or_else(|| {
                let names: Vec<_> = templates.iter().map(|t| t.name).collect();
                eyre!(
                    "Unknown template: '{}'. Available templates: {}",
                    tpl_name,
                    names.join(", ")
                )
            })?
    } else {
        // Interactive selection via cell
        let client = dialoguer_client()
            .await
            .ok_or_else(|| eyre!("Dialoguer cell not available"))?;

        let items: Vec<String> = templates
            .iter()
            .map(|t| format!("{} - {}", t.name, t.description))
            .collect();

        let result = client
            .select("Choose a starter template:".to_string(), items)
            .await
            .map_err(|e| eyre!("Failed to show template selection: {}", e))?;

        match result {
            Ok(SelectResult::Selected { index }) => &templates[index],
            Ok(SelectResult::Cancelled) => {
                eprintln!("Cancelled");
                std::process::exit(1);
            }
            Err(e) => {
                return Err(eyre!("RoamError during selection: {:?}", e));
            }
        }
    };

    // Get template directory
    let template_dir = TEMPLATES.get_dir(template.name).ok_or_else(|| {
        eyre!(
            "Template '{}' not found in embedded templates",
            template.name
        )
    })?;

    println!("\nCreating '{}'...", name.cyan());

    // Create target directory
    fs::create_dir_all(&target_dir)?;

    // Humanize the site name for use in templates
    let site_name = humanize_name(&name);

    // Copy template files
    let strip_prefix = Utf8Path::new(template.name);
    copy_template_dir(template_dir, &target_dir, &site_name, strip_prefix)?;

    // Git init if not already in a repo
    maybe_git_init(&target_dir);

    // Success message
    println!("\n{} To get started:\n", "Done!".green().bold());
    println!("  cd {}", name);
    println!("  ddc serve --open\n");

    Ok(())
}

/// Recursively copy template directory, replacing placeholders
fn copy_template_dir(
    dir: &Dir,
    target: &Utf8Path,
    site_name: &str,
    strip_prefix: &Utf8Path,
) -> Result<()> {
    for file in dir.files() {
        let full_path = Utf8Path::from_path(file.path())
            .ok_or_else(|| eyre!("Template path is not valid UTF-8: {:?}", file.path()))?;
        let rel_path = full_path.strip_prefix(strip_prefix).unwrap_or(full_path);
        let target_path = target.join(rel_path);

        // Create parent directories
        if let Some(parent) = target_path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Read content and replace placeholders
        let content = std::str::from_utf8(file.contents())
            .map_err(|_| eyre!("Template file {} is not valid UTF-8", rel_path))?;
        let content = content.replace("{{site_name}}", site_name);

        fs::write(&target_path, content)?;
        println!("  {} {}", "✓".green(), rel_path);
    }

    for subdir in dir.dirs() {
        copy_template_dir(subdir, target, site_name, strip_prefix)?;
    }

    Ok(())
}

/// Convert a kebab-case name to Title Case
/// "my-cool-site" -> "My Cool Site"
fn humanize_name(name: &str) -> String {
    name.split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(c) => c.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Initialize git repository if not already inside one
fn maybe_git_init(path: &Utf8Path) {
    // Check if already in a git repo
    let in_git = std::process::Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(path.parent().unwrap_or(path))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if !in_git {
        if let Ok(status) = std::process::Command::new("git")
            .args(["init"])
            .current_dir(path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            if status.success() {
                println!("  {} git init", "✓".green());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_humanize_name() {
        assert_eq!(humanize_name("my-site"), "My Site");
        assert_eq!(humanize_name("my-cool-blog"), "My Cool Blog");
        assert_eq!(humanize_name("site"), "Site");
        assert_eq!(humanize_name(""), "");
    }

    #[test]
    fn test_templates_embedded() {
        // Verify templates are properly embedded
        assert!(TEMPLATES.get_dir("minimal").is_some());
        assert!(TEMPLATES.get_dir("blog").is_some());

        // Verify minimal template has expected files
        // Note: include_dir stores full paths from the root, so we need to include the template name
        let minimal = TEMPLATES.get_dir("minimal").unwrap();
        assert!(minimal.get_file("minimal/.config/dodeca.yaml").is_some());
        assert!(minimal.get_file("minimal/content/_index.md").is_some());
        assert!(minimal.get_file("minimal/templates/base.html").is_some());
    }
}
