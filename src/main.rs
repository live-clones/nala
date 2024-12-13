use std::path::Path;
use std::process::ExitCode;

use anyhow::{bail, Result};
use clap::{ArgMatches, CommandFactory, FromArgMatches};
use cli::Commands;
use colors::Theme;
use config::Paths;
use deb::DebFile;
use history::history;
use rust_apt::error::AptErrors;
use rust_apt::{new_cache, PackageSort};

mod cli;
mod fetch;
mod history;
mod list;
mod show;
mod update;

mod clean;
mod colors;
mod config;
mod deb;
mod download;
mod dpkg;
mod fs;
mod glob;
mod hashsum;
mod install;
mod summary;
mod table;
mod tui;
mod upgrade;
mod util;

use crate::clean::clean;
use crate::cli::NalaParser;
use crate::config::Config;
use crate::download::download;
use crate::fetch::fetch;
use crate::install::install;
use crate::list::{list, search};
use crate::show::show;
use crate::update::update;
use crate::upgrade::upgrade;

fn main() -> ExitCode {
	let (args, derived, mut config) = match get_config() {
		Ok(conf) => conf,
		Err(err) => {
			eprintln!("\x1b[1;91mError:\x1b[0m {err:?}");
			return ExitCode::FAILURE;
		},
	};

	// TODO: We should probably have a notification system
	// to pipe messages that aren't critical back to here
	// to display before the program exists. For example
	// Notice: 'pkg' was not found
	// Notice: There are 2 additional records.
	// This can simplify some parts of the code like list/search

	// For all other errors use the color defined in the config.
	if let Err(err) = main_nala(args, derived, &mut config) {
		// Guard clause in cause it is not AptErrors
		// In this case just print it nicely
		let Some(apt_errors) = err.downcast_ref::<AptErrors>() else {
			config.stderr(Theme::Error, &format!("{err:?}"));
			return ExitCode::FAILURE;
		};

		for error in apt_errors.iter() {
			let (theme, msg) = if error.is_error {
				(Theme::Error, error.msg.replace("E: ", ""))
			} else {
				(Theme::Warning, error.msg.replace("W: ", ""))
			};
			config.stderr(theme, &msg);
		}
		return ExitCode::FAILURE;
	}
	ExitCode::SUCCESS
}

fn get_config() -> Result<(ArgMatches, NalaParser, Config)> {
	let args = NalaParser::command().get_matches();
	let derived = NalaParser::from_arg_matches(&args)?;

	let config_file = match derived.config {
		Some(ref conf_file) => conf_file,
		None => Path::new("/etc/nala/nala.conf"),
	};

	let config = match Config::new(config_file) {
		Ok(config) => config,
		Err(err) => {
			eprintln!("Warning: {err}");
			Config::default()
		},
	};

	Ok((args, derived, config))
}

#[tokio::main]
pub async fn system(config: &Config) -> Result<()> {
	// This downloads all of the pkgs into the archives directory
	// let cache = rust_apt::new_cache!()?;
	// println!("Cache Total Pkgs: {}", cache.iter().count());

	// let mut downloader = Downloader::new(config)?;

	// let versions = cache
	// 	.iter()
	// 	.filter_map(|p| {
	// 		let v = p.installed()?;
	// 		if v.is_downloadable() {
	// 			Some(v)
	// 		} else {
	// 			None
	// 		}
	// 	})
	// 	.collect::<Vec<_>>();

	// for ver in &versions {
	// 	downloader.add_version(ver, config).await?;
	// }

	// downloader.run(config, false).await?;

	let archive = config.get_path(&Paths::Archive);
	let mut debs = vec![];
	for entry in std::fs::read_dir(archive)? {
		let entry = entry?;
		let metadata = entry.metadata()?;

		let path = entry.path();

		// If it's a directory, recurse into it
		if metadata.is_dir() {
			continue;
		}

		debs.push(path.to_string_lossy().to_string())
	}

	let cache = new_cache!(&debs)?;
	let filtered_pkgs = cache
		.packages(&PackageSort::default().installed())
		.filter_map(|pkg| {
			let version = pkg.installed()?;
			let file = version
				.version_files()
				.filter_map(|vf| {
					let pf = vf.package_file();
					// Instead of archive we could match the filename
					// pf.filename().unwrap().contains(Paths::Archive.default_path())
					if pf.archive()? == "local-deb" {
						Some(pf.filename()?.to_string())
					} else {
						None
					}
				})
				.next()?;
			Some((version, file))
		})
		.collect::<Vec<_>>();

	let mut pb = tui::NalaProgressBar::new(config, true)?;
	let mut set = tokio::task::JoinSet::new();
	for (_, file) in filtered_pkgs {
		set.spawn(DebFile::new(file));
	}

	let files = pb.join(set).await?;
	for file in files {
		file.store().await?;
	}
	Ok(())
}

fn main_nala(args: ArgMatches, derived: NalaParser, config: &mut Config) -> Result<()> {
	if derived.license {
		println!("Not Yet Implemented.");
		return Ok(());
	}

	if let (Some((name, cmd)), Some(command)) = (args.subcommand(), derived.command) {
		config.command = name.to_string();
		config.load_args(cmd);
		match command {
			Commands::List(_) => list(config)?,
			Commands::Search(_) => search(config)?,
			Commands::Show(_) => show(config)?,
			Commands::Clean(_) => clean(config)?,
			Commands::Download(_) => download(config)?,
			Commands::History(_) => history(config)?,
			Commands::Fetch(_) => fetch(config)?,
			Commands::Update(_) => update(config)?,
			Commands::Upgrade(_) => upgrade(config)?,
			Commands::Install(_) => install(config)?,
			Commands::System(_) => system(config)?,
		}
	} else {
		NalaParser::command().print_help()?;
		bail!("Subcommand not found")
	}
	Ok(())
}
