//! Dodeca syntax highlighting plugin using rapace
//!
//! This binary implements the SyntaxHighlightService protocol and provides
//! syntax highlighting functionality via arborium/tree-sitter.

use mod_arborium_proto::SyntaxHighlightServiceServer;

mod syntax_highlight;

dodeca_plugin_runtime::plugin_service!(
    SyntaxHighlightServiceServer<syntax_highlight::SyntaxHighlightImpl>,
    syntax_highlight::SyntaxHighlightImpl
);

dodeca_plugin_runtime::run_plugin!(syntax_highlight::SyntaxHighlightImpl);
