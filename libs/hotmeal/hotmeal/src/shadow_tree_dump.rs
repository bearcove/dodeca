//! Shadow tree pretty-printing for debugging.
//!
//! This module is only compiled when the `tracing` feature is enabled or during tests.

use crate::diff::{HtmlNodeKind, HtmlTreeTypes, ShadowTree};
use cinereus::{NodeData, indextree::NodeId};

/// Extract short node label like "n1" from NodeId debug output.
fn node_id_short(node_id: NodeId) -> String {
    let debug = format!("{:?}", node_id);
    let Some(start) = debug.find("index1: ") else {
        return debug;
    };
    let digits = &debug[start + "index1: ".len()..];
    let value: String = digits.chars().take_while(|c| c.is_ascii_digit()).collect();
    if value.is_empty() {
        debug
    } else {
        format!("n{}", value)
    }
}

/// Helper for pretty-printing a shadow tree.
pub(crate) struct ShadowTreeDump<'a, 'b> {
    pub(crate) shadow: &'b ShadowTree<'a>,
    pub(crate) highlights: &'b [(NodeId, &'static str, &'static str)],
}

impl<'a, 'b> std::fmt::Display for ShadowTreeDump<'a, 'b> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (slot_num, slot_node) in self
            .shadow
            .super_root
            .children(&self.shadow.arena)
            .enumerate()
        {
            writeln!(f, "Slot {}:", slot_num)?;
            for content in slot_node.children(&self.shadow.arena) {
                self.fmt_node(f, content, 1)?;
            }
        }
        Ok(())
    }
}

impl<'a, 'b> ShadowTreeDump<'a, 'b> {
    fn highlight_for(&self, node_id: NodeId) -> Option<(&'static str, &'static str)> {
        self.highlights
            .iter()
            .find(|(id, _, _)| *id == node_id)
            .map(|(_, color, label)| (*color, *label))
    }

    fn fmt_node(
        &self,
        f: &mut std::fmt::Formatter<'_>,
        node: NodeId,
        depth: usize,
    ) -> std::fmt::Result {
        let indent = "  ".repeat(depth);
        let node_label = node_id_short(node);
        let prefix = format!("{indent}[{node_label}] ");
        let data = &self.shadow.arena[node].get();

        let highlight = self.highlight_for(node);
        let (hl_start, hl_end, hl_label) = if let Some((color, label)) = highlight {
            (color, "\x1b[0m", label)
        } else {
            ("", "", "")
        };
        let badge = if hl_label.is_empty() {
            String::new()
        } else {
            format!(" {hl_start}<{hl_label}>{hl_end}")
        };

        match &data.kind {
            HtmlNodeKind::Element(tag, _ns) => {
                let tag_display = if hl_start.is_empty() {
                    tag.to_string()
                } else {
                    format!("{hl_start}{tag}{hl_end}")
                };
                writeln!(f, "{prefix}<{tag_display}>{badge}")?;
                for child in node.children(&self.shadow.arena) {
                    self.fmt_node(f, child, depth + 1)?;
                }
                writeln!(f, "{prefix}</{tag_display}>")?;
            }
            HtmlNodeKind::Text => {
                let text = data.text.as_deref().unwrap_or("");
                writeln!(f, "{prefix}TEXT: {text:?}{badge}")?;
                // Placeholders are TEXT nodes but may have children
                for child in node.children(&self.shadow.arena) {
                    self.fmt_node(f, child, depth + 1)?;
                }
            }
            HtmlNodeKind::Comment => {
                let text = data.text.as_deref().unwrap_or("");
                writeln!(f, "{prefix}COMMENT: {text:?}{badge}")?;
                // Slots are COMMENT nodes with children
                for child in node.children(&self.shadow.arena) {
                    self.fmt_node(f, child, depth + 1)?;
                }
            }
        }
        Ok(())
    }
}

impl<'a> ShadowTree<'a> {
    /// Pretty-print the shadow tree for debugging.
    #[allow(dead_code)]
    pub(crate) fn debug_print_tree(&self, title: &str) {
        self.debug_print_tree_with_highlights(title, &[]);
    }

    /// Pretty-print the shadow tree with highlighted nodes.
    #[allow(dead_code)]
    pub(crate) fn debug_print_tree_with_highlights(
        &self,
        _title: &str,
        _highlights: &[(NodeId, &'static str, &'static str)],
    ) {
        crate::debug!(
            "=== {} ===\n{}",
            _title,
            ShadowTreeDump {
                shadow: self,
                highlights: _highlights
            }
        );
    }
}
