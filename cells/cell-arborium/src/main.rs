//! Dodeca syntax highlighting plugin using rapace
//!
//! This binary implements the SyntaxHighlightService protocol and provides
//! syntax highlighting functionality via arborium/tree-sitter.

use cell_arborium_proto::SyntaxHighlightServiceServer;

mod syntax_highlight;

dodeca_cell_runtime::cell_service!(
    SyntaxHighlightServiceServer<syntax_highlight::SyntaxHighlightImpl>,
    syntax_highlight::SyntaxHighlightImpl
);

dodeca_cell_runtime::run_cell!(syntax_highlight::SyntaxHighlightImpl);
