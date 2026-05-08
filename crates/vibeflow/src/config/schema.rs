//! `serde::Deserialize`-derivable schema types matching the on-disk TOML
//! layout. All fields are `Option<T>` for partial-parse tolerance — missing
//! keys map to `None` here and are filled with defaults by `Config::load`.
