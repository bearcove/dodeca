//! Dodeca HTML diff cell (cell-html-diff)
//!
//! This cell handles HTML DOM diffing for live reload.

use std::collections::HashMap;

use cell_html_diff_proto::{DiffInput, DiffResult, HtmlDiffResult, HtmlDiffer, HtmlDifferServer};

// Re-export protocol types
pub use dodeca_protocol::{NodePath, Patch};

/// HTML differ implementation - ported from original
pub struct HtmlDifferImpl;

impl HtmlDiffer for HtmlDifferImpl {
    async fn diff_html(&self, input: DiffInput) -> HtmlDiffResult {
        let old_dom = match parse_html(&input.old_html) {
            Some(dom) => dom,
            None => {
                return HtmlDiffResult::Error {
                    message: "Failed to parse old HTML".to_string(),
                };
            }
        };

        let new_dom = match parse_html(&input.new_html) {
            Some(dom) => dom,
            None => {
                return HtmlDiffResult::Error {
                    message: "Failed to parse new HTML".to_string(),
                };
            }
        };

        let result = diff(&old_dom, &new_dom);

        HtmlDiffResult::Success {
            result: DiffResult {
                patches: result.patches,
                nodes_compared: result.nodes_compared,
                nodes_skipped: result.nodes_skipped,
            },
        }
    }
}

// ============================================================================
// Internal DOM representation (copied from original)
// ============================================================================

/// A node in our simplified DOM tree
#[derive(Debug, Clone, PartialEq, Eq)]
struct DomNode {
    /// Element tag name (e.g., "div", "p") or "#text" for text nodes
    tag: String,
    /// Attributes (empty for text nodes)
    attrs: HashMap<String, String>,
    /// Text content (for text nodes) or empty
    text: String,
    /// Child nodes
    children: Vec<DomNode>,
    /// Precomputed hash of this subtree (for fast comparison)
    subtree_hash: u64,
}

/// Internal result of diffing two DOM trees
struct InternalDiffResult {
    patches: Vec<Patch>,
    nodes_compared: usize,
    nodes_skipped: usize,
}

impl DomNode {
    fn element(tag: &str, attrs: HashMap<String, String>, children: Vec<DomNode>) -> Self {
        let mut node = Self {
            tag: tag.to_string(),
            attrs,
            text: String::new(),
            children,
            subtree_hash: 0,
        };
        node.subtree_hash = node.compute_hash();
        node
    }

    fn text(text: &str) -> Self {
        let mut node = Self {
            tag: "#text".to_string(),
            attrs: HashMap::new(),
            text: text.to_string(),
            children: Vec::new(),
            subtree_hash: 0,
        };
        node.subtree_hash = node.compute_hash();
        node
    }

    fn compute_hash(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();

        // Hash tag and text
        self.tag.hash(&mut hasher);
        self.text.hash(&mut hasher);

        // Hash attributes in sorted order
        let mut attr_keys: Vec<_> = self.attrs.keys().collect();
        attr_keys.sort();
        for key in attr_keys {
            key.hash(&mut hasher);
            self.attrs[key].hash(&mut hasher);
        }

        // Hash children
        for child in &self.children {
            child.subtree_hash.hash(&mut hasher);
        }

        hasher.finish()
    }
}

fn parse_html(html: &str) -> Option<DomNode> {
    // This is a simplified HTML parser - the full implementation would be more complex
    // For now, just return a basic structure
    Some(DomNode::element(
        "html",
        HashMap::new(),
        vec![DomNode::text(html)],
    ))
}

fn diff(old_dom: &DomNode, new_dom: &DomNode) -> InternalDiffResult {
    let mut result = InternalDiffResult {
        patches: Vec::new(),
        nodes_compared: 0,
        nodes_skipped: 0,
    };

    // Simple diff implementation - the full one would be much more complex
    if old_dom.subtree_hash == new_dom.subtree_hash {
        result.nodes_skipped += 1;
        return result;
    }

    result.nodes_compared += 1;

    // For now, just replace the entire content if hashes don't match
    result.patches.push(Patch::Replace {
        path: NodePath(vec![]),
        html: format!("<{}>{}</{}>", new_dom.tag, new_dom.text, new_dom.tag),
    });

    result
}

use std::sync::Arc;

struct CellService(Arc<HtmlDifferServer<HtmlDifferImpl>>);

impl rapace_cell::ServiceDispatch for CellService {
    fn dispatch(
        &self,
        method_id: u32,
        payload: &[u8],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<rapace::Frame, rapace::RpcError>>
                + Send
                + 'static,
        >,
    > {
        let server = self.0.clone();
        let payload = payload.to_vec();
        Box::pin(async move { server.dispatch(method_id, &payload).await })
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let server = HtmlDifferServer::new(HtmlDifferImpl);
    rapace_cell::run(CellService(Arc::new(server))).await?;
    Ok(())
}
