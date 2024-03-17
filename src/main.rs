use std::process::ExitCode;

use anyhow::{bail, Result};
use clap::{CommandFactory, FromArgMatches};
use downloader::download;
use history::history_test;

mod clean;
mod cli;
mod colors;
mod config;
mod downloader;
mod history;
mod list;
mod show;
mod util;
use crate::clean::clean;
use crate::cli::NalaParser;
use crate::colors::Color;
use crate::list::{list, search};
use crate::show::show;

fn main() -> ExitCode {
	// Setup default color to print pretty even if the config fails
	let color = Color::default();
	if let Err(err) = main_nala() {
		color.error(&format!("{err:?}"));
		return ExitCode::FAILURE;
	}
	ExitCode::SUCCESS
}

fn main_nala() -> Result<()> {
	let args = NalaParser::command().get_matches();
	let derived = NalaParser::from_arg_matches(&args)?;

	if derived.license {
		println!("Not Yet Implemented.");
		return Ok(());
	}

	if let Some((name, _cmd)) = args.subcommand() {
		match name {
			"list" => list()?,
			"search" => search()?,
			"show" => show()?,
			"clean" => clean()?,
			"download" => download()?,
			"history" => history_test()?,
			// Match other subcommands here...
			_ => bail!("Unknown error in the argument parser"),
		}
	} else {
		NalaParser::command().print_help()?;
		bail!("Subcommand not found")
	}
	Ok(())
}
