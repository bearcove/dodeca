//! Theme name resolution and CSS generation for syntax highlighting

use arborium_theme::theme::builtin;

/// List all available theme names (kebab-case for config files)
pub fn list_available_themes() -> Vec<&'static str> {
    vec![
        "alabaster",
        "ayu-dark",
        "ayu-light",
        "catppuccin-frappe",
        "catppuccin-latte",
        "catppuccin-macchiato",
        "catppuccin-mocha",
        "cobalt2",
        "dayfox",
        "desert256",
        "dracula",
        "ef-melissa-dark",
        "github-dark",
        "github-light",
        "gruvbox-dark",
        "gruvbox-light",
        "kanagawa-dragon",
        "light-owl",
        "lucius-light",
        "melange-dark",
        "melange-light",
        "monokai",
        "nord",
        "one-dark",
        "rose-pine-moon",
        "rustdoc-ayu",
        "rustdoc-dark",
        "rustdoc-light",
        "solarized-dark",
        "solarized-light",
        "tokyo-night",
        "zenburn",
    ]
}

/// Resolve a theme name from kebab-case to the arborium function name
/// Returns error with suggestions if theme not found
pub fn resolve_theme_name(input: &str) -> Result<&'static str, String> {
    let normalized = input.to_lowercase();
    let theme = match normalized.as_str() {
        "alabaster" => "alabaster",
        "ayu-dark" => "ayu_dark",
        "ayu-light" => "ayu_light",
        "catppuccin-frappe" => "catppuccin_frappe",
        "catppuccin-latte" => "catppuccin_latte",
        "catppuccin-macchiato" => "catppuccin_macchiato",
        "catppuccin-mocha" => "catppuccin_mocha",
        "cobalt2" => "cobalt2",
        "dayfox" => "dayfox",
        "desert256" => "desert256",
        "dracula" => "dracula",
        "ef-melissa-dark" => "ef_melissa_dark",
        "github-dark" => "github_dark",
        "github-light" => "github_light",
        "gruvbox-dark" => "gruvbox_dark",
        "gruvbox-light" => "gruvbox_light",
        "kanagawa-dragon" => "kanagawa_dragon",
        "light-owl" => "light_owl",
        "lucius-light" => "lucius_light",
        "melange-dark" => "melange_dark",
        "melange-light" => "melange_light",
        "monokai" => "monokai",
        "nord" => "nord",
        "one-dark" => "one_dark",
        "rose-pine-moon" => "rose_pine_moon",
        "rustdoc-ayu" => "rustdoc_ayu",
        "rustdoc-dark" => "rustdoc_dark",
        "rustdoc-light" => "rustdoc_light",
        "solarized-dark" => "solarized_dark",
        "solarized-light" => "solarized_light",
        "tokyo-night" => "tokyo_night",
        "zenburn" => "zenburn",
        _ => {
            let available = list_available_themes();
            return Err(format!(
                "Unknown theme '{}'. Available themes:\n  {}",
                input,
                available.join("\n  ")
            ));
        }
    };

    Ok(theme)
}

/// Generate CSS for a theme using arborium-theme
pub fn generate_theme_css(theme_name: &str) -> Result<String, String> {
    let resolved = resolve_theme_name(theme_name)?;

    // Call the appropriate builtin theme function and convert to CSS
    let theme = match resolved {
        "alabaster" => builtin::alabaster(),
        "ayu_dark" => builtin::ayu_dark(),
        "ayu_light" => builtin::ayu_light(),
        "catppuccin_frappe" => builtin::catppuccin_frappe(),
        "catppuccin_latte" => builtin::catppuccin_latte(),
        "catppuccin_macchiato" => builtin::catppuccin_macchiato(),
        "catppuccin_mocha" => builtin::catppuccin_mocha(),
        "cobalt2" => builtin::cobalt2(),
        "dayfox" => builtin::dayfox(),
        "desert256" => builtin::desert256(),
        "dracula" => builtin::dracula(),
        "ef_melissa_dark" => builtin::ef_melissa_dark(),
        "github_dark" => builtin::github_dark(),
        "github_light" => builtin::github_light(),
        "gruvbox_dark" => builtin::gruvbox_dark(),
        "gruvbox_light" => builtin::gruvbox_light(),
        "kanagawa_dragon" => builtin::kanagawa_dragon(),
        "light_owl" => builtin::light_owl(),
        "lucius_light" => builtin::lucius_light(),
        "melange_dark" => builtin::melange_dark(),
        "melange_light" => builtin::melange_light(),
        "monokai" => builtin::monokai(),
        "nord" => builtin::nord(),
        "one_dark" => builtin::one_dark(),
        "rose_pine_moon" => builtin::rose_pine_moon(),
        "rustdoc_ayu" => builtin::rustdoc_ayu(),
        "rustdoc_dark" => builtin::rustdoc_dark(),
        "rustdoc_light" => builtin::rustdoc_light(),
        "solarized_dark" => builtin::solarized_dark(),
        "solarized_light" => builtin::solarized_light(),
        "tokyo_night" => builtin::tokyo_night(),
        "zenburn" => builtin::zenburn(),
        _ => unreachable!("resolve_theme_name returned invalid theme"),
    };

    // Generate CSS with `pre` selector to scope themes to code blocks
    let css = theme.to_css("pre");

    Ok(css)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_theme_name_valid() {
        assert_eq!(resolve_theme_name("tokyo-night").unwrap(), "tokyo_night");
        assert_eq!(resolve_theme_name("github-light").unwrap(), "github_light");
        assert_eq!(
            resolve_theme_name("catppuccin-mocha").unwrap(),
            "catppuccin_mocha"
        );
    }

    #[test]
    fn test_resolve_theme_name_invalid() {
        let result = resolve_theme_name("invalid-theme");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("Unknown theme"));
        assert!(err.contains("Available themes"));
    }

    #[test]
    fn test_generate_theme_css() {
        let css = generate_theme_css("tokyo-night").unwrap();
        assert!(!css.is_empty());
        // CSS should contain `pre` selector and custom element selectors
        assert!(css.contains("pre"));
        assert!(css.contains("a-"));
    }

    #[test]
    fn test_generate_both_themes() {
        // Test that both default themes generate valid CSS
        let light_css = generate_theme_css("github-light").unwrap();
        let dark_css = generate_theme_css("tokyo-night").unwrap();

        assert!(!light_css.is_empty());
        assert!(!dark_css.is_empty());

        // Both should contain arborium element selectors
        assert!(light_css.contains("a-"));
        assert!(dark_css.contains("a-"));

        // Print sample to verify format (only in --nocapture mode)
        println!("\n=== Light theme CSS sample (first 200 chars) ===");
        println!("{}", &light_css.chars().take(200).collect::<String>());
        println!("\n=== Dark theme CSS sample (first 200 chars) ===");
        println!("{}", &dark_css.chars().take(200).collect::<String>());
    }

    #[test]
    fn test_list_available_themes_not_empty() {
        let themes = list_available_themes();
        assert!(!themes.is_empty());
        assert!(themes.contains(&"tokyo-night"));
        assert!(themes.contains(&"github-light"));
    }
}
