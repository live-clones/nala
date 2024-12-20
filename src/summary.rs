use std::collections::HashMap;

use anyhow::Result;
use chrono::Utc;
use rust_apt::util::DiskSpace;
use rust_apt::Cache;

use crate::cmd::{
	self, apt_hook_with_pkgs, ask, run_scripts, HistoryEntry, HistoryPackage, Operation,
};
use crate::config::{color, Config, Paths};
use crate::download::Downloader;
use crate::libnala::NalaCache;
use crate::{dpkg, table, tui};

/// TODO: Implement a simple summary that is very short for serial/console users
pub async fn display_summary(
	cache: &Cache,
	config: &Config,
	pkg_set: &HashMap<Operation, Vec<HistoryPackage>>,
) -> Result<bool> {
	if config.get_no_bool("tui", true) {
		// App returns true if we should continue.
		tui::summary::SummaryTab::new(cache, config, pkg_set)
			.run()
			.await
	} else {
		let mut tables = vec![];

		for (op, pkgs) in pkg_set {
			let mut table = table::get_table(if pkgs[0].items(config).len() > 3 {
				&["Package:", "Old Version:", "New Version:", "Size:"]
			} else {
				&["Package:", "Version:", "Size:"]
			});

			table.add_rows(pkgs.iter().map(|p| p.items(config)));
			tables.push((op, table));
		}

		let width = rust_apt::util::terminal_width();
		let sep = "=".repeat(width);

		for (op, pkgs) in tables {
			println!("{sep}");
			println!(" {}", color::highlight!(op.as_str()));
			println!("{sep}");

			println!("{pkgs}");
		}
		println!("{sep}");
		println!(" Summary");
		println!("{sep}");

		for (op, pkgs) in pkg_set {
			println!(" {op} {}", pkgs.len())
		}

		println!();
		if cache.depcache().download_size() > 0 {
			println!(
				" Total download size: {}",
				config.unit_str(cache.depcache().download_size())
			)
		}

		match cache.depcache().disk_size() {
			DiskSpace::Require(disk_space) => {
				println!(" Disk space required: {}", config.unit_str(disk_space))
			},
			DiskSpace::Free(disk_space) => {
				println!(" Disk space to free: {}", config.unit_str(disk_space))
			},
		}
		println!();

		// Returns an error if yes is no selected
		ask("Do you want to continue?")?;
		Ok(true)
	}
}

pub async fn commit(cache: Cache, config: &Config) -> Result<()> {
	let pkg_set = cache.sort_changes()?;
	if pkg_set.is_empty() {
		println!("Nothing to do.");
		return Ok(());
	}

	let changed = cache.get_changes(true).collect::<Vec<_>>();
	let versions = changed
		.iter()
		.filter_map(|pkg| pkg.install_version())
		.collect::<Vec<_>>();

	let mut downloader = Downloader::new(config)?;
	let archive = config.get_path(&Paths::Archive);
	for ver in &versions {
		downloader.add_version(ver, &archive).await?;
	}

	if config.get_bool("print_uris", false) {
		for uri in downloader.uris() {
			println!("{}", uri.to_json()?);
		}
		// Print uris does not go past here
		return Ok(());
	};

	if !crate::summary::display_summary(&cache, config, &pkg_set).await? {
		return Ok(());
	};

	// Only download if needed
	// Downloader will error if empty download
	// TODO: Should probably just make run check and return Ok(vec![])?
	if !downloader.uris().is_empty() {
		let _finished = downloader.run(config, false).await?;
	}

	if config.get_bool("download_only", false) {
		return Ok(());
	}

	let history_entry = HistoryEntry::new(
		cmd::get_history(config)
			.await?
			.iter()
			.map(|entry| entry.id)
			.max()
			.unwrap_or_default()
			+ 1,
		Utc::now().to_rfc3339(),
		pkg_set.into_values().flatten().collect(),
	);

	history_entry.write_to_file(config)?;

	// TODO: There should likely be a field in the history
	// to mark that it was a transaction that failed.
	// The idea is to run the rest of this program,
	// catch any errors, and then write the history file
	// Either way but we'll know that it failed.

	run_scripts(config, "DPkg::Pre-Invoke")?;
	apt_hook_with_pkgs(config, &changed, "DPkg::Pre-Install-Pkgs")?;

	config.apt.set("Dpkg::Use-Pty", "0");

	dpkg::run_install(cache, config)?;

	run_scripts(config, "DPkg::Post-Invoke")
}
