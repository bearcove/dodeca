//! Serialization tests for patch protocol
//!
//! Tests that patches serialize/deserialize correctly with facet-postcard.

use dodeca_protocol::{facet_postcard, NodePath, Patch, ServerMessage};

/// Serialization compatibility tests (no browser needed)
#[test]
fn test_patch_serialization_compatibility() {
    let patches = vec![
        Patch::SetText {
            path: NodePath(vec![0, 1, 2]),
            text: "Hello from server".to_string(),
        },
        Patch::SetAttribute {
            path: NodePath(vec![0]),
            name: "class".to_string(),
            value: "updated".to_string(),
        },
        Patch::Remove {
            path: NodePath(vec![1, 0]),
        },
    ];

    // Patches are now sent as ServerMessage::Patches
    let msg = ServerMessage::Patches(patches.clone());
    let serialized = facet_postcard::to_vec(&msg).unwrap();
    let deserialized: ServerMessage = facet_postcard::from_bytes(&serialized).unwrap();

    match deserialized {
        ServerMessage::Patches(p) => assert_eq!(patches, p),
        _ => panic!("Expected Patches variant"),
    }
    assert!(serialized.len() < 150);
}

#[test]
fn test_all_patch_types_serialize() {
    let patches = vec![
        Patch::Replace { path: NodePath(vec![0]), html: "<p>New</p>".into() },
        Patch::InsertBefore { path: NodePath(vec![1]), html: "<span>Before</span>".into() },
        Patch::InsertAfter { path: NodePath(vec![2]), html: "<span>After</span>".into() },
        Patch::AppendChild { path: NodePath(vec![3]), html: "<div>Child</div>".into() },
        Patch::Remove { path: NodePath(vec![4]) },
        Patch::SetText { path: NodePath(vec![5]), text: "Text".into() },
        Patch::SetAttribute { path: NodePath(vec![6]), name: "id".into(), value: "test".into() },
        Patch::RemoveAttribute { path: NodePath(vec![7]), name: "class".into() },
    ];

    let msg = ServerMessage::Patches(patches.clone());
    let serialized = facet_postcard::to_vec(&msg).unwrap();
    let deserialized: ServerMessage = facet_postcard::from_bytes(&serialized).unwrap();

    match deserialized {
        ServerMessage::Patches(p) => assert_eq!(patches, p),
        _ => panic!("Expected Patches variant"),
    }
}

// NOTE: Browser-based E2E tests removed - they tested the old standalone
// livereload-client WASM module. Now everything goes through dodeca-devtools
// which is integrated into the main build and tested via the serve tests.
