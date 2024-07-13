use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use clap::parser::ValueSource;
use clap::ArgMatches;
use rust_apt::config::Config as AptConfig;
use serde::Deserialize;

use crate::cli::Commands;
use crate::colors::{RatStyle, Style, Theme};

/// Represents different file and directory paths
pub enum Paths {
	/// The Archive dir holds packages.
	/// Default dir `/var/cache/apt/archives/`
	Archive,
	/// The Lists dir hold package lists from `update` command.
	/// Default dir `/var/lib/apt/lists/`
	Lists,
	/// The main Source List.
	/// Default file `/etc/apt/sources.list`
	SourceList,
	/// The Sources parts directory
	/// Default dir `/etc/apt/sources.list.d/`
	SourceParts,
	/// Nala Sources file is generated from the `fetch` command.
	/// Default file `/etc/apt/sources.list.d/nala-sources.list`
	NalaSources,
}

impl Paths {
	pub fn path(&self) -> &'static str {
		match self {
			Paths::Archive => "Dir::Cache::Archives",
			Paths::Lists => "Dir::State::Lists",
			Paths::SourceList => "Dir::Etc::sourcelist",
			Paths::SourceParts => "Dir::Etc::sourceparts",
			Paths::NalaSources => "/etc/apt/sources.list.d/nala.sources",
		}
	}

	pub fn default_path(&self) -> &'static str {
		match self {
			Paths::Archive => "/var/cache/apt/archives/",
			Paths::Lists => "/var/lib/apt/lists/",
			Paths::SourceList => "/etc/apt/sources.list",
			Paths::SourceParts => "/etc/apt/sources.list.d/",
			Paths::NalaSources => "/etc/apt/sources.list.d/nala.sources",
		}
	}
}

#[derive(Deserialize, Debug, PartialEq)]
pub enum Switch {
	Always,
	Never,
	Auto,
}

impl Default for Switch {
	/// The default configuration for Nala.
	fn default() -> Switch { Switch::Auto }
}

#[derive(Deserialize, Debug)]
/// Configuration struct
pub struct Config {
	#[serde(rename(deserialize = "Nala"), default)]
	pub nala_map: HashMap<String, bool>,

	#[serde(rename(deserialize = "String"), default)]
	vec_map: HashMap<String, Vec<String>>,

	#[serde(rename(deserialize = "Theme"), default)]
	theme: HashMap<Theme, Style>,

	// The following fields are not used with serde
	#[serde(skip)]
	pub can_color: Switch,

	#[serde(skip)]
	pub apt: AptConfig,

	#[serde(skip)]
	pub auto: Option<u8>,

	#[serde(skip)]
	/// The command that is being run
	pub command: String,
}

impl Default for Config {
	/// The default configuration for Nala.
	fn default() -> Config {
		let mut config = Config {
			nala_map: HashMap::new(),
			theme: HashMap::new(),
			vec_map: HashMap::new(),
			can_color: Switch::Auto,
			auto: None,
			apt: AptConfig::new(),
			command: "Command Not Given Yet".to_string(),
		};
		config.set_default_theme();
		config
	}
}

impl Config {
	pub fn new(conf_file: &Path) -> Result<Config> {
		// Try to read the entire config file and map it.
		// Return an empty config and print a warning on failure.
		let mut map = Self::read_config(conf_file)?;
		map.set_default_theme();
		Ok(map)
	}

	pub fn set_default_theme(&mut self) {
		for theme in [
			Theme::Primary,
			Theme::Secondary,
			Theme::Regular,
			Theme::Highlight,
			Theme::ProgressFilled,
			Theme::ProgressUnfilled,
			Theme::Notice,
			Theme::Warning,
			Theme::Error,
		] {
			if self.theme.contains_key(&theme) {
				continue;
			}
			self.theme.insert(theme, theme.default_style());
		}
	}

	pub fn rat_style(&self, theme: Theme) -> RatStyle {
		self.theme.get(&theme).unwrap_or(&Style::default()).to_rat()
	}

	pub fn color(&self, theme: Theme, string: &str) -> String {
		if self.can_color != Switch::Never {
			if let Some(theme) = self.theme.get(&theme) {
				return format!("{theme}{string}\x1b[0m");
			}
		}
		string.to_string()
	}

	/// Hightlights the string according to configuration.
	pub fn highlight(&self, string: &str) -> String { self.color(Theme::Highlight, string) }

	/// Color the version according to configuration.
	pub fn color_ver(&self, string: &str) -> String {
		format!(
			"{}{}{}",
			self.highlight("("),
			self.color(Theme::Secondary, string),
			self.highlight(")")
		)
	}

	/// Print a notice to stderr
	pub fn stderr(&self, theme: Theme, string: &str) {
		let header = match theme {
			Theme::Error => "Error:",
			Theme::Warning => "Warning:",
			Theme::Notice => "Notice:",
			_ => panic!("'{theme:?}' is not a valid stderr!"),
		};
		eprintln!("{} {string}", self.color(theme, header));
	}

	/// Read and Return the entire toml configuration file
	fn read_config(conf_file: &Path) -> Result<Config> {
		let conf = fs::read_to_string(conf_file)
			.with_context(|| format!("Failed to read {}, using defaults", conf_file.display()))?;

		let config: Config = toml::from_str(&conf)
			.with_context(|| format!("Failed to parse {}, using defaults", conf_file.display()))?;

		Ok(config)
	}

	/// Load configuration with the command line arguments
	pub fn load_args(&mut self, args: &ArgMatches, commands: Option<Commands>) {
		if let Some(Commands::Fetch(opts)) = commands {
			self.auto = opts.auto;
		};

		for id in args.ids() {
			let key = id.as_str().to_string();
			// Don't do anything if the option wasn't specifically passed
			if Some(ValueSource::CommandLine) != args.value_source(&key) {
				continue;
			}

			if let Ok(Some(value)) = args.try_get_one::<bool>(&key) {
				self.nala_map.insert(key, *value);
				continue;
			}

			if let Ok(Some(value)) = args.try_get_occurrences::<String>(&key) {
				self.vec_map.insert(key, value.flatten().cloned().collect());
			}
		}

		// If Debug is there we can print the whole thing.
		if self.debug() {
			dbg!(&self);
		}
	}

	/// Get a bool from the configuration.
	pub fn get_bool(&self, key: &str, default: bool) -> bool {
		*self.nala_map.get(key).unwrap_or(&default)
	}

	/// Get a single str from the configuration.
	pub fn get_str(&self, key: &str) -> Option<&str> {
		self.vec_map.get(key)?.first().map(|x| x.as_str())
	}

	/// Get a file from the configuration based on the Path enum.
	pub fn get_file(&self, file: &Paths) -> String {
		match file {
			// For now NalaSources is hard coded.
			Paths::NalaSources => file.path().to_string(),
			_ => self.apt.file(file.path(), file.default_path()),
		}
	}

	/// Get a path from the configuration based on the Path enum.
	pub fn get_path(&self, dir: &Paths) -> String {
		match dir {
			// For now NalaSources is hard coded.
			Paths::NalaSources => dir.path().to_string(),
			// Everything else should be an Apt Path
			_ => self.apt.dir(dir.path(), dir.default_path()),
		}
	}

	/// Get the package names that were passed as arguments
	pub fn pkg_names(&self) -> Option<&Vec<String>> { self.vec_map.get("pkg_names") }

	/// Get the countries that were passed as arguments
	pub fn countries(&self) -> Option<&Vec<String>> { self.vec_map.get("countries") }

	/// Return true if debug is enabled
	pub fn debug(&self) -> bool { self.get_bool("debug", false) }

	/// Return true if verbose or debug is enabled
	pub fn verbose(&self) -> bool { self.get_bool("verbose", self.debug()) }
}
