use super::code_units::{self, CodeUnits};
use std::path::Path;

pub struct Lang {
    pub extensions: &'static [&'static str],
    pub grammar: fn() -> arborium_tree_sitter::LanguageFn,
    pub extract: fn(&Path, &str) -> CodeUnits,
}

pub fn for_ext(ext: &str) -> Option<&'static Lang> {
    LANGUAGES.iter().find(|l| l.extensions.contains(&ext))
}

pub static LANGUAGES: &[Lang] = &[
    Lang {
        extensions: &["rs"],
        grammar: arborium_rust::language,
        extract: code_units::extract_rust,
    },
    Lang {
        extensions: &["swift"],
        grammar: arborium_swift::language,
        extract: code_units::extract_swift,
    },
    Lang {
        extensions: &["go"],
        grammar: arborium_go::language,
        extract: code_units::extract_go,
    },
    Lang {
        extensions: &["java"],
        grammar: arborium_java::language,
        extract: code_units::extract_java,
    },
    Lang {
        extensions: &["py"],
        grammar: arborium_python::language,
        extract: code_units::extract_python,
    },
    Lang {
        extensions: &["ts", "tsx", "js", "jsx", "mts", "cts"],
        grammar: arborium_typescript::language,
        extract: code_units::extract_typescript,
    },
    Lang {
        extensions: &["php"],
        grammar: arborium_php::language,
        extract: code_units::extract_php,
    },
    Lang {
        extensions: &["c", "h"],
        grammar: arborium_c::language,
        extract: code_units::extract_c,
    },
    Lang {
        extensions: &["cpp", "cc", "cxx", "hpp"],
        grammar: arborium_cpp::language,
        extract: code_units::extract_cpp,
    },
    Lang {
        extensions: &["rb"],
        grammar: arborium_ruby::language,
        extract: code_units::extract_ruby,
    },
    Lang {
        extensions: &["r", "R"],
        grammar: arborium_r::language,
        extract: code_units::extract_r,
    },
    Lang {
        extensions: &["dart"],
        grammar: arborium_dart::language,
        extract: code_units::extract_dart,
    },
    Lang {
        extensions: &["lua"],
        grammar: arborium_lua::language,
        extract: code_units::extract_lua,
    },
    Lang {
        extensions: &["asm", "s", "S"],
        grammar: arborium_asm::language,
        extract: code_units::extract_asm,
    },
    Lang {
        extensions: &["pl", "pm"],
        grammar: arborium_perl::language,
        extract: code_units::extract_perl,
    },
    Lang {
        extensions: &["hs", "lhs"],
        grammar: arborium_haskell::language,
        extract: code_units::extract_haskell,
    },
    Lang {
        extensions: &["ex", "exs"],
        grammar: arborium_elixir::language,
        extract: code_units::extract_elixir,
    },
    Lang {
        extensions: &["erl", "hrl"],
        grammar: arborium_erlang::language,
        extract: code_units::extract_erlang,
    },
    Lang {
        extensions: &["clj", "cljs", "cljc", "edn"],
        grammar: arborium_clojure::language,
        extract: code_units::extract_clojure,
    },
    Lang {
        extensions: &["fs", "fsi", "fsx"],
        grammar: arborium_fsharp::language,
        extract: code_units::extract_fsharp,
    },
    Lang {
        extensions: &["vb", "vbs"],
        grammar: arborium_vb::language,
        extract: code_units::extract_vb,
    },
    Lang {
        extensions: &["cob", "cbl", "cpy"],
        grammar: arborium_cobol::language,
        extract: code_units::extract_cobol,
    },
    Lang {
        extensions: &["jl"],
        grammar: arborium_julia::language,
        extract: code_units::extract_julia,
    },
    Lang {
        extensions: &["d"],
        grammar: arborium_d::language,
        extract: code_units::extract_d,
    },
    Lang {
        extensions: &["ps1", "psm1", "psd1"],
        grammar: arborium_powershell::language,
        extract: code_units::extract_powershell,
    },
    Lang {
        extensions: &["cmake"],
        grammar: arborium_cmake::language,
        extract: code_units::extract_cmake,
    },
    Lang {
        extensions: &["ml", "mli"],
        grammar: arborium_ocaml::language,
        extract: code_units::extract_ocaml,
    },
    Lang {
        extensions: &["sh", "bash", "zsh"],
        grammar: arborium_bash::language,
        extract: code_units::extract_bash,
    },
    Lang {
        extensions: &["nix"],
        grammar: arborium_nix::language,
        extract: code_units::extract_nix,
    },
    Lang {
        extensions: &["lean"],
        grammar: arborium_lean::language,
        extract: code_units::extract_lean,
    },
    Lang {
        extensions: &["svelte"],
        grammar: arborium_svelte::language,
        extract: code_units::extract_svelte,
    },
    Lang {
        extensions: &["yml", "yaml"],
        grammar: arborium_yaml::language,
        extract: code_units::extract_config,
    },
    Lang {
        extensions: &["json5"],
        grammar: arborium_typescript::language,
        extract: code_units::extract_config,
    },
    Lang {
        extensions: &["mat"],
        grammar: arborium_matlab::language,
        extract: code_units::extract_matlab,
    },
];
