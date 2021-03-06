//! This crate is a library for generating structs based on a config
//! file at build time. It is intended for use in a `build.rs` file
//! so should be included in your `[build-dependencies]`.
//!
//! ```toml
//! [build-dependencies.config_struct]
//! version = "~0.3.0"
//! features = ["toml-parsing"]
//! ```
//!
//! By default, `config_struct` is markup-language-agnostic, so
//! include the relevant feature for whatever language your config
//! file is written in. Choices are:
//!
//! 1.  `json-parsing`
//! 2.  `ron-parsing`
//! 3.  `toml-parsing`
//! 4.  `yaml-parsing`
//!
//! Only `toml-parsing` is included by default, so be sure to specify
//! the features you need in your `Cargo.toml` file.
//!
//! # Examples
//!
//! ```rust,no_run
//! // build.rs
//! use config_struct::{Error, StructOptions};
//!
//! fn main() -> Result<(), Error> {
//!     config_struct::create_config(
//!         "config.toml",
//!         "src/config.rs",
//!         &StructOptions::default())
//! }
//! ```
//!
//! The above build script will take the following `config.toml` file and generate
//! a `config.rs` like the following:
//!
//! ```toml
//! # config.toml
//! name = "Application"
//! version = 5
//! features = [
//!     "one",
//!     "two",
//!     "three"
//! ]
//! ```
//!
//! ```rust,no_run
//! // config.rs
//! // ...
//! use std::borrow::Cow;
//!
//! #[derive(Debug, Clone)]
//! #[allow(non_camel_case_types)]
//! pub struct Config {
//!     pub features: Cow<'static, [Cow<'static, str>]>,
//!     pub name: Cow<'static, str>,
//!     pub version: i64,
//! }
//!
//! pub const CONFIG: Config = Config {
//!     features: Cow::Borrowed(&[Cow::Borrowed("one"), Cow::Borrowed("two"), Cow::Borrowed("three")]),
//!     name: Cow::Borrowed("Application"),
//!     version: 5,
//! };
//! ```
//!
//! Strings and arrays are represented by `Cow` types, which allows
//! the entire Config struct to be either heap allocated at runtime,
//! or a compile time constant, as shown above.

#[cfg(feature = "json-parsing")]
mod json_parsing;

#[cfg(feature = "ron-parsing")]
mod ron_parsing;

#[cfg(feature = "toml-parsing")]
mod toml_parsing;

#[cfg(feature = "yaml-parsing")]
mod yaml_parsing;

mod error;
mod format;
mod generation;
mod load_fns;
mod options;
mod parsing;
mod validation;
mod value;

#[cfg(not(any(
    feature = "json-parsing",
    feature = "ron-parsing",
    feature = "toml-parsing",
    feature = "yaml-parsing"
)))]
compile_error!("The config_struct crate requires at least one parsing feature to be enabled:\n {json-parsing, ron-parsing, toml-parsing, yaml-parsing}");

use std::path::Path;

use crate::value::GenericStruct;

pub use crate::{
    error::{Error, GenerationError, OptionsError},
    format::Format,
    options::{DynamicLoading, FloatSize, IntSize, SerdeSupport, StructOptions},
};

/// Generate Rust source code defining structs based on a config file.
///
/// The format of
/// the config file will be auto-detected from its extension.
///
/// # Examples
/// ```rust,no_run
/// # fn main() -> Result<(), config_struct::Error> {
/// let code = config_struct::generate_config("config.toml", &Default::default())?;
/// assert!(code.contains("pub struct Config"));
/// # Ok(())
/// # }
/// ```
pub fn generate_config<P: AsRef<Path>>(
    filepath: P,
    options: &StructOptions,
) -> Result<String, Error> {
    let format = Format::from_filename(filepath.as_ref())?;

    generate_config_with_format(format, filepath, options)
}

/// Generate Rust source code defining structs based on a config file
/// of an explicit format.
///
/// # Examples
/// ```rust,no_run
/// # fn main() -> Result<(), config_struct::Error> {
/// use config_struct::{Format, StructOptions};
///
/// let code = config_struct::generate_config_with_format(
///     Format::Toml,
///     "config.toml",
///     &StructOptions::default())?;
///
/// assert!(code.contains("pub struct Config"));
/// # Ok(())
/// # }
/// ```
pub fn generate_config_with_format<P: AsRef<Path>>(
    format: Format,
    filepath: P,
    options: &StructOptions,
) -> Result<String, Error> {
    let path = filepath.as_ref();
    let source = std::fs::read_to_string(path)?;
    let output = generate_config_from_source_with_filepath(format, &source, options, Some(path))?;

    Ok(output)
}

/// Generate Rust source code defining structs from a config string
/// in some specified format.
///
/// # Examples
/// ```rust
/// use config_struct::{StructOptions, Format};
///
/// let code = config_struct::generate_config_from_source(
///     Format::Toml,
///     "number = 100  # This is valid TOML.",
///     &StructOptions::default()).unwrap();
///
/// assert!(code.contains("pub struct Config"));
/// assert!(code.contains("pub number: i64"));
/// assert!(code.contains("number: 100"));
/// ```
pub fn generate_config_from_source<S: AsRef<str>>(
    format: Format,
    source: S,
    options: &StructOptions,
) -> Result<String, GenerationError> {
    generate_config_from_source_with_filepath(format, source.as_ref(), options, None)
}

fn generate_config_from_source_with_filepath(
    format: Format,
    source: &str,
    options: &StructOptions,
    filepath: Option<&Path>,
) -> Result<String, GenerationError> {
    options.validate()?;

    let config = {
        let mut root_struct: GenericStruct = match format {
            #[cfg(feature = "json-parsing")]
            Format::Json => json_parsing::parse_json(source, options)?,

            #[cfg(feature = "ron-parsing")]
            Format::Ron => ron_parsing::parse_ron(source, options)?,

            #[cfg(feature = "toml-parsing")]
            Format::Toml => toml_parsing::parse_toml(source, options)?,

            #[cfg(feature = "yaml-parsing")]
            Format::Yaml => yaml_parsing::parse_yaml(source, options)?,
        };
        root_struct.struct_name = options.struct_name.clone();
        root_struct
    };

    validation::validate_struct(&config)?;

    let mut code = String::new();

    const HEADER: &str = "#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(dead_code)]

use std::borrow::Cow;\n\n";
    code.push_str(HEADER);

    let structs = generation::generate_structs(&config, options);
    code.push_str(&structs);

    let requires_const =
        options.generate_load_fns && options.dynamic_loading != DynamicLoading::Always;

    let struct_name = &options.struct_name;
    let const_name = &options.real_const_name();

    if options.generate_const || requires_const {
        code.push_str(&format!(
            "pub const {}: {} = {};\n",
            const_name,
            struct_name,
            generation::struct_value_string(&config, 0, options.max_array_size)
        ));
    }

    if options.generate_load_fns {
        let filepath = filepath.ok_or(GenerationError::MissingFilePath);

        let dynamic_impl =
            filepath.map(|path| load_fns::dynamic_load_impl(format, struct_name, path));

        let static_impl = load_fns::static_load_impl(struct_name, const_name);

        let impl_string = match options.dynamic_loading {
            DynamicLoading::Always => dynamic_impl?,
            DynamicLoading::Never => static_impl,
            DynamicLoading::DebugOnly => format!(
                "
#[cfg(debug_assertions)]
{}

#[cfg(not(debug_assertions))]
{}
",
                dynamic_impl?, static_impl,
            ),
        };

        code.push_str(&impl_string);
    }

    Ok(code)
}

/// Generate a Rust module containing struct definitions based on a
/// given config file.
///
/// The format of the config is auto-detected from its filename
/// extension.
///
/// # Examples
///
/// ```rust,no_run
/// # fn main() -> Result<(), config_struct::Error> {
/// use config_struct::StructOptions;
///
/// config_struct::create_config("config.toml", "src/config.rs", &StructOptions::default())?;
/// # Ok(())
/// # }
/// ```
pub fn create_config<SrcPath: AsRef<Path>, DstPath: AsRef<Path>>(
    filepath: SrcPath,
    destination: DstPath,
    options: &StructOptions,
) -> Result<(), Error> {
    let output = generate_config(filepath, options)?;
    ensure_destination(destination.as_ref(), options)?;
    write_destination(destination.as_ref(), output, options)?;

    Ok(())
}

/// Generate a Rust module containing struct definitions based on a
/// given config file with an explicitly specified format.
///
/// # Examples
///
/// ```rust,no_run
/// # fn main() -> Result<(), config_struct::Error> {
/// use config_struct::{Format, StructOptions};
///
/// config_struct::create_config_with_format(
///     Format::Toml,
///     "config.toml",
///     "src/config.rs",
///     &StructOptions::default())?;
/// # Ok(())
/// # }
/// ```
pub fn create_config_with_format<SrcPath: AsRef<Path>, DstPath: AsRef<Path>>(
    format: Format,
    filepath: SrcPath,
    destination: DstPath,
    options: &StructOptions,
) -> Result<(), Error> {
    let output = generate_config_with_format(format, filepath, options)?;
    ensure_destination(destination.as_ref(), options)?;
    write_destination(destination.as_ref(), output, options)?;

    Ok(())
}

/// Generate a Rust module containing struct definitions from a
/// config string in some specified format.
///
/// # Examples
///
/// ```rust,no_run
/// # fn main() -> Result<(), config_struct::Error> {
/// use config_struct::{Format, StructOptions};
///
/// config_struct::create_config_from_source(
///     Format::Toml,
///     "number = 100  # This is valid TOML.",
///     "src/config.rs",
///     &StructOptions::default())?;
/// # Ok(())
/// # }
/// ```
pub fn create_config_from_source<S: AsRef<str>, P: AsRef<Path>>(
    format: Format,
    source: S,
    destination: P,
    options: &StructOptions,
) -> Result<(), Error> {
    let output = generate_config_from_source(format, source, options)?;
    ensure_destination(destination.as_ref(), options)?;
    write_destination(destination.as_ref(), output, options)?;

    Ok(())
}

fn ensure_destination(path: &Path, options: &StructOptions) -> Result<(), Error> {
    if options.create_dirs {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
    }

    Ok(())
}

fn write_destination(
    destination: &Path,
    output: String,
    options: &StructOptions,
) -> Result<(), std::io::Error> {
    let should_write = if options.write_only_if_changed {
        let existing = std::fs::read_to_string(destination);
        match existing {
            Ok(existing) => existing != output,
            Err(_) => true,
        }
    } else {
        true
    };

    if should_write {
        std::fs::write(destination, output)
    } else {
        Ok(())
    }
}
