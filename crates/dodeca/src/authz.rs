//! Editor authorization.
//!
//! Who may edit the wiki in-browser. **Fail closed**: there is no implicit
//! fallback — an editor must be named explicitly, by email or by group, in the
//! site's `auth` config. No `auth` config, or empty allowlists, means nobody
//! edits (the deployment is read-only). Anonymous requests (no forwarded
//! identity) are never editors.

use cell_http_proto::Identity;
use dodeca_config::AuthConfig;

/// Is this identity allowed to edit, per the site's auth config?
///
/// True iff the identity's user id is in `auth.editors` (case-insensitive) or
/// one of the identity's groups is in `auth.editor_groups`. Anything else — no
/// identity, no config, empty lists, no overlap — is false. (`email` is the git
/// author email, not an authz key.)
pub fn is_editor(identity: Option<&Identity>, auth: &AuthConfig) -> bool {
    let Some(identity) = identity else {
        return false;
    };

    let by_user = auth
        .editors
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(&identity.user));

    let by_group = auth
        .editor_groups
        .as_deref()
        .unwrap_or_default()
        .iter()
        .any(|allowed| identity.groups.iter().any(|g| g == allowed));

    by_user || by_group
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(user: &str, groups: &[&str]) -> Identity {
        Identity {
            user: user.to_string(),
            email: format!("{user}@example.com"),
            name: "Name".to_string(),
            groups: groups.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn anonymous_is_never_an_editor() {
        let auth = AuthConfig {
            editors: Some(vec!["amos".to_string()]),
            editor_groups: Some(vec!["editors".to_string()]),
        };
        assert!(!is_editor(None, &auth));
    }

    #[test]
    fn empty_config_fails_closed() {
        let auth = AuthConfig {
            editors: None,
            editor_groups: None,
        };
        assert!(!is_editor(Some(&identity("amos", &["editors"])), &auth));
    }

    #[test]
    fn matches_by_user_case_insensitively() {
        let auth = AuthConfig {
            editors: Some(vec!["Amos".to_string()]),
            editor_groups: None,
        };
        assert!(is_editor(Some(&identity("amos", &[])), &auth));
        assert!(!is_editor(Some(&identity("other", &[])), &auth));
    }

    #[test]
    fn matches_by_group() {
        let auth = AuthConfig {
            editors: None,
            editor_groups: Some(vec!["editors".to_string()]),
        };
        assert!(is_editor(
            Some(&identity("x@y.z", &["readers", "editors"])),
            &auth
        ));
        assert!(!is_editor(Some(&identity("x@y.z", &["readers"])), &auth));
    }
}
