use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[clap(name = "nala-rs")]
#[clap(author = "Blake Lee <blake@volian.org>")]
#[clap(version = "0.1.0")]
#[clap(about = "Commandline front-end for libapt-pkg", long_about = None)]
pub struct NalaParser {
	/// Print license information
	#[clap(global = true, short, long, action)]
	pub license: bool,

	/// Disable scrolling text and print extra information
	#[clap(global = true, short, long, action)]
	pub verbose: bool,

	/// Print debug statements for solving issues
	#[clap(global = true, short, long, action)]
	pub debug: bool,

	/// Specify a different configuration file
	#[clap(short, long, value_parser, value_name = "FILE")]
	pub config: Option<PathBuf>,

	/// Turn on tui if it's disabled in the config.
	#[clap(global = true, short, long, action)]
	pub tui: bool,

	/// Turn the tui off. Takes precedence over other options
	#[clap(global = true, short, long, action)]
	pub no_tui: bool,

	#[clap(subcommand)]
	pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
	List(List),
	Search(Search),
	Show(Show),
	Clean(Clean),
	Download(Download),
	History(History),
	Fetch(Fetch),
	Update(Update),
	Upgrade(Upgrade),
	Install(Install),
}

#[derive(Args, Debug)]
#[allow(clippy::struct_excessive_bools)]
pub struct List {
	/// Package names to search
	#[clap(required = false)]
	pub pkg_names: Vec<String>,

	/// Print the full description of each package
	#[clap(long, action)]
	pub description: bool,

	/// Print the summary of each package
	#[clap(long, action)]
	pub summary: bool,

	/// Show all versions of a package
	#[clap(short, long, action)]
	pub all_versions: bool,

	/// Only include packages that are installed
	#[clap(short, long, action)]
	pub installed: bool,

	/// Only include packages explicitly installed with Nala
	#[clap(short = 'N', long, action)]
	pub nala_installed: bool,

	/// Only include packages that are upgradable
	#[clap(short, long, action)]
	pub upgradable: bool,

	/// Only include virtual packages
	#[clap(short = 'V', long, action)]
	pub r#virtual: bool,
}

#[derive(Args, Debug)]
pub struct Search {
	/// Search is basically list but with an added restriction
	/// by allowing only searching pkg names
	#[clap(long, action)]
	pub names: bool,

	// Flatten list commands args into search
	#[clap(flatten)]
	pub list_args: List,
}

#[derive(Args, Debug)]
pub struct Show {
	/// Package names to show
	#[clap(required = false)]
	pub pkg_names: Vec<String>,

	#[clap(short = 'a', long, action)]
	pub all_versions: bool,
}

/// Removes the local archive of downloaded package files.
#[derive(Args, Debug)]
pub struct Clean {
	/// Removes the package lists downloaded from `update`
	#[clap(long, action)]
	pub lists: bool,

	/// Removes the `nala-sources.list` file generated by the fetch command
	#[clap(long, action)]
	pub fetch: bool,
}

/// Removes the local archive of downloaded package files.
#[derive(Args, Debug)]
pub struct Download {
	/// Package names to download
	pub pkg_names: Vec<String>,

	/// Removes the `nala-sources.list` file generated by the fetch command
	#[clap(long, action)]
	pub fetch: bool,
}

#[derive(Args, Debug)]
pub struct History {
	/// Package names to download
	pub history_id: Option<u32>,
}

#[derive(Args, Debug)]
pub struct Fetch {
	#[clap(long, action)]
	pub non_free: bool,

	#[clap(long, action)]
	pub https_only: bool,

	#[clap(long, action)]
	pub sources: bool,

	#[clap(long, num_args = 0..=1, default_missing_value="3")]
	pub auto: Option<u8>,

	#[clap(short = 'c', long, action)]
	pub country: Vec<String>,

	#[clap(long, action)]
	pub debian: Option<String>,

	#[clap(long, action)]
	pub ubuntu: Option<String>,

	#[clap(long, action)]
	pub devuan: Option<String>,
}

/// Update the package lists.
#[derive(Args, Debug)]
pub struct Update {
	#[clap(short = 'o', long, action)]
	pub dpkg_option: Vec<String>,
}

/// Upgrade packages.
#[derive(Args, Debug)]
pub struct Upgrade {
	/// TODO: Copy from Python Nala and maybe reword.
	#[clap(short = 'o', long, action)]
	pub dpkg_option: Vec<String>,

	/// Prints the URIs in json and does not perform an upgrade.
	#[clap(long, action)]
	pub print_uris: bool,

	/// Perform a Full Upgrade.
	#[clap(long, action)]
	pub full: bool,

	/// Do NOT perform a Full Upgrade.
	#[clap(long, action)]
	pub no_full: bool,

	/// Perform a Safe Upgrade.
	/// Takes precedence over other Upgrade options.
	#[clap(long, action)]
	pub safe: bool,
}

#[derive(Args, Debug)]
pub struct Install {
	#[clap(short = 'o', long, action)]
	pub dpkg_option: Vec<String>,
}
