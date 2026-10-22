//! Optional parsed-TOML reuse. Consumers still deserialize and validate every configuration.
//! The companion engine installs the cache; the simulator plugin performs no database I/O.
use serde::de::DeserializeOwned;
use std::sync::OnceLock;

type Parser = fn(&str) -> Result<toml::Value, toml::de::Error>;
static PARSER: OnceLock<Parser> = OnceLock::new();

/// Install the engine's syntax cache before loading configuration.
pub fn install_parser(parser: Parser) {
    let _ = PARSER.set(parser);
}

/// Parse current source, preserving the ordinary TOML diagnostic on a type error.
pub fn from_str<T: DeserializeOwned>(text: &str) -> Result<T, toml::de::Error> {
    match PARSER.get() {
        Some(parser) => parser(text)?.try_into().or_else(|_| toml::from_str(text)),
        None => toml::from_str(text),
    }
}
